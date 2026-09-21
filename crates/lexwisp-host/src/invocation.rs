use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use lexwisp_core::{
    ChatCheckpoint, ChatInvocationRequest, ChatModelPreference, ChatRunError, ChatRunFuture,
    ChatRunPort, ConversationId, CredentialStore, ExecutionObserver, ExecutionSnapshot,
    ExecutionStatus, InvocationId, ProviderError, TaskOwner,
};
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    AiService, ExecutionStore, HostTaskPort, ProviderRegistry, TaskScope, execution::ExecutionStart,
};

struct ActiveRun {
    conversation_id: ConversationId,
    cancellation: CancellationToken,
}

pub struct RunSupervisor {
    ai: Arc<AiService>,
    providers: ProviderRegistry,
    credentials: Arc<dyn CredentialStore>,
    executions: Arc<ExecutionStore>,
    active: Mutex<HashMap<InvocationId, ActiveRun>>,
    concurrency: Arc<Semaphore>,
    closing: CancellationToken,
}

impl RunSupervisor {
    pub fn new(
        ai: Arc<AiService>,
        providers: ProviderRegistry,
        credentials: Arc<dyn CredentialStore>,
        executions: Arc<ExecutionStore>,
    ) -> Self {
        Self {
            ai,
            providers,
            credentials,
            executions,
            active: Mutex::new(HashMap::new()),
            concurrency: Arc::new(Semaphore::new(4)),
            closing: CancellationToken::new(),
        }
    }

    async fn execute(
        self: Arc<Self>,
        invocation_id: InvocationId,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
        scope: TaskScope,
    ) -> Result<ExecutionSnapshot, ChatRunError> {
        if self.closing.is_cancelled() {
            return Err(ChatRunError::ShuttingDown);
        }
        let cancellation = CancellationToken::new();
        {
            let mut active = self
                .active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if active
                .values()
                .any(|run| run.conversation_id == request.conversation_id)
            {
                return Err(ChatRunError::ConversationBusy);
            }
            active.insert(
                invocation_id.clone(),
                ActiveRun {
                    conversation_id: request.conversation_id.clone(),
                    cancellation: cancellation.clone(),
                },
            );
        }
        let result = self
            .execute_active(&invocation_id, request, observer, cancellation, scope)
            .await;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&invocation_id);
        result
    }

    async fn execute_active(
        &self,
        invocation_id: &InvocationId,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
        cancellation: CancellationToken,
        scope: TaskScope,
    ) -> Result<ExecutionSnapshot, ChatRunError> {
        let (provider, model_id) = self
            .providers
            .resolve(&request.model_preference)
            .map_err(|error| ChatRunError::Failed(error.to_string()))?;
        let credential = if let Some(reference) = provider.credential_ref() {
            let credentials = self.credentials.clone();
            let reference = reference.to_owned();
            Some(
                scope
                    .spawn_blocking(move || credentials.read(&reference))
                    .await
                    .map_err(|error| ChatRunError::Failed(error.to_string()))?
                    .map_err(|error| ChatRunError::Failed(error.to_string()))?,
            )
        } else {
            None
        };
        self.executions.start(ExecutionStart {
            invocation_id: invocation_id.clone(),
            conversation_id: request.conversation_id,
            user_message_id: request.user_message_id,
            assistant_message_id: request.assistant_message_id,
            provider_id: provider.id().clone(),
            model_id: model_id.clone(),
            input: request.input,
            chat: ChatCheckpoint {
                conversation_title: request.conversation_title,
                model_preference: request.model_preference,
                user_ordinal: request.user_ordinal,
                assistant_ordinal: request.assistant_ordinal,
                attempt_id: request.attempt_id,
                reply_to_user_id: request.reply_to_user_id,
            },
            observer,
        });

        let permit = tokio::select! {
            _ = self.closing.cancelled() => return self.finish_cancelled(invocation_id, "host is shutting down").await,
            _ = cancellation.cancelled() => return self.finish_cancelled(invocation_id, "request was cancelled").await,
            permit = self.concurrency.clone().acquire_owned() => permit.map_err(|_| ChatRunError::ShuttingDown)?,
        };
        if cancellation.is_cancelled() {
            drop(permit);
            return self
                .finish_cancelled(invocation_id, "request was cancelled")
                .await;
        }
        self.executions
            .mark_running(invocation_id)
            .map_err(ChatRunError::Failed)?;
        let executions = self.executions.clone();
        let delta_invocation = invocation_id.clone();
        let ai_result = self
            .ai
            .chat(
                &provider,
                &model_id,
                credential.as_deref(),
                &request.messages,
                &cancellation,
                move |delta| {
                    executions
                        .append_text(&delta_invocation, &delta)
                        .map_err(ProviderError::Protocol)
                },
            )
            .await;
        drop(permit);
        match ai_result {
            Ok(_) => self
                .executions
                .commit_terminal(invocation_id, ExecutionStatus::Completed, None)
                .await
                .map_err(ChatRunError::Failed),
            Err(ProviderError::Cancelled) => {
                self.finish_cancelled(invocation_id, "request was cancelled")
                    .await
            }
            Err(error) => self
                .executions
                .commit_terminal(
                    invocation_id,
                    ExecutionStatus::Failed,
                    Some(error.to_string()),
                )
                .await
                .map_err(ChatRunError::Failed),
        }
    }

    async fn finish_cancelled(
        &self,
        invocation_id: &InvocationId,
        reason: &str,
    ) -> Result<ExecutionSnapshot, ChatRunError> {
        self.executions
            .commit_terminal(
                invocation_id,
                ExecutionStatus::Cancelled,
                Some(reason.into()),
            )
            .await
            .map_err(ChatRunError::Failed)
    }

    pub fn cancel(&self, invocation_id: &InvocationId) -> Result<(), ChatRunError> {
        let cancellation = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(invocation_id)
            .map(|run| run.cancellation.clone())
            .ok_or_else(|| ChatRunError::Failed("run is not active".into()))?;
        self.executions.mark_cancelling(invocation_id);
        cancellation.cancel();
        Ok(())
    }

    pub fn shutdown(&self) {
        self.closing.cancel();
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for run in active.values() {
            run.cancellation.cancel();
        }
    }

    pub async fn wait_until_idle(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        while !self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

pub struct ChatRunner {
    supervisor: Arc<RunSupervisor>,
    tasks: HostTaskPort,
}

impl ChatRunner {
    pub fn new(supervisor: Arc<RunSupervisor>, tasks: HostTaskPort) -> Self {
        Self { supervisor, tasks }
    }
}

impl ChatRunPort for ChatRunner {
    fn run(
        &self,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> ChatRunFuture<'_> {
        let invocation_id = InvocationId::new();
        let supervisor = self.supervisor.clone();
        let scope = self
            .tasks
            .scope(TaskOwner::Invocation(invocation_id.to_string()));
        let task_scope = scope.clone();
        let (sender, receiver) = oneshot::channel();
        scope.spawn(async move {
            let result = supervisor
                .execute(invocation_id, request, observer, task_scope)
                .await;
            let _ = sender.send(result);
        });
        Box::pin(async move { receiver.await.unwrap_or(Err(ChatRunError::ShuttingDown)) })
    }

    fn cancel(&self, invocation_id: &InvocationId) -> Result<(), ChatRunError> {
        self.supervisor.cancel(invocation_id)
    }

    fn context_budget(&self, preference: &ChatModelPreference) -> usize {
        self.supervisor
            .providers
            .resolve(preference)
            .map(|(provider, _)| provider.context_budget())
            .unwrap_or(8_192)
    }
}

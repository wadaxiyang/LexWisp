use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use lexwisp_core::{
    Capability, ChatInvocationRequest, ChatRunError, ChatRunFuture, ChatRunPort, CredentialStore,
    ExecutionObserver, ExecutionSnapshot, ExecutionStatus, InvocationId, PluginId, ProviderError,
    TaskOwner,
};
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    AiService, CapabilityAuthority, ExecutionStore, HostTaskPort, ProviderRegistry, TaskScope,
    execution::ExecutionStart,
};

struct ActiveInvocation {
    conversation_id: lexwisp_core::ConversationId,
    cancellation: CancellationToken,
}

pub struct InvocationSupervisor {
    ai: Arc<AiService>,
    providers: ProviderRegistry,
    credentials: Arc<dyn CredentialStore>,
    executions: Arc<ExecutionStore>,
    active: Mutex<HashMap<InvocationId, ActiveInvocation>>,
    concurrency: Arc<Semaphore>,
    closing: CancellationToken,
}

impl InvocationSupervisor {
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
        plugin_id: PluginId,
        plugin_generation: u64,
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
                .any(|entry| entry.conversation_id == request.conversation_id)
            {
                return Err(ChatRunError::ConversationBusy);
            }
            active.insert(
                invocation_id.clone(),
                ActiveInvocation {
                    conversation_id: request.conversation_id.clone(),
                    cancellation: cancellation.clone(),
                },
            );
        }

        let result = self
            .execute_active(
                invocation_id.clone(),
                plugin_id,
                plugin_generation,
                request,
                observer,
                cancellation,
                scope,
            )
            .await;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&invocation_id);
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_active(
        &self,
        invocation_id: InvocationId,
        plugin_id: PluginId,
        plugin_generation: u64,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
        cancellation: CancellationToken,
        scope: TaskScope,
    ) -> Result<ExecutionSnapshot, ChatRunError> {
        let (provider, model_id) = self
            .providers
            .resolve_default()
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
        let input = request
            .messages
            .last()
            .map(|message| message.content.clone())
            .unwrap_or_default();
        self.executions.start(ExecutionStart {
            invocation_id: invocation_id.clone(),
            plugin_id,
            plugin_generation,
            conversation_id: request.conversation_id,
            user_message_id: request.user_message_id,
            assistant_message_id: request.assistant_message_id,
            provider_id: provider.id().clone(),
            model_id: model_id.clone(),
            input,
            conversation_title: request.conversation_title,
            user_ordinal: request.user_ordinal,
            assistant_ordinal: request.assistant_ordinal,
            observer,
        });

        let permit = tokio::select! {
            _ = self.closing.cancelled() => {
                return self.finish_cancelled(&invocation_id, "host is shutting down").await;
            }
            _ = cancellation.cancelled() => {
                return self.finish_cancelled(&invocation_id, "request was cancelled").await;
            }
            permit = self.concurrency.clone().acquire_owned() => {
                permit.map_err(|_| ChatRunError::ShuttingDown)?
            }
        };
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
                        .append_text(&delta_invocation, plugin_generation, &delta)
                        .map_err(ProviderError::Protocol)
                },
            )
            .await;
        drop(permit);

        match ai_result {
            Ok(_) => self
                .executions
                .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
                .await
                .map_err(ChatRunError::Failed),
            Err(ProviderError::Cancelled) => {
                self.finish_cancelled(&invocation_id, "request was cancelled")
                    .await
            }
            Err(error) => self
                .executions
                .commit_terminal(
                    &invocation_id,
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
            .map(|entry| entry.cancellation.clone())
            .ok_or_else(|| ChatRunError::Failed("invocation is not active".into()))?;
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
        for invocation in active.values() {
            invocation.cancellation.cancel();
        }
    }

    pub async fn wait_until_idle(&self, timeout: std::time::Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        while !self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
            && tokio::time::Instant::now() < deadline
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}

pub struct ScopedChatRunPort {
    plugin_id: PluginId,
    actions: crate::ActionRegistry,
    capabilities: CapabilityAuthority,
    supervisor: Arc<InvocationSupervisor>,
    tasks: HostTaskPort,
}

impl ScopedChatRunPort {
    pub(crate) fn new(
        plugin_id: PluginId,
        actions: crate::ActionRegistry,
        capabilities: CapabilityAuthority,
        supervisor: Arc<InvocationSupervisor>,
        tasks: HostTaskPort,
    ) -> Self {
        Self {
            plugin_id,
            actions,
            capabilities,
            supervisor,
            tasks,
        }
    }
}

impl ChatRunPort for ScopedChatRunPort {
    fn run(
        &self,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> ChatRunFuture<'_> {
        let invalid_identity = request.action.plugin_id() != &self.plugin_id;
        let authorized = self
            .capabilities
            .is_granted(&self.plugin_id, Capability::AiInvoke);
        let registered = self.actions.contains(&request.action);
        if invalid_identity || !authorized || !registered {
            return Box::pin(async {
                Err(ChatRunError::Failed(
                    "plugin identity, registration, or AI capability is invalid".into(),
                ))
            });
        }
        let invocation_id = InvocationId::new();
        let plugin_id = self.plugin_id.clone();
        let plugin_generation = self.actions.generation();
        let supervisor = self.supervisor.clone();
        let scope = self
            .tasks
            .scope(TaskOwner::Invocation(invocation_id.as_str().to_owned()));
        let task_scope = scope.clone();
        let (sender, receiver) = oneshot::channel();
        scope.spawn(async move {
            let result = supervisor
                .execute(
                    invocation_id,
                    plugin_id,
                    plugin_generation,
                    request,
                    observer,
                    task_scope,
                )
                .await;
            let _ = sender.send(result);
        });
        Box::pin(async move { receiver.await.unwrap_or(Err(ChatRunError::ShuttingDown)) })
    }

    fn cancel(&self, invocation_id: &InvocationId) -> Result<(), ChatRunError> {
        self.supervisor.cancel(invocation_id)
    }
}

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use lexwisp_core::{
    ActionDescriptor, ActionRequest, AiMessage, AiRole, Capability, ChatCheckpoint,
    ChatInvocationRequest, ChatModelPreference, ChatRunError, ChatRunFuture, ChatRunPort,
    CredentialStore, ExecutionObserver, ExecutionSnapshot, ExecutionStatus, InvocationId, PluginId,
    ProviderError, ProviderId, QualifiedActionId, TaskOwner, TextInvocationRequest, TextRunFuture,
    TextRunPort,
};
use tokio::sync::{Semaphore, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{
    AiService, CapabilityAuthority, ExecutionStore, HostTaskPort, ProviderRegistry, TaskScope,
    execution::ExecutionStart,
};

struct ActiveInvocation {
    plugin_id: PluginId,
    conversation_id: Option<lexwisp_core::ConversationId>,
    cancellation: CancellationToken,
}

struct PreparedInvocation {
    action: QualifiedActionId,
    messages: Vec<AiMessage>,
    input: String,
    conversation_id: Option<lexwisp_core::ConversationId>,
    user_message_id: Option<lexwisp_core::MessageId>,
    assistant_message_id: Option<lexwisp_core::MessageId>,
    chat: Option<ChatCheckpoint>,
    model_preference: ChatModelPreference,
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
        request: PreparedInvocation,
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
            if request
                .conversation_id
                .as_ref()
                .is_some_and(|conversation| {
                    active
                        .values()
                        .any(|entry| entry.conversation_id.as_ref() == Some(conversation))
                })
            {
                return Err(ChatRunError::ConversationBusy);
            }
            active.insert(
                invocation_id.clone(),
                ActiveInvocation {
                    plugin_id: plugin_id.clone(),
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
        request: PreparedInvocation,
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
            plugin_id: plugin_id.clone(),
            action: request.action,
            plugin_generation,
            conversation_id: request.conversation_id,
            user_message_id: request.user_message_id,
            assistant_message_id: request.assistant_message_id,
            provider_id: provider.id().clone(),
            model_id: model_id.clone(),
            input: request.input,
            chat: request.chat,
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
        if cancellation.is_cancelled() {
            drop(permit);
            return self
                .finish_cancelled(&invocation_id, "request was cancelled")
                .await;
        }
        self.executions
            .mark_running(&invocation_id)
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

    pub fn cancel_plugin(&self, plugin_id: &PluginId) -> usize {
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut cancelled = 0;
        for (invocation_id, invocation) in active.iter() {
            if &invocation.plugin_id == plugin_id {
                self.executions.mark_cancelling(invocation_id);
                invocation.cancellation.cancel();
                cancelled += 1;
            }
        }
        cancelled
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

    pub(crate) fn begin_script(
        &self,
        invocation_id: InvocationId,
        descriptor: &ActionDescriptor,
        generation: u64,
        request: &ActionRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> Result<CancellationToken, String> {
        if self.closing.is_cancelled() {
            return Err("host is shutting down".into());
        }
        let cancellation = CancellationToken::new();
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                invocation_id.clone(),
                ActiveInvocation {
                    plugin_id: descriptor.plugin_id().clone(),
                    conversation_id: None,
                    cancellation: cancellation.clone(),
                },
            );
        self.executions.start(ExecutionStart {
            invocation_id: invocation_id.clone(),
            plugin_id: descriptor.plugin_id().clone(),
            action: descriptor.qualified_id(),
            plugin_generation: generation,
            conversation_id: None,
            user_message_id: None,
            assistant_message_id: None,
            provider_id: ProviderId::parse("script").map_err(|error| error.to_string())?,
            model_id: "quickjs".into(),
            input: request.input.clone(),
            chat: None,
            observer,
        });
        self.executions.mark_running(&invocation_id)?;
        Ok(cancellation)
    }

    pub(crate) fn script_output_len(&self, invocation_id: &InvocationId) -> usize {
        self.executions
            .snapshot(invocation_id)
            .map(|snapshot| snapshot.output.len())
            .unwrap_or_default()
    }

    pub(crate) fn append_script(
        &self,
        invocation_id: &InvocationId,
        generation: u64,
        text: &str,
    ) -> Result<(), String> {
        self.executions.append_text(invocation_id, generation, text)
    }

    pub(crate) async fn finish_script(
        &self,
        invocation_id: &InvocationId,
        generation: u64,
        result: Result<String, String>,
    ) -> Result<String, String> {
        let cancelled = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(invocation_id)
            .is_some_and(|active| active.cancellation.is_cancelled());
        let (status, error) = match &result {
            Ok(text) if !cancelled => {
                match self.executions.append_text(invocation_id, generation, text) {
                    Ok(()) => (ExecutionStatus::Completed, None),
                    Err(error) => (ExecutionStatus::Failed, Some(error)),
                }
            }
            _ if cancelled => (
                ExecutionStatus::Cancelled,
                Some("request was cancelled".into()),
            ),
            Err(error) => (ExecutionStatus::Failed, Some(error.clone())),
            Ok(_) => unreachable!(),
        };
        let committed = self
            .executions
            .commit_terminal(invocation_id, status, error)
            .await;
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(invocation_id);
        let snapshot = committed?;
        if status == ExecutionStatus::Completed {
            Ok(snapshot.output)
        } else {
            Err(snapshot
                .error
                .unwrap_or_else(|| "script invocation did not complete".into()))
        }
    }

    pub(crate) async fn script_ai(
        &self,
        system: Option<String>,
        input: String,
        cancellation: &CancellationToken,
    ) -> Result<String, String> {
        let (provider, model_id) = self
            .providers
            .resolve(&ChatModelPreference::Fast)
            .map_err(|error| error.to_string())?;
        let credential = if let Some(reference) = provider.credential_ref() {
            let store = self.credentials.clone();
            let reference = reference.to_owned();
            Some(
                tokio::task::spawn_blocking(move || store.read(&reference))
                    .await
                    .map_err(|error| error.to_string())?
                    .map_err(|error| error.to_string())?,
            )
        } else {
            None
        };
        let mut messages = Vec::new();
        if let Some(system) = system {
            messages.push(AiMessage {
                role: AiRole::System,
                content: system,
            });
        }
        messages.push(AiMessage {
            role: AiRole::User,
            content: input,
        });
        let output = Arc::new(Mutex::new(String::new()));
        let collector = output.clone();
        self.ai
            .chat(
                &provider,
                &model_id,
                credential.as_deref(),
                &messages,
                cancellation,
                move |delta| {
                    collector
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push_str(&delta);
                    Ok(())
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone())
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
        let prepared = PreparedInvocation {
            action: request.action,
            messages: request.messages.clone(),
            input: request.input,
            conversation_id: Some(request.conversation_id),
            user_message_id: Some(request.user_message_id),
            assistant_message_id: Some(request.assistant_message_id),
            chat: Some(ChatCheckpoint {
                conversation_title: request.conversation_title,
                model_preference: request.model_preference.clone(),
                user_ordinal: request.user_ordinal,
                assistant_ordinal: request.assistant_ordinal,
                attempt_id: request.attempt_id,
                reply_to_user_id: request.reply_to_user_id,
            }),
            model_preference: request.model_preference,
        };
        scope.spawn(async move {
            let result = supervisor
                .execute(
                    invocation_id,
                    plugin_id,
                    plugin_generation,
                    prepared,
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

    fn context_budget(&self, preference: &ChatModelPreference) -> usize {
        self.supervisor
            .providers
            .resolve(preference)
            .map(|(provider, _)| provider.context_budget())
            .unwrap_or(8_192)
    }
}

pub struct ScopedTextRunPort {
    plugin_id: PluginId,
    actions: crate::ActionRegistry,
    capabilities: CapabilityAuthority,
    supervisor: Arc<InvocationSupervisor>,
    tasks: HostTaskPort,
    binding: Option<(String, u64)>,
}

impl ScopedTextRunPort {
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
            binding: None,
        }
    }

    pub(crate) fn new_bound(
        plugin_id: PluginId,
        actions: crate::ActionRegistry,
        capabilities: CapabilityAuthority,
        supervisor: Arc<InvocationSupervisor>,
        tasks: HostTaskPort,
        package_hash: String,
        generation: u64,
    ) -> Self {
        Self {
            plugin_id,
            actions,
            capabilities,
            supervisor,
            tasks,
            binding: Some((package_hash, generation)),
        }
    }
}

impl TextRunPort for ScopedTextRunPort {
    fn run(
        &self,
        request: TextInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> TextRunFuture<'_> {
        let invalid_identity = request.action.plugin_id() != &self.plugin_id;
        let authorized = self.binding.as_ref().map_or_else(
            || {
                self.capabilities
                    .is_granted(&self.plugin_id, Capability::AiInvoke)
            },
            |(hash, generation)| {
                self.capabilities.is_bound_grant_valid(
                    &self.plugin_id,
                    Capability::AiInvoke,
                    hash,
                    *generation,
                )
            },
        );
        let registered = self.actions.contains(&request.action);
        if invalid_identity || !authorized || !registered {
            return Box::pin(async {
                Err(ChatRunError::Failed(
                    "plugin identity, registration, or AI capability is invalid".into(),
                ))
            });
        }
        let prompt = match render_prompt(&request.definition, &request.parameters) {
            Ok(prompt) => prompt,
            Err(error) => return Box::pin(async move { Err(ChatRunError::Failed(error)) }),
        };
        let invocation_id = InvocationId::new();
        let plugin_id = self.plugin_id.clone();
        let plugin_generation = self
            .binding
            .as_ref()
            .map(|(_, generation)| *generation)
            .unwrap_or_else(|| self.actions.generation());
        let supervisor = self.supervisor.clone();
        let scope = self
            .tasks
            .scope(TaskOwner::Invocation(invocation_id.as_str().to_owned()));
        let task_scope = scope.clone();
        let prepared = PreparedInvocation {
            action: request.action,
            messages: vec![
                AiMessage {
                    role: AiRole::System,
                    content: prompt,
                },
                AiMessage {
                    role: AiRole::User,
                    content: request.input.clone(),
                },
            ],
            input: request.input,
            conversation_id: None,
            user_message_id: None,
            assistant_message_id: None,
            chat: None,
            model_preference: ChatModelPreference::Fast,
        };
        let (sender, receiver) = oneshot::channel();
        scope.spawn(async move {
            let result = supervisor
                .execute(
                    invocation_id,
                    plugin_id,
                    plugin_generation,
                    prepared,
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

fn render_prompt(
    definition: &lexwisp_core::DeclarativeActionDefinition,
    provided: &std::collections::BTreeMap<String, String>,
) -> Result<String, String> {
    let mut values = std::collections::BTreeMap::new();
    for parameter in &definition.parameters {
        let value = provided
            .get(&parameter.key)
            .cloned()
            .or_else(|| parameter.default_value.clone());
        let Some(value) = value else {
            if parameter.required {
                return Err(format!(
                    "required parameter '{}' is missing",
                    parameter.label
                ));
            }
            continue;
        };
        if value.len() > 256 {
            return Err(format!("parameter '{}' is too long", parameter.label));
        }
        match parameter.kind {
            lexwisp_core::ParameterKind::Enum
                if !parameter.choices.iter().any(|choice| choice == &value) =>
            {
                return Err(format!(
                    "parameter '{}' has an invalid value",
                    parameter.label
                ));
            }
            lexwisp_core::ParameterKind::Boolean if !matches!(value.as_str(), "true" | "false") => {
                return Err(format!(
                    "parameter '{}' must be true or false",
                    parameter.label
                ));
            }
            lexwisp_core::ParameterKind::Number if value.parse::<f64>().is_err() => {
                return Err(format!("parameter '{}' must be a number", parameter.label));
            }
            _ => {}
        }
        values.insert(parameter.key.clone(), value);
    }
    if let Some(unknown) = provided.keys().find(|key| !values.contains_key(*key)) {
        return Err(format!("unknown parameter '{unknown}'"));
    }

    let mut prompt = definition.prompt.clone();
    while let Some(start) = prompt.find("{{") {
        let end = prompt[start + 2..]
            .find("}}")
            .map(|offset| start + 2 + offset)
            .ok_or_else(|| "prompt contains an unterminated template variable".to_string())?;
        let token = prompt[start + 2..end].trim();
        let key = token
            .strip_prefix("params.")
            .ok_or_else(|| format!("unknown template variable '{{{{{token}}}}}'"))?;
        let value = values
            .get(key)
            .ok_or_else(|| format!("template parameter '{key}' is missing"))?;
        prompt.replace_range(start..end + 2, value);
    }
    Ok(prompt)
}

#[cfg(test)]
mod declarative_tests {
    use std::collections::BTreeMap;

    use lexwisp_core::{
        ActionInputSource, ActionOutputPolicy, ActionParameter, DeclarativeActionDefinition,
        DismissPolicy, ParameterKind,
    };

    use super::render_prompt;

    fn definition() -> DeclarativeActionDefinition {
        DeclarativeActionDefinition {
            prompt: "Apply {{params.mode}}. Input stays user content.".into(),
            parameters: vec![ActionParameter {
                key: "mode".into(),
                label: "Mode".into(),
                kind: ParameterKind::Text,
                required: true,
                default_value: Some("standard".into()),
                choices: Vec::new(),
            }],
            allowed_sources: vec![ActionInputSource::Manual],
            dismiss_policy: DismissPolicy::Cancel,
            output: ActionOutputPolicy {
                allow_copy: true,
                allow_favorite: true,
                allow_replace: false,
            },
        }
    }

    #[test]
    fn template_only_replaces_declared_parameters() {
        assert_eq!(
            render_prompt(&definition(), &BTreeMap::new()).expect("default renders"),
            "Apply standard. Input stays user content."
        );
        let mut unknown = BTreeMap::new();
        unknown.insert("other".into(), "value".into());
        assert!(render_prompt(&definition(), &unknown).is_err());
    }
}

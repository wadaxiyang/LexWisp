use std::sync::{Arc, Mutex, Weak};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{
    ActionDescriptor, ActionError, ActionFuture, ActionHandler, ActionInputSource, ActionKind,
    ActionRequest, ActionResult, ExecutionObserver, ExecutionSnapshot, ExecutionStatus,
    InputSource, InvocationId, ParameterKind, ScriptInvocationHost, StorageState, TextActionError,
    TextActionSnapshot, TextActionUiPort,
};

use crate::runtime::RuntimeHandle;

struct ControllerState {
    input: String,
    output: String,
    status: ExecutionStatus,
    invocation_id: Option<InvocationId>,
    active_invocation: Option<InvocationId>,
    last_sequence: u64,
    generation: u64,
    status_text: String,
    storage: StorageState,
    starting: bool,
    visible: bool,
    subscribers: Vec<Sender<TextActionSnapshot>>,
}

pub(crate) struct ScriptController {
    self_weak: Weak<Self>,
    descriptor: ActionDescriptor,
    host: Arc<dyn ScriptInvocationHost>,
    settings: Arc<dyn lexwisp_core::SettingsUiPort>,
    runtime: RuntimeHandle,
    state: Mutex<ControllerState>,
}

impl ScriptController {
    pub(crate) fn new(
        descriptor: ActionDescriptor,
        host: Arc<dyn ScriptInvocationHost>,
        settings: Arc<dyn lexwisp_core::SettingsUiPort>,
        runtime: RuntimeHandle,
    ) -> Result<Arc<Self>, String> {
        if !matches!(descriptor.kind(), ActionKind::Script(_)) {
            return Err("script controller requires a script action".into());
        }
        Ok(Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            descriptor,
            host,
            settings,
            runtime,
            state: Mutex::new(ControllerState {
                input: String::new(),
                output: String::new(),
                status: ExecutionStatus::Completed,
                invocation_id: None,
                active_invocation: None,
                last_sequence: 0,
                generation: 0,
                status_text: "Ready".into(),
                storage: StorageState::Saved,
                starting: false,
                visible: false,
                subscribers: Vec::new(),
            }),
        }))
    }

    fn definition(&self) -> &lexwisp_core::ScriptActionDefinition {
        match self.descriptor.kind() {
            ActionKind::Script(definition) => definition,
            _ => unreachable!("constructor validates script action kind"),
        }
    }

    fn snapshot_from(&self, state: &ControllerState) -> TextActionSnapshot {
        TextActionSnapshot {
            action: self.descriptor.qualified_id(),
            input: state.input.clone(),
            output: state.output.clone(),
            status: state.status,
            invocation_id: state.invocation_id.clone(),
            active_invocation: state.active_invocation.clone(),
            generation: state.generation,
            status_text: state.status_text.clone(),
            storage: state.storage,
        }
    }

    fn publish(&self, state: &mut ControllerState) {
        state.generation = state.generation.saturating_add(1);
        if !state.visible {
            return;
        }
        let snapshot = self.snapshot_from(state);
        state.subscribers.retain(|subscriber| {
            matches!(
                subscriber.try_send(snapshot.clone()),
                Ok(()) | Err(TrySendError::Full(_))
            )
        });
    }

    async fn execute_request(
        &self,
        mut request: ActionRequest,
    ) -> Result<ActionResult, ActionError> {
        let input = request.input.trim().to_owned();
        if input.is_empty() {
            return Err(ActionError::Failed("input is empty".into()));
        }
        let source = match request.source {
            InputSource::Selection | InputSource::Candidate => ActionInputSource::Selection,
            InputSource::Manual => ActionInputSource::Manual,
            InputSource::Clipboard => ActionInputSource::Clipboard,
        };
        if !self.definition().allowed_sources.contains(&source) {
            return Err(ActionError::Failed(
                "this action does not allow the selected input source".into(),
            ));
        }
        validate_parameters(self.definition(), &mut request.parameters)?;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.starting || state.active_invocation.is_some() {
                return Err(ActionError::Failed(TextActionError::Busy.to_string()));
            }
            state.input.clone_from(&input);
            state.output.clear();
            state.status = ExecutionStatus::Queued;
            state.status_text = "Starting…".into();
            state.storage = StorageState::Pending;
            state.starting = true;
            state.last_sequence = 0;
            self.publish(&mut state);
        }
        let observer: Arc<dyn ExecutionObserver> = Arc::new(ScriptObserver {
            controller: self.self_weak.clone(),
        });
        let invocation = match self
            .host
            .begin(&self.descriptor, request.clone(), observer)
            .await
        {
            Ok(invocation) => invocation,
            Err(error) => {
                self.fail_start(&error);
                return Err(ActionError::Failed(error));
            }
        };
        let params_json = parameters_json(self.definition(), &request.parameters)?;
        let input_json = serde_json::json!({
            "text": input,
            "source": match request.source {
                InputSource::Selection => "selection",
                InputSource::Candidate => "candidate",
                InputSource::Manual => "manual",
                InputSource::Clipboard => "clipboard",
            }
        })
        .to_string();
        let result = self
            .runtime
            .invoke(
                self.descriptor.plugin_id().clone(),
                self.definition().handler.clone(),
                input_json,
                params_json,
                invocation.clone(),
            )
            .await;
        let final_result = invocation.finish(result).await;
        match final_result {
            Ok(output) => Ok(ActionResult { output }),
            Err(_error) if invocation.is_cancelled() => Err(ActionError::Cancelled),
            Err(error) => Err(ActionError::Failed(error)),
        }
    }

    fn fail_start(&self, error: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.starting = false;
        state.status = ExecutionStatus::Failed;
        state.status_text = error.to_owned();
        self.publish(&mut state);
    }
}

impl ActionHandler for ScriptController {
    fn execute<'a>(&'a self, request: ActionRequest) -> ActionFuture<'a> {
        Box::pin(self.execute_request(request))
    }
}

impl TextActionUiPort for ScriptController {
    fn action(&self) -> lexwisp_core::QualifiedActionId {
        self.descriptor.qualified_id()
    }

    fn snapshot(&self) -> TextActionSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.snapshot_from(&state)
    }

    fn subscribe(&self, capacity: usize) -> Receiver<TextActionSnapshot> {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = sender.try_send(self.snapshot_from(&state));
        state.subscribers.push(sender);
        receiver
    }

    fn set_surface_visible(&self, visible: bool) {
        let invocation = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.visible = visible;
            if visible {
                let snapshot = self.snapshot_from(&state);
                state.subscribers.retain(|subscriber| {
                    matches!(
                        subscriber.try_send(snapshot.clone()),
                        Ok(()) | Err(TrySendError::Full(_))
                    )
                });
            }
            (!visible && self.resolved_dismiss_policy() == lexwisp_core::DismissPolicy::Cancel)
                .then(|| state.active_invocation.clone())
                .flatten()
        };
        if let Some(invocation) = invocation {
            let _ = self.host.cancel(&invocation);
        }
    }

    fn stop(&self) -> Result<(), TextActionError> {
        let invocation = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active_invocation
            .clone()
            .ok_or(TextActionError::NotRunning)?;
        self.host
            .cancel(&invocation)
            .map_err(TextActionError::Failed)
    }
}

impl ScriptController {
    fn resolved_dismiss_policy(&self) -> lexwisp_core::DismissPolicy {
        self.settings
            .snapshot()
            .settings()
            .dismiss_override(&self.descriptor.qualified_id().to_string())
            .unwrap_or(self.definition().dismiss_policy)
    }
}

struct ScriptObserver {
    controller: Weak<ScriptController>,
}

impl ExecutionObserver for ScriptObserver {
    fn on_execution(&self, snapshot: ExecutionSnapshot) {
        let Some(controller) = self.controller.upgrade() else {
            return;
        };
        let mut state = controller
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if snapshot.action != controller.descriptor.qualified_id()
            || snapshot.sequence <= state.last_sequence
        {
            return;
        }
        state.last_sequence = snapshot.sequence;
        state.starting = false;
        state.invocation_id = Some(snapshot.invocation_id.clone());
        state.active_invocation =
            (!snapshot.status.is_terminal()).then(|| snapshot.invocation_id.clone());
        state.output = snapshot.output;
        state.status = snapshot.status;
        state.storage = snapshot.storage;
        state.status_text = match snapshot.status {
            ExecutionStatus::Queued => "Queued".into(),
            ExecutionStatus::Running => "Running script…".into(),
            ExecutionStatus::Cancelling => "Stopping…".into(),
            ExecutionStatus::Completed if snapshot.storage == StorageState::Pending => {
                "Complete · saving…".into()
            }
            ExecutionStatus::Completed => "Complete".into(),
            ExecutionStatus::Cancelled => "Stopped · partial result kept".into(),
            ExecutionStatus::Failed | ExecutionStatus::Interrupted => snapshot
                .error
                .unwrap_or_else(|| "Failed · partial result kept".into()),
        };
        controller.publish(&mut state);
    }
}

fn validate_parameters(
    definition: &lexwisp_core::ScriptActionDefinition,
    provided: &mut std::collections::BTreeMap<String, String>,
) -> Result<(), ActionError> {
    for parameter in &definition.parameters {
        if !provided.contains_key(&parameter.key)
            && let Some(default) = &parameter.default_value
        {
            provided.insert(parameter.key.clone(), default.clone());
        }
        let Some(value) = provided.get(&parameter.key) else {
            if parameter.required {
                return Err(ActionError::Failed(format!(
                    "required parameter '{}' is missing",
                    parameter.label
                )));
            }
            continue;
        };
        if value.len() > 256 {
            return Err(ActionError::Failed(format!(
                "parameter '{}' is too long",
                parameter.label
            )));
        }
        let valid = match parameter.kind {
            ParameterKind::Text => true,
            ParameterKind::Enum => parameter.choices.contains(value),
            ParameterKind::Boolean => matches!(value.as_str(), "true" | "false"),
            ParameterKind::Number => value.parse::<f64>().is_ok(),
        };
        if !valid {
            return Err(ActionError::Failed(format!(
                "parameter '{}' has an invalid value",
                parameter.label
            )));
        }
    }
    if let Some(unknown) = provided.keys().find(|key| {
        !definition
            .parameters
            .iter()
            .any(|parameter| &parameter.key == *key)
    }) {
        return Err(ActionError::Failed(format!(
            "unknown parameter '{unknown}'"
        )));
    }
    Ok(())
}

fn parameters_json(
    definition: &lexwisp_core::ScriptActionDefinition,
    provided: &std::collections::BTreeMap<String, String>,
) -> Result<String, ActionError> {
    let mut values = serde_json::Map::new();
    for parameter in &definition.parameters {
        let Some(value) = provided.get(&parameter.key) else {
            continue;
        };
        let value = match parameter.kind {
            ParameterKind::Text | ParameterKind::Enum => serde_json::Value::String(value.clone()),
            ParameterKind::Boolean => serde_json::Value::Bool(value == "true"),
            ParameterKind::Number => serde_json::Number::from_f64(
                value
                    .parse::<f64>()
                    .map_err(|error| ActionError::Failed(error.to_string()))?,
            )
            .map(serde_json::Value::Number)
            .ok_or_else(|| ActionError::Failed("number parameter is not finite".into()))?,
        };
        values.insert(parameter.key.clone(), value);
    }
    Ok(serde_json::Value::Object(values).to_string())
}

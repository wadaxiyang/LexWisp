use std::sync::{Arc, Mutex, Weak};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{
    ActionDescriptor, ActionError, ActionFuture, ActionHandler, ActionId, ActionInputSource,
    ActionKind, ActionOutputPolicy, ActionParameter, ActionRequest, ActionResult, Capability,
    DeclarativeActionDefinition, DismissPolicy, ExecutionObserver, ExecutionSnapshot,
    ExecutionStatus, InputSource, InvocationId, ParameterKind, PluginDescriptor, PluginId,
    QualifiedActionId, SettingsUiPort, StorageState, TextActionError, TextActionSnapshot,
    TextActionUiPort, TextInvocationRequest, TextRunPort,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    plugin: ManifestPlugin,
    capabilities: ManifestCapabilities,
    actions: Vec<ManifestAction>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestPlugin {
    id: String,
    name: String,
    version: String,
    kind: String,
    host_api: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestCapabilities {
    required: Vec<String>,
    #[serde(default)]
    optional: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestAction {
    id: String,
    name: String,
    input_kind: String,
    allowed_sources: Vec<String>,
    prompt: String,
    model_profile: String,
    dismiss_policy: String,
    #[serde(default)]
    parameters: Vec<ManifestParameter>,
    output: ManifestOutput,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestParameter {
    key: String,
    label: String,
    kind: String,
    required: bool,
    default: Option<String>,
    #[serde(default)]
    choices: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestOutput {
    format: String,
    allow_copy: bool,
    allow_favorite: bool,
    allow_replace: bool,
}

pub struct DeclarativePackage {
    pub plugin: PluginDescriptor,
    pub actions: Vec<ActionDescriptor>,
}

impl DeclarativePackage {
    pub fn parse(manifest: &str, prompt_name: &str, prompt: &str) -> Result<Self, String> {
        let manifest: Manifest = toml::from_str(manifest).map_err(|error| error.to_string())?;
        if manifest.schema_version != 1
            || manifest.plugin.kind != "declarative"
            || manifest.plugin.host_api != "^1.0"
            || manifest.plugin.version.trim().is_empty()
        {
            return Err(
                "unsupported declarative package schema, kind, version, or Host API".into(),
            );
        }
        if !manifest.capabilities.optional.is_empty()
            || manifest.capabilities.required != ["ai.invoke"]
        {
            return Err("built-in declarative actions may request only ai.invoke".into());
        }
        let plugin_id = PluginId::parse(manifest.plugin.id).map_err(|error| error.to_string())?;
        let plugin = PluginDescriptor::new(
            plugin_id.clone(),
            manifest.plugin.name,
            vec![Capability::AiInvoke],
        );
        let mut actions = Vec::new();
        for action in manifest.actions {
            if action.input_kind != "text"
                || action.prompt != prompt_name
                || action.model_profile != "Fast"
                || action.output.format != "text"
            {
                return Err(
                    "unsupported declarative action input, prompt, profile, or output".into(),
                );
            }
            let allowed_sources = action
                .allowed_sources
                .into_iter()
                .map(|source| match source.as_str() {
                    "selection" => Ok(ActionInputSource::Selection),
                    "manual" => Ok(ActionInputSource::Manual),
                    "clipboard" => Ok(ActionInputSource::Clipboard),
                    _ => Err(format!("unknown action source '{source}'")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let dismiss_policy = match action.dismiss_policy.as_str() {
                "cancel" => DismissPolicy::Cancel,
                "continue" => DismissPolicy::Continue,
                value => return Err(format!("unknown dismiss policy '{value}'")),
            };
            let parameters = action
                .parameters
                .into_iter()
                .map(|parameter| {
                    let kind = match parameter.kind.as_str() {
                        "text" => ParameterKind::Text,
                        "enum" => ParameterKind::Enum,
                        "boolean" => ParameterKind::Boolean,
                        "number" => ParameterKind::Number,
                        value => return Err(format!("unknown parameter kind '{value}'")),
                    };
                    Ok(ActionParameter {
                        key: parameter.key,
                        label: parameter.label,
                        kind,
                        required: parameter.required,
                        default_value: parameter.default,
                        choices: parameter.choices,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            actions.push(ActionDescriptor::declarative(
                plugin_id.clone(),
                ActionId::parse(action.id).map_err(|error| error.to_string())?,
                action.name,
                DeclarativeActionDefinition {
                    prompt: prompt.to_owned(),
                    parameters,
                    allowed_sources,
                    dismiss_policy,
                    output: ActionOutputPolicy {
                        allow_copy: action.output.allow_copy,
                        allow_favorite: action.output.allow_favorite,
                        allow_replace: action.output.allow_replace,
                    },
                },
            ));
        }
        if actions.is_empty() {
            return Err("declarative package has no actions".into());
        }
        Ok(Self { plugin, actions })
    }
}

struct DeclarativeState {
    input: String,
    output: String,
    status: ExecutionStatus,
    active_invocation: Option<InvocationId>,
    invocation_id: Option<InvocationId>,
    last_sequence: u64,
    generation: u64,
    status_text: String,
    storage: StorageState,
    starting: bool,
    visible: bool,
    subscribers: Vec<Sender<TextActionSnapshot>>,
}

pub struct DeclarativeController {
    self_weak: Weak<Self>,
    descriptor: ActionDescriptor,
    runner: Arc<dyn TextRunPort>,
    settings: Arc<dyn SettingsUiPort>,
    state: Mutex<DeclarativeState>,
}

impl DeclarativeController {
    pub fn new(
        descriptor: ActionDescriptor,
        runner: Arc<dyn TextRunPort>,
        settings: Arc<dyn SettingsUiPort>,
    ) -> Result<Arc<Self>, String> {
        if !matches!(descriptor.kind(), ActionKind::Declarative(_)) {
            return Err("declarative controller requires a declarative action".into());
        }
        Ok(Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            descriptor,
            runner,
            settings,
            state: Mutex::new(DeclarativeState {
                input: String::new(),
                output: String::new(),
                status: ExecutionStatus::Completed,
                active_invocation: None,
                invocation_id: None,
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

    pub fn descriptor(&self) -> &ActionDescriptor {
        &self.descriptor
    }

    fn definition(&self) -> &DeclarativeActionDefinition {
        match self.descriptor.kind() {
            ActionKind::Declarative(definition) => definition,
            ActionKind::Native => unreachable!("constructor validates action kind"),
        }
    }

    fn make_snapshot(&self, state: &DeclarativeState) -> TextActionSnapshot {
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

    fn publish(&self, state: &mut DeclarativeState) {
        state.generation = state.generation.saturating_add(1);
        if !state.visible {
            return;
        }
        let snapshot = self.make_snapshot(state);
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
        if let Some(defaults) = self
            .settings
            .snapshot()
            .settings()
            .action_parameter_defaults(&self.descriptor.qualified_id().to_string())
        {
            for (key, value) in defaults {
                request
                    .parameters
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
        }
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.starting || state.active_invocation.is_some() {
                return Err(ActionError::Failed(TextActionError::Busy.to_string()));
            }
            state.input = input.clone();
            state.output.clear();
            state.status = ExecutionStatus::Queued;
            state.status_text = "Starting…".into();
            state.storage = StorageState::Pending;
            state.last_sequence = 0;
            state.starting = true;
            self.publish(&mut state);
        }
        let observer: Arc<dyn ExecutionObserver> = Arc::new(DeclarativeObserver {
            controller: self.self_weak.clone(),
        });
        let invocation = TextInvocationRequest {
            action: self.descriptor.qualified_id(),
            definition: self.definition().clone(),
            input,
            parameters: request.parameters,
        };
        match self.runner.run(invocation, observer).await {
            Ok(snapshot) if snapshot.status == ExecutionStatus::Completed => Ok(ActionResult {
                output: snapshot.output,
            }),
            Ok(snapshot) if snapshot.status == ExecutionStatus::Cancelled => {
                Err(ActionError::Cancelled)
            }
            Ok(snapshot) => Err(ActionError::Failed(
                snapshot.error.unwrap_or_else(|| "request failed".into()),
            )),
            Err(error) => {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.starting = false;
                state.status = ExecutionStatus::Failed;
                state.status_text = error.to_string();
                self.publish(&mut state);
                Err(ActionError::Failed(error.to_string()))
            }
        }
    }

    fn resolved_dismiss_policy(&self) -> DismissPolicy {
        self.settings
            .snapshot()
            .settings()
            .dismiss_override(&self.descriptor.qualified_id().to_string())
            .unwrap_or(self.definition().dismiss_policy)
    }
}

impl ActionHandler for DeclarativeController {
    fn execute<'a>(&'a self, request: ActionRequest) -> ActionFuture<'a> {
        Box::pin(self.execute_request(request))
    }
}

impl TextActionUiPort for DeclarativeController {
    fn action(&self) -> QualifiedActionId {
        self.descriptor.qualified_id()
    }

    fn snapshot(&self) -> TextActionSnapshot {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.make_snapshot(&state)
    }

    fn subscribe(&self, capacity: usize) -> Receiver<TextActionSnapshot> {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = sender.try_send(self.make_snapshot(&state));
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
                let snapshot = self.make_snapshot(&state);
                state.subscribers.retain(|subscriber| {
                    matches!(
                        subscriber.try_send(snapshot.clone()),
                        Ok(()) | Err(TrySendError::Full(_))
                    )
                });
            }
            (!visible && self.resolved_dismiss_policy() == DismissPolicy::Cancel)
                .then(|| state.active_invocation.clone())
                .flatten()
        };
        if let Some(invocation) = invocation {
            let _ = self.runner.cancel(&invocation);
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
        self.runner
            .cancel(&invocation)
            .map_err(|error| TextActionError::Failed(error.to_string()))
    }
}

struct DeclarativeObserver {
    controller: Weak<DeclarativeController>,
}

impl ExecutionObserver for DeclarativeObserver {
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
            ExecutionStatus::Running => "Generating…".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"
schema_version = 1
[plugin]
id = "org.lexwisp.fixture"
name = "Fixture"
version = "0.1.0"
kind = "declarative"
host_api = "^1.0"
[capabilities]
required = ["ai.invoke"]
optional = []
[[actions]]
id = "run"
name = "Run"
input_kind = "text"
allowed_sources = ["manual"]
prompt = "prompt.md"
model_profile = "Fast"
dismiss_policy = "cancel"
[actions.output]
format = "text"
allow_copy = true
allow_favorite = true
allow_replace = false
"#;

    #[test]
    fn manifest_is_the_source_of_action_behavior() {
        let package =
            DeclarativePackage::parse(MANIFEST, "prompt.md", "Do it.").expect("manifest parses");
        assert_eq!(package.plugin.display_name(), "Fixture");
        assert_eq!(package.actions.len(), 1);
        assert!(matches!(
            package.actions[0].kind(),
            ActionKind::Declarative(_)
        ));
    }
}

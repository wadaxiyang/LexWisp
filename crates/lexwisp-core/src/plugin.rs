use std::{
    collections::BTreeMap, fmt, future::Future, path::PathBuf, pin::Pin, str::FromStr, sync::Arc,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct PluginId(String);

impl PluginId {
    pub fn parse(value: impl Into<String>) -> Result<Self, PluginIdError> {
        let value = value.into();
        let valid = value.len() <= 128
            && value.split('.').count() >= 3
            && value.split('.').all(|segment| {
                !segment.is_empty()
                    && segment.len() <= 40
                    && segment.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
                    && !segment.starts_with('-')
                    && !segment.ends_with('-')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(PluginIdError(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl FromStr for PluginId {
    type Err = PluginIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("'{0}' is not a valid reverse-DNS plugin ID")]
pub struct PluginIdError(String);

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ActionId(String);

impl ActionId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ActionIdError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(ActionIdError(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct QualifiedActionId {
    plugin_id: PluginId,
    action_id: ActionId,
}

impl FromStr for QualifiedActionId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (plugin, action) = value
            .split_once('/')
            .ok_or_else(|| format!("'{value}' is not a qualified action ID"))?;
        Ok(Self::new(
            PluginId::parse(plugin).map_err(|error| error.to_string())?,
            ActionId::parse(action).map_err(|error| error.to_string())?,
        ))
    }
}

impl QualifiedActionId {
    pub const fn new(plugin_id: PluginId, action_id: ActionId) -> Self {
        Self {
            plugin_id,
            action_id,
        }
    }

    pub const fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub const fn action_id(&self) -> &ActionId {
        &self.action_id
    }
}

impl fmt::Display for QualifiedActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.plugin_id, self.action_id)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("'{0}' is not a valid action ID")]
pub struct ActionIdError(String);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    AiInvoke,
    NetworkRequest,
    SelectionRead,
    ClipboardRead,
    ClipboardWrite,
    WindowRead,
    StorageRead,
    StorageWrite,
}

impl Capability {
    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::AiInvoke => "ai.invoke",
            Self::NetworkRequest => "network.request",
            Self::SelectionRead => "selection.read",
            Self::ClipboardRead => "clipboard.read",
            Self::ClipboardWrite => "clipboard.write",
            Self::WindowRead => "window.read",
            Self::StorageRead => "storage.read",
            Self::StorageWrite => "storage.write",
        }
    }

    pub fn parse_manifest(value: &str) -> Result<Self, String> {
        match value {
            "ai.invoke" => Ok(Self::AiInvoke),
            "network.request" => Ok(Self::NetworkRequest),
            "selection.read" => Ok(Self::SelectionRead),
            "clipboard.read" => Ok(Self::ClipboardRead),
            "clipboard.write" => Ok(Self::ClipboardWrite),
            "window.read" => Ok(Self::WindowRead),
            "storage.read" => Ok(Self::StorageRead),
            "storage.write" => Ok(Self::StorageWrite),
            _ => Err(format!("unknown capability '{value}'")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDescriptor {
    id: PluginId,
    display_name: String,
    requested_capabilities: Vec<Capability>,
}

impl PluginDescriptor {
    pub fn new(
        id: PluginId,
        display_name: impl Into<String>,
        requested_capabilities: Vec<Capability>,
    ) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            requested_capabilities,
        }
    }

    pub const fn id(&self) -> &PluginId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn requested_capabilities(&self) -> &[Capability] {
        &self.requested_capabilities
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionDescriptor {
    plugin_id: PluginId,
    id: ActionId,
    display_name: String,
    kind: ActionKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionKind {
    Native,
    Declarative(DeclarativeActionDefinition),
    Script(ScriptActionDefinition),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptActionDefinition {
    pub handler: String,
    pub parameters: Vec<ActionParameter>,
    pub allowed_sources: Vec<ActionInputSource>,
    pub dismiss_policy: crate::DismissPolicy,
    pub output: ActionOutputPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterKind {
    Text,
    Enum,
    Boolean,
    Number,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionParameter {
    pub key: String,
    pub label: String,
    pub kind: ParameterKind,
    pub required: bool,
    pub default_value: Option<String>,
    pub choices: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionOutputPolicy {
    pub allow_copy: bool,
    pub allow_favorite: bool,
    pub allow_replace: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeclarativeActionDefinition {
    pub prompt: String,
    pub parameters: Vec<ActionParameter>,
    pub allowed_sources: Vec<ActionInputSource>,
    pub dismiss_policy: crate::DismissPolicy,
    pub output: ActionOutputPolicy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionInputSource {
    Selection,
    Manual,
    Clipboard,
}

impl ActionDescriptor {
    pub fn new(plugin_id: PluginId, id: ActionId, display_name: impl Into<String>) -> Self {
        Self {
            plugin_id,
            id,
            display_name: display_name.into(),
            kind: ActionKind::Native,
        }
    }

    pub fn declarative(
        plugin_id: PluginId,
        id: ActionId,
        display_name: impl Into<String>,
        definition: DeclarativeActionDefinition,
    ) -> Self {
        Self {
            plugin_id,
            id,
            display_name: display_name.into(),
            kind: ActionKind::Declarative(definition),
        }
    }

    pub fn script(
        plugin_id: PluginId,
        id: ActionId,
        display_name: impl Into<String>,
        definition: ScriptActionDefinition,
    ) -> Self {
        Self {
            plugin_id,
            id,
            display_name: display_name.into(),
            kind: ActionKind::Script(definition),
        }
    }

    pub fn text_parameters(&self) -> Option<&[ActionParameter]> {
        match &self.kind {
            ActionKind::Declarative(definition) => Some(&definition.parameters),
            ActionKind::Script(definition) => Some(&definition.parameters),
            ActionKind::Native => None,
        }
    }

    pub fn text_sources(&self) -> Option<&[ActionInputSource]> {
        match &self.kind {
            ActionKind::Declarative(definition) => Some(&definition.allowed_sources),
            ActionKind::Script(definition) => Some(&definition.allowed_sources),
            ActionKind::Native => None,
        }
    }

    pub fn text_output(&self) -> Option<&ActionOutputPolicy> {
        match &self.kind {
            ActionKind::Declarative(definition) => Some(&definition.output),
            ActionKind::Script(definition) => Some(&definition.output),
            ActionKind::Native => None,
        }
    }

    pub fn text_dismiss_policy(&self) -> Option<crate::DismissPolicy> {
        match &self.kind {
            ActionKind::Declarative(definition) => Some(definition.dismiss_policy),
            ActionKind::Script(definition) => Some(definition.dismiss_policy),
            ActionKind::Native => None,
        }
    }

    pub const fn plugin_id(&self) -> &PluginId {
        &self.plugin_id
    }

    pub const fn id(&self) -> &ActionId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub const fn kind(&self) -> &ActionKind {
        &self.kind
    }

    pub fn qualified_id(&self) -> QualifiedActionId {
        QualifiedActionId::new(self.plugin_id.clone(), self.id.clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    pub input: String,
    pub source: crate::InputSource,
    pub parameters: BTreeMap<String, String>,
    pub context_token: Option<crate::ContextToken>,
}

impl ActionRequest {
    pub fn manual(input: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            source: crate::InputSource::Manual,
            parameters: BTreeMap::new(),
            context_token: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionResult {
    pub output: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ActionError {
    #[error("action was cancelled")]
    Cancelled,
    #[error("action failed: {0}")]
    Failed(String),
}

pub type ActionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ActionResult, ActionError>> + Send + 'a>>;

pub trait ActionHandler: Send + Sync {
    fn execute<'a>(&'a self, request: ActionRequest) -> ActionFuture<'a>;
}

pub trait ActionUiPort: Send + Sync {
    fn invoke<'a>(&'a self, action: QualifiedActionId, request: ActionRequest) -> ActionFuture<'a>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManagedPluginStatus {
    Enabled,
    Disabled,
    Faulted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginKind {
    Declarative,
    Script,
}

impl PluginKind {
    pub const fn manifest_name(self) -> &'static str {
        match self {
            Self::Declarative => "declarative",
            Self::Script => "script",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedPluginSummary {
    pub kind: PluginKind,
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub source_path: PathBuf,
    pub install_path: PathBuf,
    pub package_hash: String,
    pub generation: u64,
    pub status: ManagedPluginStatus,
    pub granted_capabilities: Vec<Capability>,
    pub actions: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PluginImportPreview {
    pub token: String,
    pub id: PluginId,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    pub source_path: PathBuf,
    pub package_hash: String,
    pub requested_capabilities: Vec<Capability>,
    pub added_capabilities: Vec<Capability>,
    pub network_scopes: Vec<String>,
    pub actions: Vec<String>,
    pub replaces_version: Option<String>,
}

#[derive(Clone)]
pub struct ManagedActionSnapshot {
    pub generation: u64,
    pub descriptors: Vec<ActionDescriptor>,
    pub controllers: Vec<Arc<dyn crate::TextActionUiPort>>,
}

pub type PluginManagementFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;

pub trait PluginManagementUiPort: Send + Sync {
    fn list(&self) -> Vec<ManagedPluginSummary>;
    fn action_snapshot(&self) -> ManagedActionSnapshot;
    fn preview<'a>(&'a self, source: PathBuf) -> PluginManagementFuture<'a, PluginImportPreview>;
    fn confirm<'a>(&'a self, token: String) -> PluginManagementFuture<'a, ()>;
    fn discard_preview<'a>(&'a self, token: String) -> PluginManagementFuture<'a, ()>;
    fn set_enabled<'a>(
        &'a self,
        plugin_id: PluginId,
        enabled: bool,
    ) -> PluginManagementFuture<'a, ()>;
    fn preview_reload<'a>(
        &'a self,
        plugin_id: PluginId,
    ) -> PluginManagementFuture<'a, PluginImportPreview>;
    fn uninstall<'a>(&'a self, plugin_id: PluginId) -> PluginManagementFuture<'a, ()>;
    fn open_directory(&self, plugin_id: &PluginId) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_ids_require_reverse_dns_shape() {
        assert!(PluginId::parse("org.lexwisp.fixture").is_ok());
        assert!(PluginId::parse("Fixture").is_err());
        assert!(PluginId::parse("org..fixture").is_err());
    }

    #[test]
    fn action_ids_are_stable_ascii_tokens() {
        assert!(ActionId::parse("open-settings").is_ok());
        assert!(ActionId::parse("Open Settings").is_err());
    }
}

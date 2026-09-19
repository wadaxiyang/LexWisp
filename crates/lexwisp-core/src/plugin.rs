use std::{fmt, future::Future, pin::Pin, str::FromStr};

use thiserror::Error;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
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

#[derive(Clone, Debug, Error, Eq, PartialEq)]
#[error("'{0}' is not a valid action ID")]
pub struct ActionIdError(String);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Capability {
    AiInvoke,
    StorageRead,
    StorageWrite,
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
}

impl ActionDescriptor {
    pub fn new(plugin_id: PluginId, id: ActionId, display_name: impl Into<String>) -> Self {
        Self {
            plugin_id,
            id,
            display_name: display_name.into(),
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRequest {
    pub input: String,
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

pub trait ActionHandler: Send + Sync {
    fn execute<'a>(
        &'a self,
        request: ActionRequest,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult, ActionError>> + Send + 'a>>;
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

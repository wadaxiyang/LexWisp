use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn parse(value: impl Into<String>) -> Result<Self, ProviderError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 64
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
            });
        valid
            .then_some(Self(value.clone()))
            .ok_or(ProviderError::InvalidId(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    id: ProviderId,
    display_name: String,
    base_url: String,
    credential_ref: Option<String>,
    model_ids: Vec<String>,
    stream: bool,
    #[serde(default = "default_context_budget")]
    context_budget: usize,
    #[serde(default)]
    proxy_url: Option<String>,
    #[serde(default = "default_connect_timeout_seconds")]
    connect_timeout_seconds: u64,
    #[serde(default = "default_total_timeout_seconds")]
    total_timeout_seconds: u64,
    #[serde(default = "default_event_timeout_seconds")]
    event_timeout_seconds: u64,
    #[serde(default)]
    temperature_milli: Option<u16>,
    #[serde(default)]
    max_output_tokens: Option<u32>,
}

impl ProviderConfig {
    pub fn new(
        id: ProviderId,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
        credential_ref: Option<String>,
        model_ids: Vec<String>,
        stream: bool,
    ) -> Self {
        Self {
            id,
            display_name: display_name.into(),
            base_url: base_url.into(),
            credential_ref,
            model_ids,
            stream,
            context_budget: default_context_budget(),
            proxy_url: None,
            connect_timeout_seconds: default_connect_timeout_seconds(),
            total_timeout_seconds: default_total_timeout_seconds(),
            event_timeout_seconds: default_event_timeout_seconds(),
            temperature_milli: None,
            max_output_tokens: None,
        }
    }

    pub const fn id(&self) -> &ProviderId {
        &self.id
    }

    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn credential_ref(&self) -> Option<&str> {
        self.credential_ref.as_deref()
    }

    pub fn model_ids(&self) -> &[String] {
        &self.model_ids
    }

    pub const fn stream(&self) -> bool {
        self.stream
    }

    pub const fn context_budget(&self) -> usize {
        self.context_budget
    }

    pub fn with_context_budget(mut self, context_budget: usize) -> Self {
        self.context_budget = context_budget;
        self
    }

    pub fn proxy_url(&self) -> Option<&str> {
        self.proxy_url.as_deref()
    }

    pub const fn connect_timeout_seconds(&self) -> u64 {
        self.connect_timeout_seconds
    }

    pub const fn total_timeout_seconds(&self) -> u64 {
        self.total_timeout_seconds
    }

    pub const fn event_timeout_seconds(&self) -> u64 {
        self.event_timeout_seconds
    }

    pub const fn temperature_milli(&self) -> Option<u16> {
        self.temperature_milli
    }

    pub const fn max_output_tokens(&self) -> Option<u32> {
        self.max_output_tokens
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_advanced_options(
        mut self,
        context_budget: usize,
        proxy_url: Option<String>,
        connect_timeout_seconds: u64,
        total_timeout_seconds: u64,
        event_timeout_seconds: u64,
        temperature_milli: Option<u16>,
        max_output_tokens: Option<u32>,
    ) -> Self {
        self.context_budget = context_budget;
        self.proxy_url = proxy_url;
        self.connect_timeout_seconds = connect_timeout_seconds;
        self.total_timeout_seconds = total_timeout_seconds;
        self.event_timeout_seconds = event_timeout_seconds;
        self.temperature_milli = temperature_milli;
        self.max_output_tokens = max_output_tokens;
        self
    }
}

const fn default_context_budget() -> usize {
    8_192
}

const fn default_connect_timeout_seconds() -> u64 {
    10
}
const fn default_total_timeout_seconds() -> u64 {
    180
}
const fn default_event_timeout_seconds() -> u64 {
    60
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelProfile {
    name: String,
    provider_id: ProviderId,
    model_id: String,
}

impl ModelProfile {
    pub fn new(
        name: impl Into<String>,
        provider_id: ProviderId,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            provider_id,
            model_id: model_id.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDraft {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub model_id: String,
    pub stream: bool,
    pub use_authentication: bool,
    pub api_key: Option<String>,
    pub context_budget: String,
    pub proxy_url: String,
    pub connect_timeout_seconds: String,
    pub total_timeout_seconds: String,
    pub event_timeout_seconds: String,
    pub temperature: String,
    pub max_output_tokens: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderUiSnapshot {
    pub configured: bool,
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub model_id: String,
    pub stream: bool,
    pub use_authentication: bool,
    pub has_saved_credential: bool,
    pub generation: u64,
    pub context_budget: usize,
    pub proxy_url: String,
    pub connect_timeout_seconds: u64,
    pub total_timeout_seconds: u64,
    pub event_timeout_seconds: u64,
    pub temperature_milli: Option<u16>,
    pub max_output_tokens: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderTestResult {
    pub model_id: String,
    pub response_preview: String,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ProviderError {
    #[error("'{0}' is not a valid provider ID")]
    InvalidId(String),
    #[error("provider configuration is invalid: {0}")]
    InvalidConfiguration(String),
    #[error("no default provider is configured")]
    NotConfigured,
    #[error("provider credentials are unavailable: {0}")]
    Credential(String),
    #[error("provider request was rejected: {0}")]
    Http(String),
    #[error("provider response was invalid: {0}")]
    Protocol(String),
    #[error("provider request timed out while waiting for {0}")]
    Timeout(&'static str),
    #[error("provider request was cancelled")]
    Cancelled,
    #[error("provider settings could not be saved: {0}")]
    Save(String),
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CredentialError {
    #[error("credential operation failed: {0}")]
    Platform(String),
    #[error("credential '{0}' was not found")]
    NotFound(String),
    #[error("credential value is not valid UTF-8")]
    InvalidEncoding,
}

pub trait CredentialStore: Send + Sync {
    fn read(&self, reference: &str) -> Result<String, CredentialError>;
    fn write(&self, reference: &str, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self, reference: &str) -> Result<(), CredentialError>;
}

pub type ProviderUiFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ProviderError>> + Send + 'a>>;

pub trait ProviderUiPort: Send + Sync {
    fn snapshot(&self) -> ProviderUiSnapshot;
    fn test(&self, draft: ProviderDraft) -> ProviderUiFuture<'_, ProviderTestResult>;
    fn save(&self, draft: ProviderDraft) -> ProviderUiFuture<'_, ProviderUiSnapshot>;
}

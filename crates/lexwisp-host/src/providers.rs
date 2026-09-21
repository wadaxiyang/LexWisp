use std::sync::{Arc, RwLock};

use lexwisp_core::{
    AiMessage, AiRole, AppSettings, ChatModelPreference, CredentialStore, ModelProfile,
    ProviderConfig, ProviderDraft, ProviderError, ProviderId, ProviderTestResult, ProviderUiFuture,
    ProviderUiPort, ProviderUiSnapshot, SettingsUiPort,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{AiService, SettingsService, TaskScope};

#[derive(Clone)]
pub struct ProviderRegistry {
    state: Arc<RwLock<ProviderRegistryState>>,
}

struct ProviderRegistryState {
    settings_generation: u64,
    providers: Vec<ProviderConfig>,
    default_profile: Option<ModelProfile>,
}

impl ProviderRegistry {
    pub fn new(settings: &AppSettings) -> Self {
        Self {
            state: Arc::new(RwLock::new(ProviderRegistryState {
                settings_generation: 0,
                providers: settings.providers().to_vec(),
                default_profile: settings.default_profile().cloned(),
            })),
        }
    }

    pub fn replace(&self, settings: &AppSettings, generation: u64) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.settings_generation = generation;
        state.providers = settings.providers().to_vec();
        state.default_profile = settings.default_profile().cloned();
    }

    pub fn resolve_default(&self) -> Result<(ProviderConfig, String), ProviderError> {
        let state = self
            .state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let profile = state
            .default_profile
            .as_ref()
            .ok_or(ProviderError::NotConfigured)?;
        let provider = state
            .providers
            .iter()
            .find(|provider| provider.id() == profile.provider_id())
            .cloned()
            .ok_or(ProviderError::NotConfigured)?;
        Ok((provider, profile.model_id().to_owned()))
    }

    pub fn resolve(
        &self,
        preference: &ChatModelPreference,
    ) -> Result<(ProviderConfig, String), ProviderError> {
        let (provider, default_model) = self.resolve_default()?;
        match preference {
            ChatModelPreference::Fast | ChatModelPreference::Smart => Ok((provider, default_model)),
            ChatModelPreference::Model(model)
                if provider
                    .model_ids()
                    .iter()
                    .any(|candidate| candidate == model) =>
            {
                Ok((provider, model.clone()))
            }
            ChatModelPreference::Model(model) => Err(ProviderError::InvalidConfiguration(format!(
                "model '{model}' is not configured for the selected provider"
            ))),
        }
    }

    pub fn generation(&self) -> u64 {
        self.state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .settings_generation
    }
}

pub struct ProviderService {
    settings: Arc<SettingsService>,
    registry: ProviderRegistry,
    credentials: Arc<dyn CredentialStore>,
    ai: Arc<AiService>,
    tasks: TaskScope,
}

impl ProviderService {
    pub fn new(
        settings: Arc<SettingsService>,
        registry: ProviderRegistry,
        credentials: Arc<dyn CredentialStore>,
        ai: Arc<AiService>,
        tasks: TaskScope,
    ) -> Self {
        Self {
            settings,
            registry,
            credentials,
            ai,
            tasks,
        }
    }

    fn ui_snapshot(&self) -> ProviderUiSnapshot {
        let snapshot = self.settings.snapshot();
        let configured = snapshot.settings().default_profile().and_then(|profile| {
            snapshot
                .settings()
                .provider(profile.provider_id())
                .map(|provider| (profile, provider))
        });
        if let Some((profile, provider)) = configured {
            ProviderUiSnapshot {
                configured: true,
                id: provider.id().as_str().to_owned(),
                display_name: provider.display_name().to_owned(),
                base_url: provider.base_url().to_owned(),
                model_id: profile.model_id().to_owned(),
                stream: provider.stream(),
                use_authentication: provider.credential_ref().is_some(),
                has_saved_credential: provider.credential_ref().is_some(),
                generation: snapshot.generation(),
                context_budget: provider.context_budget(),
                proxy_url: provider.proxy_url().unwrap_or_default().to_owned(),
                connect_timeout_seconds: provider.connect_timeout_seconds(),
                total_timeout_seconds: provider.total_timeout_seconds(),
                event_timeout_seconds: provider.event_timeout_seconds(),
                temperature_milli: provider.temperature_milli(),
                max_output_tokens: provider.max_output_tokens(),
            }
        } else {
            ProviderUiSnapshot {
                configured: false,
                id: "default".into(),
                display_name: "OpenAI compatible".into(),
                base_url: "https://api.openai.com/v1".into(),
                model_id: String::new(),
                stream: true,
                use_authentication: true,
                has_saved_credential: false,
                generation: snapshot.generation(),
                context_budget: 8_192,
                proxy_url: String::new(),
                connect_timeout_seconds: 10,
                total_timeout_seconds: 180,
                event_timeout_seconds: 60,
                temperature_milli: None,
                max_output_tokens: None,
            }
        }
    }

    async fn resolve_test_credential(
        &self,
        draft: &ProviderDraft,
    ) -> Result<Option<String>, ProviderError> {
        if !draft.use_authentication {
            return Ok(None);
        }
        if let Some(key) = draft
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
        {
            return Ok(Some(key.to_owned()));
        }
        let snapshot = self.settings.snapshot();
        let existing = snapshot
            .settings()
            .default_profile()
            .and_then(|profile| snapshot.settings().provider(profile.provider_id()).cloned())
            .and_then(|provider| provider.credential_ref().map(str::to_owned));
        let Some(reference) = existing else {
            return Err(ProviderError::Credential(
                "enter an API key or disable authentication".into(),
            ));
        };
        let credentials = self.credentials.clone();
        self.tasks
            .spawn_blocking(move || credentials.read(&reference))
            .await
            .map_err(|error| ProviderError::Credential(error.to_string()))?
            .map(Some)
            .map_err(|error| ProviderError::Credential(error.to_string()))
    }

    async fn test_inner(&self, draft: ProviderDraft) -> Result<ProviderTestResult, ProviderError> {
        let (provider, model) = build_config(&draft, None)?;
        let credential = self.resolve_test_credential(&draft).await?;
        let messages = [AiMessage {
            role: AiRole::User,
            content: "Reply with OK.".into(),
            attachments: Vec::new(),
        }];
        let output = self
            .ai
            .chat(
                &provider,
                &model,
                credential.as_deref(),
                &messages,
                &CancellationToken::new(),
                |_| Ok(()),
            )
            .await?;
        Ok(ProviderTestResult {
            model_id: model,
            response_preview: output.chars().take(160).collect(),
        })
    }

    async fn save_inner(&self, draft: ProviderDraft) -> Result<ProviderUiSnapshot, ProviderError> {
        let current = self.settings.snapshot();
        let old_reference = current
            .settings()
            .default_profile()
            .and_then(|profile| current.settings().provider(profile.provider_id()))
            .and_then(ProviderConfig::credential_ref)
            .map(str::to_owned);
        let supplied_key = draft
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(str::to_owned);
        let new_reference = if draft.use_authentication {
            if supplied_key.is_some() {
                Some(format!("LexWisp/provider/{}/{}", draft.id, Uuid::new_v4()))
            } else {
                Some(old_reference.clone().ok_or_else(|| {
                    ProviderError::Credential("enter an API key before saving".into())
                })?)
            }
        } else {
            None
        };
        let (provider, model) = build_config(&draft, new_reference.clone())?;
        if let (Some(reference), Some(key)) = (new_reference.as_ref(), supplied_key.as_ref()) {
            let credentials = self.credentials.clone();
            let reference = reference.clone();
            let key = key.clone();
            self.tasks
                .spawn_blocking(move || credentials.write(&reference, &key))
                .await
                .map_err(|error| ProviderError::Credential(error.to_string()))?
                .map_err(|error| ProviderError::Credential(error.to_string()))?;
        }

        let profile = ModelProfile::new("Fast", provider.id().clone(), model);
        let updated = current.settings().clone().with_provider(provider, profile);
        let saved = self.settings.apply(updated).await;
        let saved = match saved {
            Ok(saved) => saved,
            Err(error) => {
                if new_reference != old_reference
                    && let Some(reference) = new_reference
                {
                    let credentials = self.credentials.clone();
                    let _ = self
                        .tasks
                        .spawn_blocking(move || credentials.delete(&reference))
                        .await;
                }
                return Err(ProviderError::Save(error.to_string()));
            }
        };
        self.registry.replace(saved.settings(), saved.generation());
        if old_reference != new_reference
            && let Some(reference) = old_reference
        {
            let credentials = self.credentials.clone();
            let _ = self
                .tasks
                .spawn_blocking(move || credentials.delete(&reference))
                .await;
        }
        Ok(self.ui_snapshot())
    }
}

impl ProviderUiPort for ProviderService {
    fn snapshot(&self) -> ProviderUiSnapshot {
        self.ui_snapshot()
    }

    fn test(&self, draft: ProviderDraft) -> ProviderUiFuture<'_, ProviderTestResult> {
        Box::pin(self.test_inner(draft))
    }

    fn save(&self, draft: ProviderDraft) -> ProviderUiFuture<'_, ProviderUiSnapshot> {
        Box::pin(self.save_inner(draft))
    }
}

fn build_config(
    draft: &ProviderDraft,
    credential_ref: Option<String>,
) -> Result<(ProviderConfig, String), ProviderError> {
    let id = ProviderId::parse(draft.id.trim())?;
    let display_name = draft.display_name.trim();
    let base_url = draft.base_url.trim();
    let model = draft.model_id.trim();
    if display_name.is_empty() || base_url.is_empty() || model.is_empty() {
        return Err(ProviderError::InvalidConfiguration(
            "name, Base URL, and model are required".into(),
        ));
    }
    crate::ai::completion_endpoint(base_url)?;
    let context_budget = parse_required::<usize>(&draft.context_budget, "context budget")?;
    let connect_timeout = parse_required::<u64>(&draft.connect_timeout_seconds, "connect timeout")?;
    let total_timeout = parse_required::<u64>(&draft.total_timeout_seconds, "total timeout")?;
    let event_timeout = parse_required::<u64>(&draft.event_timeout_seconds, "event timeout")?;
    let temperature_milli = parse_optional::<f64>(&draft.temperature, "temperature")?
        .map(|temperature| {
            if !(0.0..=2.0).contains(&temperature) {
                return Err(ProviderError::InvalidConfiguration(
                    "temperature must be between 0 and 2".into(),
                ));
            }
            Ok((temperature * 1_000.0).round() as u16)
        })
        .transpose()?;
    let max_output_tokens = parse_optional::<u32>(&draft.max_output_tokens, "max output tokens")?;
    if context_budget == 0
        || !(1..=120).contains(&connect_timeout)
        || !(1..=600).contains(&total_timeout)
        || !(1..=300).contains(&event_timeout)
        || max_output_tokens == Some(0)
    {
        return Err(ProviderError::InvalidConfiguration(
            "context budget, timeouts, or output limit is outside the supported range".into(),
        ));
    }
    let proxy_url = (!draft.proxy_url.trim().is_empty()).then(|| draft.proxy_url.trim().to_owned());
    Ok((
        ProviderConfig::new(
            id,
            display_name,
            base_url,
            credential_ref,
            vec![model.to_owned()],
            draft.stream,
        )
        .with_advanced_options(
            context_budget,
            proxy_url,
            connect_timeout,
            total_timeout,
            event_timeout,
            temperature_milli,
            max_output_tokens,
        ),
        model.to_owned(),
    ))
}

fn parse_required<T: std::str::FromStr>(value: &str, label: &str) -> Result<T, ProviderError> {
    value
        .trim()
        .parse()
        .map_err(|_| ProviderError::InvalidConfiguration(format!("{label} must be a valid number")))
}

fn parse_optional<T: std::str::FromStr>(
    value: &str,
    label: &str,
) -> Result<Option<T>, ProviderError> {
    let value = value.trim();
    if value.is_empty() {
        Ok(None)
    } else {
        parse_required(value, label).map(Some)
    }
}

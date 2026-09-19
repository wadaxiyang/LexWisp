use std::{collections::BTreeMap, future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ModelProfile, ProviderConfig, ProviderId};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GlobalHotkey {
    ControlAltSpace,
    ControlShiftSpace,
    AltShiftSpace,
}

impl GlobalHotkey {
    pub const ALL: [Self; 3] = [
        Self::ControlAltSpace,
        Self::ControlShiftSpace,
        Self::AltShiftSpace,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ControlAltSpace => "Ctrl + Alt + Space",
            Self::ControlShiftSpace => "Ctrl + Shift + Space",
            Self::AltShiftSpace => "Alt + Shift + Space",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchMode {
    #[default]
    ActionPalette,
    TranslateSelection,
    DefaultAction,
}

impl LaunchMode {
    pub const ALL: [Self; 3] = [
        Self::ActionPalette,
        Self::TranslateSelection,
        Self::DefaultAction,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::ActionPalette => "Choose an action",
            Self::TranslateSelection => "Translate selection",
            Self::DefaultAction => "Run default action",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DismissPolicy {
    Cancel,
    Continue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchRoute {
    ActionPalette,
    QuickAsk,
    Automatic(String),
    MissingDefaultAction,
}

impl ThemePreference {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub const fn label(self) -> &'static str {
        match self {
            Self::System => "Follow Windows",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppSettings {
    schema_version: u32,
    hotkey: GlobalHotkey,
    theme: ThemePreference,
    launch_at_startup: bool,
    popup_retention_seconds: u64,
    #[serde(default)]
    launch_mode: LaunchMode,
    #[serde(default = "default_translation_action")]
    translation_action_id: String,
    #[serde(default)]
    default_action_id: Option<String>,
    #[serde(default)]
    dismiss_overrides: BTreeMap<String, DismissPolicy>,
    #[serde(default)]
    providers: Vec<ProviderConfig>,
    #[serde(default)]
    default_profile: Option<ModelProfile>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            hotkey: GlobalHotkey::ControlAltSpace,
            theme: ThemePreference::System,
            launch_at_startup: false,
            popup_retention_seconds: 30,
            launch_mode: LaunchMode::ActionPalette,
            translation_action_id: default_translation_action(),
            default_action_id: None,
            dismiss_overrides: BTreeMap::new(),
            providers: Vec::new(),
            default_profile: None,
        }
    }
}

impl AppSettings {
    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.schema_version != SETTINGS_SCHEMA_VERSION {
            return Err(SettingsError::UnsupportedSchema(self.schema_version));
        }
        if self.popup_retention_seconds > 600 {
            return Err(SettingsError::Invalid(
                "popup retention must be between 0 and 600 seconds".into(),
            ));
        }
        if self.translation_action_id.trim().is_empty() {
            return Err(SettingsError::Invalid(
                "translation action ID cannot be empty".into(),
            ));
        }
        self.translation_action_id
            .parse::<crate::QualifiedActionId>()
            .map_err(SettingsError::Invalid)?;
        if self
            .default_action_id
            .as_ref()
            .is_some_and(|id| id.trim().is_empty())
        {
            return Err(SettingsError::Invalid(
                "default action ID cannot be empty".into(),
            ));
        }
        if let Some(action) = &self.default_action_id {
            action
                .parse::<crate::QualifiedActionId>()
                .map_err(SettingsError::Invalid)?;
        }
        for action in self.dismiss_overrides.keys() {
            action
                .parse::<crate::QualifiedActionId>()
                .map_err(SettingsError::Invalid)?;
        }
        if self
            .providers
            .iter()
            .any(|provider| provider.context_budget() == 0)
        {
            return Err(SettingsError::Invalid(
                "provider context budget must be greater than zero".into(),
            ));
        }
        if let Some(profile) = &self.default_profile {
            let provider = self
                .providers
                .iter()
                .find(|provider| provider.id() == profile.provider_id())
                .ok_or_else(|| {
                    SettingsError::Invalid(format!(
                        "default profile references missing provider '{}'",
                        profile.provider_id().as_str()
                    ))
                })?;
            if !provider
                .model_ids()
                .iter()
                .any(|model| model == profile.model_id())
            {
                return Err(SettingsError::Invalid(format!(
                    "default profile references missing model '{}'",
                    profile.model_id()
                )));
            }
        }
        Ok(())
    }

    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub const fn hotkey(&self) -> GlobalHotkey {
        self.hotkey
    }

    pub const fn theme(&self) -> ThemePreference {
        self.theme
    }

    pub const fn launch_at_startup(&self) -> bool {
        self.launch_at_startup
    }

    pub const fn popup_retention_seconds(&self) -> u64 {
        self.popup_retention_seconds
    }

    pub const fn launch_mode(&self) -> LaunchMode {
        self.launch_mode
    }

    pub fn translation_action_id(&self) -> &str {
        &self.translation_action_id
    }

    pub fn default_action_id(&self) -> Option<&str> {
        self.default_action_id.as_deref()
    }

    pub fn launch_route(&self, has_verified_selection: bool) -> LaunchRoute {
        if !has_verified_selection {
            return LaunchRoute::QuickAsk;
        }
        match self.launch_mode {
            LaunchMode::ActionPalette => LaunchRoute::ActionPalette,
            LaunchMode::TranslateSelection => {
                LaunchRoute::Automatic(self.translation_action_id.clone())
            }
            LaunchMode::DefaultAction => self
                .default_action_id
                .clone()
                .map(LaunchRoute::Automatic)
                .unwrap_or(LaunchRoute::MissingDefaultAction),
        }
    }

    pub fn dismiss_override(&self, action: &str) -> Option<DismissPolicy> {
        self.dismiss_overrides.get(action).copied()
    }

    pub fn providers(&self) -> &[ProviderConfig] {
        &self.providers
    }

    pub const fn default_profile(&self) -> Option<&ModelProfile> {
        self.default_profile.as_ref()
    }

    pub fn provider(&self, id: &ProviderId) -> Option<&ProviderConfig> {
        self.providers.iter().find(|provider| provider.id() == id)
    }

    pub fn with_hotkey(mut self, hotkey: GlobalHotkey) -> Self {
        self.hotkey = hotkey;
        self
    }

    pub fn with_theme(mut self, theme: ThemePreference) -> Self {
        self.theme = theme;
        self
    }

    pub fn with_launch_at_startup(mut self, enabled: bool) -> Self {
        self.launch_at_startup = enabled;
        self
    }

    pub fn with_popup_retention_seconds(mut self, seconds: u64) -> Self {
        self.popup_retention_seconds = seconds;
        self
    }

    pub fn with_launch_mode(mut self, mode: LaunchMode) -> Self {
        self.launch_mode = mode;
        self
    }

    pub fn with_default_action_id(mut self, id: Option<String>) -> Self {
        self.default_action_id = id;
        self
    }

    pub fn with_dismiss_override(
        mut self,
        action: impl Into<String>,
        policy: Option<DismissPolicy>,
    ) -> Self {
        let action = action.into();
        if let Some(policy) = policy {
            self.dismiss_overrides.insert(action, policy);
        } else {
            self.dismiss_overrides.remove(&action);
        }
        self
    }

    pub fn with_provider(mut self, provider: ProviderConfig, profile: ModelProfile) -> Self {
        self.providers
            .retain(|existing| existing.id() != provider.id());
        self.providers.push(provider);
        self.default_profile = Some(profile);
        self
    }
}

fn default_translation_action() -> String {
    "org.lexwisp.translate/translate".into()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsSnapshot {
    settings: AppSettings,
    generation: u64,
}

impl SettingsSnapshot {
    pub const fn new(settings: AppSettings, generation: u64) -> Self {
        Self {
            settings,
            generation,
        }
    }

    pub const fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SettingsError {
    #[error("settings schema version {0} is not supported")]
    UnsupportedSchema(u32),
    #[error("settings are invalid: {0}")]
    Invalid(String),
    #[error("settings could not be loaded: {0}")]
    Load(String),
    #[error("settings could not be saved: {0}")]
    Save(String),
    #[error("Windows integration could not be updated: {0}")]
    Platform(String),
}

pub type SettingsFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SettingsSnapshot, SettingsError>> + Send + 'a>>;

pub trait SettingsUiPort: Send + Sync {
    fn snapshot(&self) -> SettingsSnapshot;
    fn apply(&self, settings: AppSettings) -> SettingsFuture<'_>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_are_valid() {
        assert_eq!(AppSettings::default().validate(), Ok(()));
    }

    #[test]
    fn excessive_popup_retention_is_rejected() {
        let settings = AppSettings::default().with_popup_retention_seconds(601);
        assert!(matches!(
            settings.validate(),
            Err(SettingsError::Invalid(_))
        ));
    }

    #[test]
    fn all_launch_modes_have_explicit_selection_and_no_selection_routes() {
        for mode in LaunchMode::ALL {
            let settings = AppSettings::default()
                .with_launch_mode(mode)
                .with_default_action_id(Some("org.lexwisp.polish/polish".into()));
            assert_eq!(settings.launch_route(false), LaunchRoute::QuickAsk);
            match mode {
                LaunchMode::ActionPalette => {
                    assert_eq!(settings.launch_route(true), LaunchRoute::ActionPalette)
                }
                LaunchMode::TranslateSelection => assert_eq!(
                    settings.launch_route(true),
                    LaunchRoute::Automatic("org.lexwisp.translate/translate".into())
                ),
                LaunchMode::DefaultAction => assert_eq!(
                    settings.launch_route(true),
                    LaunchRoute::Automatic("org.lexwisp.polish/polish".into())
                ),
            }
        }
        assert_eq!(
            AppSettings::default()
                .with_launch_mode(LaunchMode::DefaultAction)
                .launch_route(true),
            LaunchRoute::MissingDefaultAction
        );
    }
}

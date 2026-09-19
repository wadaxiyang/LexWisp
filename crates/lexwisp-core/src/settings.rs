use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;

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
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            hotkey: GlobalHotkey::ControlAltSpace,
            theme: ThemePreference::System,
            launch_at_startup: false,
            popup_retention_seconds: 30,
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
}

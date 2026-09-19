use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use lexwisp_core::{AppSettings, SettingsError, SettingsFuture, SettingsSnapshot, SettingsUiPort};
use lexwisp_platform_windows::WindowsShellHandle;
use lexwisp_storage::ConfigStore;
use tokio::sync::Mutex;

use crate::{ExecutionStore, ProviderRegistry, TaskScope};

pub struct SettingsService {
    current: RwLock<SettingsSnapshot>,
    apply_lock: Mutex<()>,
    config: ConfigStore,
    shell: WindowsShellHandle,
    executable: PathBuf,
    tasks: TaskScope,
    executions: Arc<ExecutionStore>,
    providers: ProviderRegistry,
}

impl SettingsService {
    pub fn new(
        initial: AppSettings,
        config: ConfigStore,
        shell: WindowsShellHandle,
        executable: PathBuf,
        tasks: TaskScope,
        executions: Arc<ExecutionStore>,
        providers: ProviderRegistry,
    ) -> Self {
        Self {
            current: RwLock::new(SettingsSnapshot::new(initial, 0)),
            apply_lock: Mutex::new(()),
            config,
            shell,
            executable,
            tasks,
            executions,
            providers,
        }
    }

    pub fn config_path(&self) -> &Path {
        self.config.path()
    }

    async fn apply_inner(&self, settings: AppSettings) -> Result<SettingsSnapshot, SettingsError> {
        settings.validate()?;
        let _guard = self.apply_lock.lock().await;
        let previous = self.snapshot();
        let previous_settings = previous.settings().clone();

        if settings.hotkey() != previous_settings.hotkey() {
            self.shell
                .replace_hotkey(settings.hotkey())
                .await
                .map_err(|error| SettingsError::Platform(error.to_string()))?;
        }
        if settings.launch_at_startup() != previous_settings.launch_at_startup()
            && let Err(error) = self
                .shell
                .set_startup(settings.launch_at_startup(), self.executable.clone())
                .await
        {
            let _ = self.shell.replace_hotkey(previous_settings.hotkey()).await;
            return Err(SettingsError::Platform(error.to_string()));
        }

        let config = self.config.clone();
        let to_save = settings.clone();
        let saved = self
            .tasks
            .spawn_blocking(move || config.save(&to_save))
            .await
            .map_err(|error| SettingsError::Save(error.to_string()))?;
        if let Err(error) = saved {
            let _ = self.shell.replace_hotkey(previous_settings.hotkey()).await;
            let _ = self
                .shell
                .set_startup(
                    previous_settings.launch_at_startup(),
                    self.executable.clone(),
                )
                .await;
            return Err(error);
        }

        let next = SettingsSnapshot::new(settings, previous.generation().saturating_add(1));
        *self
            .current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = next.clone();
        self.providers.replace(next.settings(), next.generation());
        if next.settings().recording_enabled() != previous_settings.recording_enabled() {
            self.executions
                .set_recording_enabled(next.settings().recording_enabled());
        }
        Ok(next)
    }

    async fn reload_inner(&self) -> Result<SettingsSnapshot, SettingsError> {
        let config = self.config.clone();
        let loaded = self
            .tasks
            .spawn_blocking(move || config.load())
            .await
            .map_err(|error| SettingsError::Load(error.to_string()))??;
        self.apply_inner(loaded.settings().clone()).await
    }
}

impl SettingsUiPort for SettingsService {
    fn snapshot(&self) -> SettingsSnapshot {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn apply(&self, settings: AppSettings) -> SettingsFuture<'_> {
        Box::pin(self.apply_inner(settings))
    }

    fn reload(&self) -> SettingsFuture<'_> {
        Box::pin(self.reload_inner())
    }
}

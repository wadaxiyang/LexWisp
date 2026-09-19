use std::{path::PathBuf, sync::RwLock};

use lexwisp_core::{AppSettings, SettingsError, SettingsFuture, SettingsSnapshot, SettingsUiPort};
use lexwisp_platform_windows::WindowsShellHandle;
use lexwisp_storage::ConfigStore;
use tokio::sync::Mutex;

use crate::TaskScope;

pub struct SettingsService {
    current: RwLock<SettingsSnapshot>,
    apply_lock: Mutex<()>,
    config: ConfigStore,
    shell: WindowsShellHandle,
    executable: PathBuf,
    tasks: TaskScope,
}

impl SettingsService {
    pub fn new(
        initial: AppSettings,
        config: ConfigStore,
        shell: WindowsShellHandle,
        executable: PathBuf,
        tasks: TaskScope,
    ) -> Self {
        Self {
            current: RwLock::new(SettingsSnapshot::new(initial, 0)),
            apply_lock: Mutex::new(()),
            config,
            shell,
            executable,
            tasks,
        }
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
        Ok(next)
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
}

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use lexwisp_core::{AppSettings, AtomicFileWriter, SettingsError};

mod content;

pub use content::{ContentStore, ContentStoreOwner, StorageError, WriteReceipt};

#[derive(Clone)]
pub struct ConfigStore {
    path: PathBuf,
    writer: Arc<dyn AtomicFileWriter>,
}

#[derive(Clone, Debug)]
pub struct LoadedSettings {
    settings: AppSettings,
    first_run: bool,
}

impl LoadedSettings {
    pub const fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub const fn first_run(&self) -> bool {
        self.first_run
    }
}

impl ConfigStore {
    pub fn discover(
        executable_path: &Path,
        writer: Arc<dyn AtomicFileWriter>,
    ) -> Result<Self, SettingsError> {
        let executable_directory = executable_path.parent().ok_or_else(|| {
            SettingsError::Load("the executable path has no parent directory".into())
        })?;
        let data_directory = if executable_directory.join("portable.flag").is_file() {
            executable_directory.join("data")
        } else {
            let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
                SettingsError::Load("LOCALAPPDATA is not available for this process".into())
            })?;
            PathBuf::from(local_app_data).join("LexWisp")
        };
        Ok(Self {
            path: data_directory.join("settings.toml"),
            writer,
        })
    }

    pub fn at(path: PathBuf, writer: Arc<dyn AtomicFileWriter>) -> Self {
        Self { path, writer }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn data_directory(&self) -> &Path {
        self.path
            .parent()
            .expect("ConfigStore paths are always created with a data directory")
    }

    pub fn load(&self) -> Result<LoadedSettings, SettingsError> {
        let source = match fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(LoadedSettings {
                    settings: AppSettings::default(),
                    first_run: true,
                });
            }
            Err(error) => return Err(SettingsError::Load(error.to_string())),
        };
        let settings: AppSettings =
            toml::from_str(&source).map_err(|error| SettingsError::Load(error.to_string()))?;
        settings.validate()?;
        Ok(LoadedSettings {
            settings,
            first_run: false,
        })
    }

    pub fn save(&self, settings: &AppSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        let source = toml::to_string_pretty(settings)
            .map_err(|error| SettingsError::Save(error.to_string()))?;
        self.writer
            .write_atomic(&self.path, source.as_bytes())
            .map_err(|error| SettingsError::Save(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use lexwisp_core::{DismissPolicy, FileWriteError, GlobalHotkey, LaunchMode};

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestWriter;

    impl AtomicFileWriter for TestWriter {
        fn write_atomic(&self, destination: &Path, contents: &[u8]) -> Result<(), FileWriteError> {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| FileWriteError::CreateDirectory(error.to_string()))?;
            }
            fs::write(destination, contents)
                .map_err(|error| FileWriteError::WriteTemporary(error.to_string()))
        }
    }

    fn isolated_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the test clock must be after the Unix epoch")
            .as_nanos();
        env::temp_dir()
            .join("lexwisp-stage1-tests")
            .join(format!(
                "{}-{nonce}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ))
            .join("settings.toml")
    }

    #[test]
    fn settings_round_trip_without_losing_fields() {
        let path = isolated_path();
        let store = ConfigStore::at(path.clone(), Arc::new(TestWriter));
        let expected = AppSettings::default()
            .with_hotkey(GlobalHotkey::AltShiftSpace)
            .with_launch_at_startup(true)
            .with_popup_retention_seconds(0)
            .with_launch_mode(LaunchMode::DefaultAction)
            .with_default_action_id(Some("org.lexwisp.polish/polish".into()))
            .with_dismiss_override(
                "org.lexwisp.translate/translate",
                Some(DismissPolicy::Continue),
            );
        store.save(&expected).expect("settings should save");

        let loaded = store.load().expect("settings should load");
        assert_eq!(loaded.settings(), &expected);
        assert!(!loaded.first_run());

        let test_root = path
            .parent()
            .expect("the isolated path has a bounded test root");
        fs::remove_dir_all(test_root).expect("the bounded test directory should be removable");
    }

    #[test]
    fn corrupt_settings_are_reported_and_preserved() {
        let path = isolated_path();
        fs::create_dir_all(path.parent().expect("settings have a parent"))
            .expect("test directory should be created");
        fs::write(&path, "this is not toml = [").expect("fixture should be written");
        let store = ConfigStore::at(path.clone(), Arc::new(TestWriter));

        assert!(matches!(store.load(), Err(SettingsError::Load(_))));
        assert_eq!(
            fs::read_to_string(&path).expect("fixture should remain"),
            "this is not toml = ["
        );

        let test_root = path
            .parent()
            .expect("the isolated path has a bounded test root");
        fs::remove_dir_all(test_root).expect("the bounded test directory should be removable");
    }

    #[test]
    fn missing_settings_are_a_first_run() {
        let store = ConfigStore::at(isolated_path(), Arc::new(TestWriter));
        let loaded = store.load().expect("missing settings should use defaults");
        assert!(loaded.first_run());
        assert_eq!(loaded.settings(), &AppSettings::default());
    }
}

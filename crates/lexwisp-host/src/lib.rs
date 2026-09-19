mod registry;
mod settings;
mod tasks;

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_channel::Sender;
use lexwisp_core::{AppSettings, HostUiCommand, SettingsUiPort, TaskOwner};
use lexwisp_platform_windows::WindowsShellHandle;
use lexwisp_storage::ConfigStore;
use tokio::runtime::{Builder, Runtime};

pub use registry::{
    ActionRegistry, CapabilityAuthority, PluginRegistry, RegistryError, RegistryEvent,
};
pub use tasks::{HostTaskPort, TaskScope};

use settings::SettingsService;

#[derive(Clone)]
pub struct HostUiCommandPort {
    sender: Sender<HostUiCommand>,
}

impl HostUiCommandPort {
    pub fn send(&self, command: HostUiCommand) -> Result<(), String> {
        self.sender
            .try_send(command)
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub struct HostHandles {
    plugins: PluginRegistry,
    actions: ActionRegistry,
    capabilities: CapabilityAuthority,
    tasks: HostTaskPort,
    settings: Arc<dyn SettingsUiPort>,
    ui_commands: HostUiCommandPort,
}

impl HostHandles {
    pub const fn plugins(&self) -> &PluginRegistry {
        &self.plugins
    }

    pub const fn actions(&self) -> &ActionRegistry {
        &self.actions
    }

    pub const fn capabilities(&self) -> &CapabilityAuthority {
        &self.capabilities
    }

    pub const fn tasks(&self) -> &HostTaskPort {
        &self.tasks
    }

    pub fn settings(&self) -> Arc<dyn SettingsUiPort> {
        self.settings.clone()
    }

    pub const fn ui_commands(&self) -> &HostUiCommandPort {
        &self.ui_commands
    }
}

pub struct Host {
    runtime: Runtime,
    tasks: HostTaskPort,
}

impl Host {
    pub fn build(
        initial_settings: AppSettings,
        config: ConfigStore,
        shell: WindowsShellHandle,
        ui_commands: Sender<HostUiCommand>,
        executable: PathBuf,
    ) -> Result<(Self, HostHandles), String> {
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lexwisp-business")
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the business runtime: {error}"))?;
        let tasks = HostTaskPort::new(runtime.handle().clone());
        let process_tasks = tasks.scope(TaskOwner::Process);
        let settings: Arc<dyn SettingsUiPort> = Arc::new(SettingsService::new(
            initial_settings,
            config,
            shell,
            executable,
            process_tasks,
        ));
        let plugins = PluginRegistry::default();
        let actions = plugins.actions();
        let handles = HostHandles {
            plugins,
            actions,
            capabilities: CapabilityAuthority::default(),
            tasks: tasks.clone(),
            settings,
            ui_commands: HostUiCommandPort {
                sender: ui_commands,
            },
        };
        Ok((Self { runtime, tasks }, handles))
    }

    pub fn shutdown(self) {
        self.tasks.cancel_all();
        self.runtime.shutdown_timeout(Duration::from_secs(3));
    }
}

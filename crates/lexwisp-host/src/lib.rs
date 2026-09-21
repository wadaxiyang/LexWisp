mod ai;
mod chat;
mod execution;
mod history;
mod invocation;
mod providers;
mod settings;
mod tasks;

use std::{path::PathBuf, sync::Arc, time::Duration};

use async_channel::Sender;
use lexwisp_core::{
    AppSettings, ChatHistoryPort, ChatRunPort, HistoryUiPort, HostUiCommand, ProviderUiPort,
    SettingsUiPort, TaskOwner,
};
use lexwisp_platform_windows::{WindowsCredentialStore, WindowsShellHandle};
use lexwisp_storage::{ConfigStore, ContentStoreOwner};
use tokio::runtime::{Builder, Runtime};

pub use ai::AiService;
pub use chat::ChatController;
pub use execution::ExecutionStore;
pub use history::HistoryService;
pub use invocation::RunSupervisor;
pub use providers::{ProviderRegistry, ProviderService};
pub use tasks::{HostTaskPort, TaskScope};

use invocation::ChatRunner;
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
    tasks: HostTaskPort,
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    history: Arc<dyn HistoryUiPort>,
    chat_history: Arc<dyn ChatHistoryPort>,
    executions: Arc<ExecutionStore>,
    supervisor: Arc<RunSupervisor>,
    ui_commands: HostUiCommandPort,
}

impl HostHandles {
    pub const fn tasks(&self) -> &HostTaskPort {
        &self.tasks
    }
    pub fn settings(&self) -> Arc<dyn SettingsUiPort> {
        self.settings.clone()
    }
    pub fn providers(&self) -> Arc<dyn ProviderUiPort> {
        self.providers.clone()
    }
    pub fn history(&self) -> Arc<dyn HistoryUiPort> {
        self.history.clone()
    }
    pub fn chat_history(&self) -> Arc<dyn ChatHistoryPort> {
        self.chat_history.clone()
    }
    pub fn executions(&self) -> Arc<ExecutionStore> {
        self.executions.clone()
    }
    pub fn chat_run_port(&self) -> Arc<dyn ChatRunPort> {
        Arc::new(ChatRunner::new(self.supervisor.clone(), self.tasks.clone()))
    }
    pub const fn ui_commands(&self) -> &HostUiCommandPort {
        &self.ui_commands
    }
}

pub struct Host {
    runtime: Runtime,
    tasks: HostTaskPort,
    supervisor: Arc<RunSupervisor>,
    content: ContentStoreOwner,
}

impl Host {
    pub fn build(
        initial_settings: AppSettings,
        config: ConfigStore,
        shell: WindowsShellHandle,
        ui_commands: Sender<HostUiCommand>,
        executable: PathBuf,
    ) -> Result<(Self, HostHandles), String> {
        let data_directory = config.data_directory().to_path_buf();
        let database_path = data_directory.join("lexwisp.db");
        let (content_owner, content) =
            ContentStoreOwner::start(database_path).map_err(|error| error.to_string())?;
        let retention_generation = content
            .retention_generation()
            .map_err(|error| error.to_string())?;
        let runtime = Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lexwisp-business")
            .enable_all()
            .build()
            .map_err(|error| format!("could not start the business runtime: {error}"))?;
        let tasks = HostTaskPort::new(runtime.handle().clone());
        let process_tasks = tasks.scope(TaskOwner::Process);
        let executions = Arc::new(ExecutionStore::new(
            content.clone(),
            initial_settings.recording_enabled(),
            retention_generation,
        ));
        let provider_registry = ProviderRegistry::new(&initial_settings);
        let settings_service = Arc::new(SettingsService::new(
            initial_settings,
            config,
            shell,
            executable,
            process_tasks.clone(),
            executions.clone(),
            provider_registry.clone(),
        ));
        let settings: Arc<dyn SettingsUiPort> = settings_service.clone();
        let credentials: Arc<dyn lexwisp_core::CredentialStore> = Arc::new(WindowsCredentialStore);
        let ai = Arc::new(AiService::new().map_err(|error| error.to_string())?);
        let chat_history: Arc<dyn ChatHistoryPort> = Arc::new(content.clone());
        let supervisor = Arc::new(RunSupervisor::new(
            ai.clone(),
            provider_registry.clone(),
            credentials.clone(),
            executions.clone(),
        ));
        let providers: Arc<dyn ProviderUiPort> = Arc::new(ProviderService::new(
            settings_service.clone(),
            provider_registry,
            credentials,
            ai,
            process_tasks.clone(),
        ));
        let history: Arc<dyn HistoryUiPort> = Arc::new(HistoryService::new(
            content,
            executions.clone(),
            settings_service,
            process_tasks,
            data_directory,
        ));
        let handles = HostHandles {
            tasks: tasks.clone(),
            settings,
            providers,
            history,
            chat_history,
            executions,
            supervisor: supervisor.clone(),
            ui_commands: HostUiCommandPort {
                sender: ui_commands,
            },
        };
        Ok((
            Self {
                runtime,
                tasks,
                supervisor,
                content: content_owner,
            },
            handles,
        ))
    }

    pub fn shutdown(self) {
        self.supervisor.shutdown();
        self.runtime
            .block_on(self.supervisor.wait_until_idle(Duration::from_secs(2)));
        self.tasks.cancel_all();
        self.runtime.shutdown_timeout(Duration::from_secs(3));
        self.content.shutdown();
    }
}

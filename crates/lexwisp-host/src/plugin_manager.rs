use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{self, File},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

use lexwisp_core::{
    ActionDescriptor, ActionHandler, Capability, HostUiCommand, ManagedActionSnapshot,
    ManagedPluginStatus, ManagedPluginSummary, PluginId, PluginImportPreview, PluginKind,
    PluginManagementFuture, PluginManagementUiPort, ScriptInvocationHost, ScriptPackageDefinition,
    ScriptPackageFactory, ScriptPluginLifecycle, SettingsUiPort, TextActionUiPort, TextRunPort,
};
use lexwisp_storage::{ContentStore, StoredPlugin};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zip::ZipArchive;

use crate::{
    CapabilityAuthority, DeclarativeController, DeclarativePackage, HostTaskPort,
    InvocationSupervisor, PluginRegistry, invocation::ScopedTextRunPort,
};

const MAX_ARCHIVE_BYTES: u64 = 10 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_FILE_COUNT: usize = 256;
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
const MAX_PROMPT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_ICON_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub struct PluginManager {
    inner: Arc<PluginManagerInner>,
}

struct PluginManagerInner {
    plugins_root: PathBuf,
    staging_root: PathBuf,
    content: ContentStore,
    registry: PluginRegistry,
    capabilities: CapabilityAuthority,
    supervisor: Arc<InvocationSupervisor>,
    tasks: HostTaskPort,
    settings: Arc<dyn SettingsUiPort>,
    ui_commands: async_channel::Sender<HostUiCommand>,
    script_factory: Arc<dyn ScriptPackageFactory>,
    script_http: Arc<reqwest::Client>,
    operation: Mutex<()>,
    state: Mutex<ManagerState>,
}

#[derive(Default)]
struct ManagerState {
    generation: u64,
    installed: BTreeMap<PluginId, RuntimePlugin>,
    previews: HashMap<String, PreparedPackage>,
}

struct RuntimePlugin {
    summary: ManagedPluginSummary,
    descriptors: Vec<ActionDescriptor>,
    controllers: Vec<Arc<dyn TextActionUiPort>>,
    lifecycle: Option<Arc<dyn ScriptPluginLifecycle>>,
}

struct PreparedPackage {
    source_path: PathBuf,
    staging_path: PathBuf,
    package_hash: String,
    package: PreparedPackageKind,
}

enum PreparedPackageKind {
    Declarative(DeclarativePackage),
    Script(ScriptPackageDefinition),
}

impl PreparedPackageKind {
    fn plugin(&self) -> &lexwisp_core::PluginDescriptor {
        match self {
            Self::Declarative(package) => &package.plugin,
            Self::Script(package) => &package.plugin,
        }
    }

    fn actions(&self) -> &[ActionDescriptor] {
        match self {
            Self::Declarative(package) => &package.actions,
            Self::Script(package) => &package.actions,
        }
    }

    fn version(&self) -> String {
        match self {
            Self::Declarative(package) => package.version.to_string(),
            Self::Script(package) => package.version.clone(),
        }
    }

    const fn kind(&self) -> PluginKind {
        match self {
            Self::Declarative(_) => PluginKind::Declarative,
            Self::Script(_) => PluginKind::Script,
        }
    }

    fn network_scopes(&self) -> Vec<String> {
        match self {
            Self::Declarative(_) => Vec::new(),
            Self::Script(package) => package
                .network_rules
                .iter()
                .map(|rule| {
                    format!(
                        "{}://{}:{} {} {}",
                        rule.scheme,
                        rule.host,
                        rule.port
                            .unwrap_or(if rule.scheme == "https" { 443 } else { 80 }),
                        rule.methods.join("/"),
                        if rule.path_prefixes.is_empty() {
                            "/".into()
                        } else {
                            rule.path_prefixes.join(",")
                        }
                    )
                })
                .collect(),
        }
    }
}

impl PluginManager {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        data_directory: &Path,
        content: ContentStore,
        registry: PluginRegistry,
        capabilities: CapabilityAuthority,
        supervisor: Arc<InvocationSupervisor>,
        tasks: HostTaskPort,
        settings: Arc<dyn SettingsUiPort>,
        ui_commands: async_channel::Sender<HostUiCommand>,
        script_factory: Arc<dyn ScriptPackageFactory>,
    ) -> Result<Self, String> {
        let plugins_root = data_directory.join("plugins");
        let staging_root = plugins_root.join(".staging");
        fs::create_dir_all(&staging_root)
            .map_err(|error| format!("could not create plugin staging directory: {error}"))?;
        // Script HTTP has a stricter transport profile than Provider traffic, but every Script
        // plugin shares this one Host-owned pool instead of creating a Client per activation.
        let script_http = Arc::new(
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .connect_timeout(std::time::Duration::from_secs(10))
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|error| format!("could not create script HTTP client: {error}"))?,
        );
        Ok(Self {
            inner: Arc::new(PluginManagerInner {
                plugins_root,
                staging_root,
                content,
                registry,
                capabilities,
                supervisor,
                tasks,
                settings,
                ui_commands,
                script_factory,
                script_http,
                operation: Mutex::new(()),
                state: Mutex::new(ManagerState::default()),
            }),
        })
    }

    pub fn activate_installed(&self) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for stored in self
            .inner
            .content
            .list_plugins()
            .map_err(|error| error.to_string())?
        {
            let plugin_id =
                PluginId::parse(stored.id.clone()).map_err(|error| error.to_string())?;
            let capabilities = stored
                .capabilities
                .iter()
                .map(|value| Capability::parse_manifest(value))
                .collect::<Result<Vec<_>, _>>()?;
            let mut summary = stored_summary(&stored, plugin_id.clone(), capabilities.clone());
            if !stored.enabled {
                self.inner
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .installed
                    .insert(
                        plugin_id,
                        RuntimePlugin {
                            summary,
                            descriptors: Vec::new(),
                            controllers: Vec::new(),
                            lifecycle: None,
                        },
                    );
                continue;
            }
            match prepare_directory(
                Path::new(&stored.install_path),
                None,
                self.inner.script_factory.as_ref(),
            ) {
                Ok(prepared) if prepared.package_hash == stored.package_hash => {
                    let predicted = self.inner.registry.generation().saturating_add(1);
                    match self.activate_package(&prepared, predicted, false) {
                        Ok(mut runtime) => {
                            runtime.summary.source_path = PathBuf::from(&stored.source_path);
                            runtime.summary.install_path = PathBuf::from(&stored.install_path);
                            self.inner.capabilities.replace_bound_grants(
                                plugin_id.clone(),
                                stored.package_hash.clone(),
                                predicted,
                                capabilities,
                            );
                            self.inner
                                .content
                                .save_plugin(summary_to_stored(&runtime.summary))
                                .map_err(|error| error.to_string())?;
                            self.inner
                                .state
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .installed
                                .insert(plugin_id, runtime);
                        }
                        Err(error) => {
                            summary.status = ManagedPluginStatus::Faulted;
                            summary.last_error = Some(error);
                            self.insert_faulted(summary);
                        }
                    }
                    cleanup_prepared(&prepared.staging_path);
                }
                Ok(prepared) => {
                    cleanup_prepared(&prepared.staging_path);
                    summary.status = ManagedPluginStatus::Faulted;
                    summary.last_error =
                        Some("managed package content changed; choose Reload to review it".into());
                    self.insert_faulted(summary);
                }
                Err(error) => {
                    summary.status = ManagedPluginStatus::Faulted;
                    summary.last_error = Some(error);
                    self.insert_faulted(summary);
                }
            }
        }
        Ok(())
    }

    fn insert_faulted(&self, summary: ManagedPluginSummary) {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .insert(
                summary.id.clone(),
                RuntimePlugin {
                    summary,
                    descriptors: Vec::new(),
                    controllers: Vec::new(),
                    lifecycle: None,
                },
            );
    }

    fn preview_sync(&self, source: PathBuf) -> Result<PluginImportPreview, String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stale = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .previews
                .drain()
                .map(|(_, prepared)| prepared.staging_path)
                .collect::<Vec<_>>()
        };
        for path in stale {
            cleanup_prepared(&path);
        }
        let token = Uuid::new_v4().to_string();
        let staging = self.inner.staging_root.join(&token);
        let prepared = if source.is_dir() {
            prepare_directory(&source, Some(staging), self.inner.script_factory.as_ref())?
        } else {
            prepare_zip(&source, &staging, self.inner.script_factory.as_ref())?
        };
        let plugin_id = prepared.package.plugin().id().clone();
        let installed = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(&plugin_id)
            .map(|runtime| runtime.summary.clone());
        if installed.is_none()
            && self
                .inner
                .registry
                .descriptors()
                .iter()
                .any(|descriptor| descriptor.id() == &plugin_id)
        {
            cleanup_prepared(&prepared.staging_path);
            return Err(format!(
                "plugin ID '{plugin_id}' is reserved by an existing package"
            ));
        }
        if installed.as_ref().is_some_and(|current| {
            current.package_hash == prepared.package_hash && current.install_path != source
        }) {
            cleanup_prepared(&prepared.staging_path);
            return Err("this exact plugin package is already installed".into());
        }
        let requested = prepared.package.plugin().requested_capabilities().to_vec();
        let old_grants = installed
            .as_ref()
            .map(|current| current.granted_capabilities.as_slice())
            .unwrap_or_default();
        let added = requested
            .iter()
            .copied()
            .filter(|capability| !old_grants.contains(capability))
            .collect();
        let preview = PluginImportPreview {
            token: token.clone(),
            id: plugin_id,
            name: prepared.package.plugin().display_name().to_owned(),
            version: prepared.package.version(),
            kind: prepared.package.kind(),
            source_path: source,
            package_hash: prepared.package_hash.clone(),
            requested_capabilities: requested,
            added_capabilities: added,
            network_scopes: prepared.package.network_scopes(),
            actions: prepared
                .package
                .actions()
                .iter()
                .map(|action| action.display_name().to_owned())
                .collect(),
            replaces_version: installed.map(|current| current.version),
        };
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .previews
            .insert(token, prepared);
        Ok(preview)
    }

    fn confirm_sync(&self, token: &str) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prepared = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .previews
            .remove(token)
            .ok_or_else(|| "the import preview expired or was already used".to_string())?;
        let plugin_id = prepared.package.plugin().id().clone();
        let old = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(&plugin_id)
            .map(|runtime| runtime.summary.clone());
        let final_path = self
            .inner
            .plugins_root
            .join(plugin_id.as_str())
            .join(format!(
                "{}-{}",
                prepared.package.version(),
                &prepared.package_hash[..12]
            ));
        let reuse_existing = old.as_ref().is_some_and(|current| {
            current.package_hash == prepared.package_hash && current.install_path == final_path
        });
        if final_path.exists() && !reuse_existing {
            cleanup_prepared(&prepared.staging_path);
            return Err("the managed destination already exists".into());
        }
        if !reuse_existing {
            fs::create_dir_all(final_path.parent().ok_or("invalid managed plugin path")?)
                .map_err(|error| format!("could not create managed plugin directory: {error}"))?;
            fs::rename(&prepared.staging_path, &final_path)
                .map_err(|error| format!("could not commit plugin files: {error}"))?;
        }

        let generation = self.inner.registry.generation().saturating_add(1);
        let grants = prepared.package.plugin().requested_capabilities().to_vec();
        let stored = StoredPlugin {
            id: plugin_id.to_string(),
            name: prepared.package.plugin().display_name().to_owned(),
            version: prepared.package.version(),
            kind: prepared.package.kind().manifest_name().to_owned(),
            package_hash: prepared.package_hash.clone(),
            source_path: prepared.source_path.to_string_lossy().into_owned(),
            install_path: final_path.to_string_lossy().into_owned(),
            enabled: true,
            generation,
            capabilities: grants
                .iter()
                .map(|capability| capability.manifest_name().to_owned())
                .collect(),
            last_error: None,
        };
        if let Err(error) = self.inner.content.save_plugin(stored.clone()) {
            if reuse_existing {
                cleanup_prepared(&prepared.staging_path);
            } else {
                cleanup_managed(&final_path, &self.inner.plugins_root);
            }
            return Err(error.to_string());
        }

        self.inner.supervisor.cancel_plugin(&plugin_id);
        self.inner.capabilities.revoke(&plugin_id);
        let replacing = old.is_some();
        let activation = self.activate_package_at(&prepared, &final_path, generation, replacing);
        let runtime = match activation {
            Ok(runtime) => runtime,
            Err(error) => {
                if let Some(old) = old.as_ref() {
                    let _ = self.inner.content.save_plugin(summary_to_stored(old));
                    self.inner.capabilities.replace_bound_grants(
                        old.id.clone(),
                        old.package_hash.clone(),
                        old.generation,
                        old.granted_capabilities.iter().copied(),
                    );
                } else {
                    let _ = self.inner.content.remove_plugin(plugin_id.as_str());
                }
                if reuse_existing {
                    cleanup_prepared(&prepared.staging_path);
                } else {
                    cleanup_managed(&final_path, &self.inner.plugins_root);
                }
                return Err(error);
            }
        };
        self.inner.capabilities.replace_bound_grants(
            plugin_id.clone(),
            prepared.package_hash.clone(),
            generation,
            grants,
        );
        let replaced = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .insert(plugin_id, runtime);
        if let Some(lifecycle) = replaced.and_then(|runtime| runtime.lifecycle) {
            lifecycle.stop();
        }
        if reuse_existing {
            cleanup_prepared(&prepared.staging_path);
        }
        if let Some(old) = old
            && old.install_path != final_path
        {
            cleanup_managed(&old.install_path, &self.inner.plugins_root);
        }
        self.changed();
        Ok(())
    }

    fn discard_preview_sync(&self, token: &str) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prepared = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .previews
            .remove(token);
        if let Some(prepared) = prepared {
            cleanup_prepared(&prepared.staging_path);
        }
        Ok(())
    }

    fn activate_package_at(
        &self,
        prepared: &PreparedPackage,
        install_path: &Path,
        generation: u64,
        replacing: bool,
    ) -> Result<RuntimePlugin, String> {
        let plugin_id = prepared.package.plugin().id().clone();
        let (registrations, controllers, lifecycle) = match &prepared.package {
            PreparedPackageKind::Declarative(package) => {
                let runner: Arc<dyn TextRunPort> = Arc::new(ScopedTextRunPort::new_bound(
                    plugin_id.clone(),
                    self.inner.registry.actions(),
                    self.inner.capabilities.clone(),
                    self.inner.supervisor.clone(),
                    self.inner.tasks.clone(),
                    prepared.package_hash.clone(),
                    generation,
                ));
                let mut registrations = Vec::new();
                let mut controllers = Vec::new();
                for descriptor in &package.actions {
                    let controller = DeclarativeController::new(
                        descriptor.clone(),
                        runner.clone(),
                        self.inner.settings.clone(),
                    )?;
                    let handler: Arc<dyn ActionHandler> = controller.clone();
                    let port: Arc<dyn TextActionUiPort> = controller;
                    registrations.push((descriptor.clone(), handler));
                    controllers.push(port);
                }
                (registrations, controllers, None)
            }
            PreparedPackageKind::Script(package) => {
                let host: Arc<dyn ScriptInvocationHost> =
                    Arc::new(crate::script::BoundScriptHost::new(
                        plugin_id.clone(),
                        prepared.package_hash.clone(),
                        generation,
                        package.network_rules.clone(),
                        self.inner.registry.actions(),
                        self.inner.capabilities.clone(),
                        self.inner.supervisor.clone(),
                        self.inner.tasks.clone(),
                        self.inner.content.clone(),
                        self.inner.script_http.clone(),
                        self.inner.ui_commands.clone(),
                    ));
                let activation = self.inner.script_factory.activate(
                    install_path,
                    package,
                    host,
                    self.inner.settings.clone(),
                )?;
                if activation.handlers.len() != package.actions.len()
                    || activation.controllers.len() != package.actions.len()
                {
                    activation.lifecycle.stop();
                    return Err("script runtime returned an incomplete action activation".into());
                }
                let registrations = package
                    .actions
                    .iter()
                    .cloned()
                    .zip(activation.handlers)
                    .collect();
                (
                    registrations,
                    activation.controllers,
                    Some(activation.lifecycle),
                )
            }
        };
        let actual_generation = if replacing {
            self.inner
                .registry
                .replace_package(prepared.package.plugin().clone(), registrations)
        } else {
            self.inner
                .registry
                .register_package(prepared.package.plugin().clone(), registrations)
        }
        .map_err(|error| error.to_string())?;
        if actual_generation != generation {
            return Err("plugin registry generation changed during activation".into());
        }
        Ok(RuntimePlugin {
            summary: ManagedPluginSummary {
                kind: prepared.package.kind(),
                id: plugin_id,
                name: prepared.package.plugin().display_name().to_owned(),
                version: prepared.package.version(),
                source_path: prepared.source_path.clone(),
                install_path: install_path.to_path_buf(),
                package_hash: prepared.package_hash.clone(),
                generation,
                status: ManagedPluginStatus::Enabled,
                granted_capabilities: prepared.package.plugin().requested_capabilities().to_vec(),
                actions: prepared
                    .package
                    .actions()
                    .iter()
                    .map(|action| action.display_name().to_owned())
                    .collect(),
                last_error: None,
            },
            descriptors: prepared.package.actions().to_vec(),
            controllers,
            lifecycle,
        })
    }

    fn activate_package(
        &self,
        prepared: &PreparedPackage,
        generation: u64,
        replacing: bool,
    ) -> Result<RuntimePlugin, String> {
        self.activate_package_at(prepared, &prepared.source_path, generation, replacing)
    }

    fn set_enabled_sync(&self, plugin_id: PluginId, enabled: bool) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let summary = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(&plugin_id)
            .map(|runtime| runtime.summary.clone())
            .ok_or_else(|| format!("plugin '{plugin_id}' is not installed"))?;
        if enabled == (summary.status == ManagedPluginStatus::Enabled) {
            return Ok(());
        }
        if !enabled {
            let mut disabled = summary.clone();
            disabled.status = ManagedPluginStatus::Disabled;
            disabled.generation = self.inner.registry.generation().saturating_add(1);
            self.inner
                .content
                .save_plugin(summary_to_stored(&disabled))
                .map_err(|error| error.to_string())?;
            self.inner.supervisor.cancel_plugin(&plugin_id);
            self.inner.capabilities.revoke(&plugin_id);
            self.inner.registry.remove_package(&plugin_id);
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(runtime) = state.installed.get_mut(&plugin_id) {
                if let Some(lifecycle) = runtime.lifecycle.take() {
                    lifecycle.stop();
                }
                runtime.summary = disabled;
                runtime.descriptors.clear();
                runtime.controllers.clear();
            }
            drop(state);
            self.changed();
            return Ok(());
        }

        let prepared = prepare_directory(
            &summary.install_path,
            None,
            self.inner.script_factory.as_ref(),
        )?;
        if prepared.package_hash != summary.package_hash {
            cleanup_prepared(&prepared.staging_path);
            return Err("managed files changed; choose Reload to preview and confirm them".into());
        }
        let generation = self.inner.registry.generation().saturating_add(1);
        let mut enabled_summary = summary.clone();
        enabled_summary.status = ManagedPluginStatus::Enabled;
        enabled_summary.generation = generation;
        enabled_summary.last_error = None;
        self.inner
            .content
            .save_plugin(summary_to_stored(&enabled_summary))
            .map_err(|error| error.to_string())?;
        let activation =
            self.activate_package_at(&prepared, &summary.install_path, generation, false);
        cleanup_prepared(&prepared.staging_path);
        let mut runtime = match activation {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = self.inner.content.save_plugin(summary_to_stored(&summary));
                return Err(error);
            }
        };
        runtime.summary.source_path = summary.source_path.clone();
        self.inner.capabilities.replace_bound_grants(
            plugin_id.clone(),
            summary.package_hash.clone(),
            generation,
            summary.granted_capabilities.iter().copied(),
        );
        let replaced = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .insert(plugin_id, runtime);
        if let Some(lifecycle) = replaced.and_then(|runtime| runtime.lifecycle) {
            lifecycle.stop();
        }
        self.changed();
        Ok(())
    }

    fn uninstall_sync(&self, plugin_id: PluginId) -> Result<(), String> {
        let _operation = self
            .inner
            .operation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let summary = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(&plugin_id)
            .map(|runtime| runtime.summary.clone())
            .ok_or_else(|| format!("plugin '{plugin_id}' is not installed"))?;
        self.inner
            .content
            .remove_plugin(plugin_id.as_str())
            .map_err(|error| error.to_string())?;
        self.inner.supervisor.cancel_plugin(&plugin_id);
        self.inner.capabilities.revoke(&plugin_id);
        self.inner.registry.remove_package(&plugin_id);
        let removed = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .remove(&plugin_id);
        if let Some(lifecycle) = removed.and_then(|runtime| runtime.lifecycle) {
            lifecycle.stop();
        }
        cleanup_managed(&summary.install_path, &self.inner.plugins_root);
        self.changed();
        Ok(())
    }

    fn changed(&self) {
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.generation = state.generation.saturating_add(1);
        drop(state);
        let _ = self
            .inner
            .ui_commands
            .try_send(HostUiCommand::RefreshPlugins);
    }
}

impl PluginManagementUiPort for PluginManager {
    fn list(&self) -> Vec<ManagedPluginSummary> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .values()
            .map(|runtime| runtime.summary.clone())
            .collect()
    }

    fn action_snapshot(&self) -> ManagedActionSnapshot {
        let state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ManagedActionSnapshot {
            generation: state.generation,
            descriptors: state
                .installed
                .values()
                .flat_map(|runtime| runtime.descriptors.clone())
                .collect(),
            controllers: state
                .installed
                .values()
                .flat_map(|runtime| {
                    runtime.controllers.iter().cloned().map(|controller| {
                        let controller: Arc<dyn TextActionUiPort> = controller;
                        controller
                    })
                })
                .collect(),
        }
    }

    fn preview<'a>(&'a self, source: PathBuf) -> PluginManagementFuture<'a, PluginImportPreview> {
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            scope
                .spawn_blocking(move || this.preview_sync(source))
                .await
                .map_err(|error| format!("plugin preview task failed: {error}"))?
        })
    }

    fn confirm<'a>(&'a self, token: String) -> PluginManagementFuture<'a, ()> {
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            scope
                .spawn_blocking(move || this.confirm_sync(&token))
                .await
                .map_err(|error| format!("plugin install task failed: {error}"))?
        })
    }

    fn discard_preview<'a>(&'a self, token: String) -> PluginManagementFuture<'a, ()> {
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            scope
                .spawn_blocking(move || this.discard_preview_sync(&token))
                .await
                .map_err(|error| format!("plugin preview cleanup failed: {error}"))?
        })
    }

    fn set_enabled<'a>(
        &'a self,
        plugin_id: PluginId,
        enabled: bool,
    ) -> PluginManagementFuture<'a, ()> {
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            scope
                .spawn_blocking(move || this.set_enabled_sync(plugin_id, enabled))
                .await
                .map_err(|error| format!("plugin lifecycle task failed: {error}"))?
        })
    }

    fn preview_reload<'a>(
        &'a self,
        plugin_id: PluginId,
    ) -> PluginManagementFuture<'a, PluginImportPreview> {
        let current = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(&plugin_id)
            .map(|runtime| {
                (
                    runtime.summary.install_path.clone(),
                    runtime.summary.source_path.clone(),
                )
            });
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            let (source, original_source) =
                current.ok_or_else(|| format!("plugin '{plugin_id}' is not installed"))?;
            scope
                .spawn_blocking(move || {
                    let preview = this.preview_sync(source)?;
                    if let Some(prepared) = this
                        .inner
                        .state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .previews
                        .get_mut(&preview.token)
                    {
                        prepared.source_path = original_source;
                    }
                    Ok(preview)
                })
                .await
                .map_err(|error| format!("plugin reload preview failed: {error}"))?
        })
    }

    fn uninstall<'a>(&'a self, plugin_id: PluginId) -> PluginManagementFuture<'a, ()> {
        let this = self.clone();
        let scope = self.inner.tasks.scope(lexwisp_core::TaskOwner::Process);
        Box::pin(async move {
            scope
                .spawn_blocking(move || this.uninstall_sync(plugin_id))
                .await
                .map_err(|error| format!("plugin uninstall task failed: {error}"))?
        })
    }

    fn open_directory(&self, plugin_id: &PluginId) -> Result<(), String> {
        let path = self
            .inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .installed
            .get(plugin_id)
            .map(|runtime| runtime.summary.install_path.clone())
            .ok_or_else(|| format!("plugin '{plugin_id}' is not installed"))?;
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("could not open plugin directory: {error}"))
    }
}

fn stored_summary(
    stored: &StoredPlugin,
    id: PluginId,
    capabilities: Vec<Capability>,
) -> ManagedPluginSummary {
    ManagedPluginSummary {
        kind: match stored.kind.as_str() {
            "script" => PluginKind::Script,
            _ => PluginKind::Declarative,
        },
        id,
        name: stored.name.clone(),
        version: stored.version.clone(),
        source_path: PathBuf::from(&stored.source_path),
        install_path: PathBuf::from(&stored.install_path),
        package_hash: stored.package_hash.clone(),
        generation: stored.generation,
        status: if stored.enabled {
            ManagedPluginStatus::Enabled
        } else {
            ManagedPluginStatus::Disabled
        },
        granted_capabilities: capabilities,
        actions: Vec::new(),
        last_error: stored.last_error.clone(),
    }
}

fn summary_to_stored(summary: &ManagedPluginSummary) -> StoredPlugin {
    StoredPlugin {
        id: summary.id.to_string(),
        name: summary.name.clone(),
        version: summary.version.clone(),
        kind: summary.kind.manifest_name().to_owned(),
        package_hash: summary.package_hash.clone(),
        source_path: summary.source_path.to_string_lossy().into_owned(),
        install_path: summary.install_path.to_string_lossy().into_owned(),
        enabled: summary.status == ManagedPluginStatus::Enabled,
        generation: summary.generation,
        capabilities: summary
            .granted_capabilities
            .iter()
            .map(|capability| capability.manifest_name().to_owned())
            .collect(),
        last_error: summary.last_error.clone(),
    }
}

fn prepare_directory(
    source: &Path,
    destination: Option<PathBuf>,
    script_factory: &dyn ScriptPackageFactory,
) -> Result<PreparedPackage, String> {
    if !source.is_dir() {
        return Err(format!(
            "plugin directory '{}' does not exist",
            source.display()
        ));
    }
    let staging_path = destination.unwrap_or_else(|| {
        std::env::temp_dir()
            .join("lexwisp-plugin-preview")
            .join(Uuid::new_v4().to_string())
    });
    fs::create_dir_all(&staging_path)
        .map_err(|error| format!("could not create plugin staging directory: {error}"))?;
    let result = copy_tree(source, &staging_path)
        .and_then(|_| parse_staged(source.to_path_buf(), staging_path.clone(), script_factory));
    if result.is_err() {
        let boundary = staging_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| staging_path.clone());
        cleanup_staging(&staging_path, &boundary);
    }
    result
}

fn prepare_zip(
    source: &Path,
    staging_path: &Path,
    script_factory: &dyn ScriptPackageFactory,
) -> Result<PreparedPackage, String> {
    let metadata =
        fs::metadata(source).map_err(|error| format!("could not inspect plugin ZIP: {error}"))?;
    if metadata.len() > MAX_ARCHIVE_BYTES {
        return Err("plugin ZIP exceeds the 10 MiB compressed limit".into());
    }
    fs::create_dir_all(staging_path)
        .map_err(|error| format!("could not create plugin staging directory: {error}"))?;
    let result = (|| {
        let file =
            File::open(source).map_err(|error| format!("could not open plugin ZIP: {error}"))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|error| format!("invalid or damaged plugin ZIP: {error}"))?;
        if archive.len() > MAX_FILE_COUNT {
            return Err(format!(
                "plugin ZIP contains more than {MAX_FILE_COUNT} entries"
            ));
        }
        let mut seen = HashSet::new();
        let mut total = 0_u64;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| format!("could not read plugin ZIP entry: {error}"))?;
            let relative = validate_relative_path(entry.name())?;
            if relative.as_os_str().is_empty() {
                continue;
            }
            let folded = relative.to_string_lossy().to_lowercase();
            if !seen.insert(folded) {
                return Err(format!(
                    "plugin ZIP contains a case-insensitive path collision at '{}'",
                    relative.display()
                ));
            }
            if entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err("plugin ZIP contains a symbolic link".into());
            }
            total = total
                .checked_add(entry.size())
                .ok_or("plugin extracted size overflow")?;
            if total > MAX_EXTRACTED_BYTES {
                return Err("plugin exceeds the 32 MiB extracted limit".into());
            }
            let output = staging_path.join(&relative);
            if entry.is_dir() {
                fs::create_dir_all(&output)
                    .map_err(|error| format!("could not create staged directory: {error}"))?;
                continue;
            }
            validate_file_limit(&relative, entry.size())?;
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| format!("could not create staged directory: {error}"))?;
            }
            let mut writer = File::create(&output)
                .map_err(|error| format!("could not create staged file: {error}"))?;
            let entry_size = entry.size();
            std::io::copy(
                &mut entry.by_ref().take(entry_size.saturating_add(1)),
                &mut writer,
            )
            .map_err(|error| format!("could not extract plugin file: {error}"))?;
            writer
                .flush()
                .map_err(|error| format!("could not flush staged file: {error}"))?;
        }
        parse_staged(
            source.to_path_buf(),
            staging_path.to_path_buf(),
            script_factory,
        )
    })();
    if result.is_err() {
        let boundary = staging_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| staging_path.to_path_buf());
        cleanup_staging(staging_path, &boundary);
    }
    result
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let mut pending = vec![(source.to_path_buf(), PathBuf::new())];
    let mut count = 0_usize;
    let mut total = 0_u64;
    let mut seen = HashSet::new();
    while let Some((directory, relative_directory)) = pending.pop() {
        for item in fs::read_dir(&directory)
            .map_err(|error| format!("could not read plugin directory: {error}"))?
        {
            let item = item.map_err(|error| format!("could not read plugin entry: {error}"))?;
            let name = item
                .file_name()
                .to_str()
                .ok_or("plugin paths must be valid Unicode")?
                .to_owned();
            let relative =
                validate_relative_path(&relative_directory.join(name).to_string_lossy())?;
            let metadata = fs::symlink_metadata(item.path())
                .map_err(|error| format!("could not inspect plugin entry: {error}"))?;
            if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
                return Err(format!(
                    "plugin entry '{}' is a link or reparse point",
                    relative.display()
                ));
            }
            let folded = relative.to_string_lossy().to_lowercase();
            if !seen.insert(folded) {
                return Err(format!(
                    "plugin contains a case-insensitive path collision at '{}'",
                    relative.display()
                ));
            }
            count += 1;
            if count > MAX_FILE_COUNT {
                return Err(format!(
                    "plugin contains more than {MAX_FILE_COUNT} entries"
                ));
            }
            let output = destination.join(&relative);
            if metadata.is_dir() {
                fs::create_dir_all(&output)
                    .map_err(|error| format!("could not create staged directory: {error}"))?;
                pending.push((item.path(), relative));
            } else if metadata.is_file() {
                total = total
                    .checked_add(metadata.len())
                    .ok_or("plugin size overflow")?;
                if total > MAX_EXTRACTED_BYTES {
                    return Err("plugin exceeds the 32 MiB extracted limit".into());
                }
                validate_file_limit(&relative, metadata.len())?;
                if let Some(parent) = output.parent() {
                    fs::create_dir_all(parent)
                        .map_err(|error| format!("could not create staged directory: {error}"))?;
                }
                fs::copy(item.path(), output)
                    .map_err(|error| format!("could not stage plugin file: {error}"))?;
            } else {
                return Err("plugin contains an unsupported filesystem entry".into());
            }
        }
    }
    Ok(())
}

fn parse_staged(
    source_path: PathBuf,
    staging_path: PathBuf,
    script_factory: &dyn ScriptPackageFactory,
) -> Result<PreparedPackage, String> {
    let manifest_path = staging_path.join("manifest.toml");
    let manifest = read_limited(&manifest_path, MAX_MANIFEST_BYTES)?;
    let manifest_value: toml::Value =
        toml::from_str(&manifest).map_err(|error| format!("invalid plugin manifest: {error}"))?;
    let kind = manifest_value
        .get("plugin")
        .and_then(|plugin| plugin.get("kind"))
        .and_then(toml::Value::as_str)
        .ok_or("plugin manifest is missing plugin.kind")?;
    let package = match kind {
        "declarative" => {
            let package = DeclarativePackage::parse_with_prompts(&manifest, |name| {
                let relative = validate_relative_path(name)?;
                if relative.extension().and_then(|value| value.to_str()) != Some("md") {
                    return Err("declarative prompt files must use the .md extension".into());
                }
                read_limited(&staging_path.join(relative), MAX_PROMPT_BYTES)
            })?;
            validate_package_files(&staging_path, PluginKind::Declarative)?;
            PreparedPackageKind::Declarative(package)
        }
        "script" => {
            validate_package_files(&staging_path, PluginKind::Script)?;
            PreparedPackageKind::Script(script_factory.inspect(&staging_path)?)
        }
        value => return Err(format!("unsupported plugin kind '{value}'")),
    };
    let package_hash = hash_tree(&staging_path)?;
    Ok(PreparedPackage {
        source_path,
        staging_path,
        package_hash,
        package,
    })
}

fn validate_package_files(root: &Path, kind: PluginKind) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let item = item.map_err(|error| error.to_string())?;
            let metadata = item.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                pending.push(item.path());
                continue;
            }
            let item_path = item.path();
            let relative = item_path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?;
            let allowed = relative == Path::new("manifest.toml")
                || relative == Path::new("icon.svg")
                || match kind {
                    PluginKind::Declarative => {
                        relative.extension().and_then(|value| value.to_str()) == Some("md")
                    }
                    PluginKind::Script => {
                        relative.extension().and_then(|value| value.to_str()) == Some("js")
                    }
                };
            if !allowed {
                return Err(format!(
                    "{} packages cannot contain '{}'",
                    kind.manifest_name(),
                    relative.display()
                ));
            }
        }
    }
    Ok(())
}

fn hash_tree(root: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for item in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let item = item.map_err(|error| error.to_string())?;
            let metadata = item.metadata().map_err(|error| error.to_string())?;
            if metadata.is_dir() {
                pending.push(item.path());
            } else if metadata.is_file() {
                files.push(item.path());
            }
        }
    }
    files.sort_by_key(|path| {
        path.strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_lowercase()
    });
    let mut hasher = Sha256::new();
    for path in files {
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        hasher.update(relative.to_string_lossy().replace('\\', "/").as_bytes());
        hasher.update([0]);
        let mut file = File::open(&path).map_err(|error| error.to_string())?;
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.update([0xff]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn validate_relative_path(value: &str) -> Result<PathBuf, String> {
    if value.is_empty()
        || value.starts_with(['/', '\\'])
        || value.contains(':')
        || value.contains('\\')
    {
        return Err(format!("unsafe plugin path '{value}'"));
    }
    let path = PathBuf::from(value);
    for component in path.components() {
        let Component::Normal(segment) = component else {
            return Err(format!("unsafe plugin path '{value}'"));
        };
        let name = segment.to_string_lossy();
        if name.ends_with(['.', ' ']) || is_reserved_windows_name(&name) {
            return Err(format!("unsafe Windows plugin path '{value}'"));
        }
    }
    Ok(path)
}

fn is_reserved_windows_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end_matches(['.', ' ']);
    matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn validate_file_limit(path: &Path, size: u64) -> Result<(), String> {
    let limit = match path.file_name().and_then(|value| value.to_str()) {
        Some("manifest.toml") => MAX_MANIFEST_BYTES,
        Some("icon.svg") => MAX_ICON_BYTES,
        _ if path.extension().and_then(|value| value.to_str()) == Some("md") => MAX_PROMPT_BYTES,
        _ if path.extension().and_then(|value| value.to_str()) == Some("js") => MAX_PROMPT_BYTES,
        _ => MAX_EXTRACTED_BYTES,
    };
    if size > limit {
        Err(format!(
            "plugin file '{}' exceeds its size limit",
            path.display()
        ))
    } else {
        Ok(())
    }
}

fn read_limited(path: &Path, limit: u64) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("could not read '{}': {error}", path.display()))?;
    if metadata.len() > limit {
        return Err(format!(
            "plugin file '{}' exceeds its size limit",
            path.display()
        ));
    }
    fs::read_to_string(path).map_err(|error| {
        format!(
            "plugin file '{}' is not valid UTF-8: {error}",
            path.display()
        )
    })
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;
    metadata.file_attributes() & 0x400 != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_: &fs::Metadata) -> bool {
    false
}

fn cleanup_staging(path: &Path, boundary: &Path) {
    if path.parent() == Some(boundary) {
        let _ = fs::remove_dir_all(path);
    }
}

fn cleanup_prepared(path: &Path) {
    if let Some(parent) = path.parent() {
        cleanup_staging(path, parent);
    }
}

fn cleanup_managed(path: &Path, plugins_root: &Path) {
    if path
        .parent()
        .and_then(Path::parent)
        .is_some_and(|parent| parent == plugins_root)
    {
        let _ = fs::remove_dir_all(path);
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use semver::Version;

    struct NoScriptFactory;

    impl ScriptPackageFactory for NoScriptFactory {
        fn inspect(&self, _: &Path) -> Result<ScriptPackageDefinition, String> {
            Err("script factory must not be reached by this fixture".into())
        }

        fn activate(
            &self,
            _: &Path,
            _: &ScriptPackageDefinition,
            _: Arc<dyn ScriptInvocationHost>,
            _: Arc<dyn SettingsUiPort>,
        ) -> Result<lexwisp_core::ScriptActivation, String> {
            Err("script factory must not be reached by this fixture".into())
        }

        fn shutdown(&self) {}
    }

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir()
            .join("lexwisp-stage6-tests")
            .join(format!("{label}-{}-{}", std::process::id(), Uuid::new_v4()))
    }

    #[test]
    fn rejects_escape_ads_and_reserved_paths() {
        for value in [
            "../manifest.toml",
            "C:/manifest.toml",
            "manifest.toml:evil",
            "CON/file",
        ] {
            assert!(validate_relative_path(value).is_err(), "{value}");
        }
    }

    #[test]
    fn example_package_matches_the_real_parser() {
        let manifest = include_str!("../../../examples/plugins/academic-polish/manifest.toml");
        let prompt = include_str!("../../../examples/plugins/academic-polish/prompt.md");
        let package = DeclarativePackage::parse(manifest, "prompt.md", prompt)
            .expect("documented example parses");
        assert_eq!(package.plugin.id().as_str(), "org.example.academic-polish");
        assert_eq!(package.version, Version::new(1, 0, 0));
    }

    #[test]
    fn damaged_manifest_and_unknown_fields_are_rejected() {
        assert!(DeclarativePackage::parse("not = [toml", "prompt.md", "prompt").is_err());
        let manifest = include_str!("../../../examples/plugins/academic-polish/manifest.toml")
            .replace(
                "name = \"Academic Polish\"",
                "name = \"Academic Polish\"\nunsafe_field = true",
            );
        assert!(DeclarativePackage::parse(&manifest, "prompt.md", "prompt").is_err());
    }

    #[test]
    fn stage6_rejects_script_payloads() {
        let root = test_root("script-payload");
        fs::create_dir_all(&root).expect("test root");
        fs::write(root.join("manifest.toml"), "fixture").expect("manifest");
        fs::write(root.join("main.js"), "export function run() {};").expect("script");
        assert!(validate_package_files(&root, PluginKind::Declarative).is_err());
        fs::remove_dir_all(&root).expect("bounded test root is removable");
    }

    #[test]
    fn oversized_zip_is_rejected_before_extraction() {
        let root = test_root("oversized-zip");
        fs::create_dir_all(&root).expect("test root");
        let archive = root.join("oversized.zip");
        File::create(&archive)
            .expect("archive fixture")
            .set_len(MAX_ARCHIVE_BYTES + 1)
            .expect("sparse archive size");
        assert!(prepare_zip(&archive, &root.join("staging"), &NoScriptFactory).is_err());
        assert!(!root.join("staging").exists());
        fs::remove_dir_all(&root).expect("bounded test root is removable");
    }
}

use std::{
    path::PathBuf,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use lexwisp_core::{
    ActionRequest, ActionUiPort, ClearHistoryMode, DiagnosticsSnapshot, HistoryDetail,
    HistoryError, HistoryFuture, HistoryPage, HistoryQuery, HistoryUiPort, InvocationId,
};
use lexwisp_storage::ContentStore;

use crate::{ExecutionStore, FavoriteService, SettingsService, TaskScope};

pub struct HistoryService {
    storage: ContentStore,
    executions: Arc<ExecutionStore>,
    favorites: Arc<FavoriteService>,
    actions: Arc<dyn ActionUiPort>,
    settings: Arc<SettingsService>,
    tasks: TaskScope,
    data_directory: PathBuf,
}

impl HistoryService {
    pub fn new(
        storage: ContentStore,
        executions: Arc<ExecutionStore>,
        favorites: Arc<FavoriteService>,
        actions: Arc<dyn ActionUiPort>,
        settings: Arc<SettingsService>,
        tasks: TaskScope,
        data_directory: PathBuf,
    ) -> Self {
        Self {
            storage,
            executions,
            favorites,
            actions,
            settings,
            tasks,
            data_directory,
        }
    }
}

impl HistoryUiPort for HistoryService {
    fn page(&self, query: HistoryQuery) -> HistoryFuture<'_, HistoryPage> {
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        Box::pin(async move {
            tasks
                .spawn_blocking(move || storage.query_history(query))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))
        })
    }

    fn detail(&self, invocation_id: InvocationId) -> HistoryFuture<'_, HistoryDetail> {
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        Box::pin(async move {
            tasks
                .spawn_blocking(move || storage.history_detail(invocation_id.as_str()))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .ok_or(HistoryError::NotFound)
        })
    }

    fn set_favorite(
        &self,
        invocation_id: InvocationId,
        favorite: bool,
        note: String,
    ) -> HistoryFuture<'_, ()> {
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        let favorites = self.favorites.clone();
        Box::pin(async move {
            let id = invocation_id.clone();
            tasks
                .spawn_blocking(move || storage.set_favorite(id.as_str(), favorite, &note))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?;
            favorites.set_known(invocation_id, favorite);
            Ok(())
        })
    }

    fn delete(&self, invocation_id: InvocationId) -> HistoryFuture<'_, ()> {
        self.executions.revoke_persistence(&invocation_id);
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        let favorites = self.favorites.clone();
        Box::pin(async move {
            let id = invocation_id.clone();
            tasks
                .spawn_blocking(move || storage.delete_history(id.as_str()))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?;
            favorites.set_known(invocation_id, false);
            Ok(())
        })
    }

    fn clear(&self, mode: ClearHistoryMode) -> HistoryFuture<'_, ()> {
        let generation = self.executions.advance_retention_generation();
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        let favorites = self.favorites.clone();
        Box::pin(async move {
            tasks
                .spawn_blocking(move || storage.clear_history(mode, generation))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?;
            if mode == ClearHistoryMode::IncludeFavorites {
                favorites.clear_known();
            }
            Ok(())
        })
    }

    fn retry(&self, invocation_id: InvocationId) -> HistoryFuture<'_, String> {
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        let actions = self.actions.clone();
        Box::pin(async move {
            let id = invocation_id.clone();
            let detail = tasks
                .spawn_blocking(move || storage.history_detail(id.as_str()))
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .ok_or(HistoryError::NotFound)?;
            actions
                .invoke(detail.item.action, ActionRequest::manual(detail.input))
                .await
                .map(|result| result.output)
                .map_err(|error| HistoryError::Retry(error.to_string()))
        })
    }

    fn create_backup(&self) -> HistoryFuture<'_, PathBuf> {
        let storage = self.storage.clone();
        let tasks = self.tasks.clone();
        let backups = self.data_directory.join("backups");
        Box::pin(async move {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .as_secs();
            let path = backups.join(format!("lexwisp-{stamp}.db"));
            let result_path = path.clone();
            tasks
                .spawn_blocking(move || {
                    storage.backup(path)?;
                    prune_database_backups(&backups, 5)
                        .map_err(|error| lexwisp_storage::StorageError::Sql(error.to_string()))
                })
                .await
                .map_err(|error| HistoryError::Storage(error.to_string()))?
                .map_err(|error| HistoryError::Storage(error.to_string()))?;
            Ok(result_path)
        })
    }

    fn diagnostics(&self) -> DiagnosticsSnapshot {
        DiagnosticsSnapshot {
            data_directory: self.data_directory.clone(),
            log_directory: self.data_directory.join("logs"),
            settings_path: self.settings.config_path().to_path_buf(),
            database_path: self.data_directory.join("lexwisp.db"),
            recording_enabled: self.executions.recording_enabled(),
            retention_generation: self.executions.retention_generation(),
            active_executions: self.executions.active_count(),
        }
    }
}

fn prune_database_backups(directory: &std::path::Path, keep: usize) -> std::io::Result<()> {
    let mut backups = std::fs::read_dir(directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("lexwisp-") && name.ends_with(".db"))
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    backups.sort();
    let remove_count = backups.len().saturating_sub(keep);
    for path in backups.into_iter().take(remove_count) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

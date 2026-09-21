use std::{future::Future, path::PathBuf, pin::Pin};

use thiserror::Error;

use crate::{ExecutionStatus, InvocationId};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryQuery {
    pub search: String,
    pub status: Option<ExecutionStatus>,
    pub favorites_only: bool,
    pub cursor: Option<HistoryCursor>,
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryCursor {
    pub updated_at_ms: i64,
    pub invocation_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryItem {
    pub invocation_id: InvocationId,
    pub title: String,
    pub preview: String,
    pub status: ExecutionStatus,
    pub provider_id: String,
    pub model_id: String,
    pub updated_at_ms: i64,
    pub favorite: bool,
    pub favorite_note: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    pub next_cursor: Option<HistoryCursor>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryDetail {
    pub item: HistoryItem,
    pub input: String,
    pub output: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearHistoryMode {
    PreserveFavorites,
    IncludeFavorites,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticsSnapshot {
    pub data_directory: PathBuf,
    pub log_directory: PathBuf,
    pub settings_path: PathBuf,
    pub database_path: PathBuf,
    pub recording_enabled: bool,
    pub retention_generation: u64,
    pub active_executions: usize,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum HistoryError {
    #[error("history request is invalid: {0}")]
    Invalid(String),
    #[error("history item was not found")]
    NotFound,
    #[error("history operation failed: {0}")]
    Storage(String),
    #[error("history retry failed: {0}")]
    Retry(String),
}

pub type HistoryFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, HistoryError>> + Send + 'a>>;

pub trait HistoryUiPort: Send + Sync {
    fn page(&self, query: HistoryQuery) -> HistoryFuture<'_, HistoryPage>;
    fn detail(&self, invocation_id: InvocationId) -> HistoryFuture<'_, HistoryDetail>;
    fn set_favorite(
        &self,
        invocation_id: InvocationId,
        favorite: bool,
        note: String,
    ) -> HistoryFuture<'_, ()>;
    fn delete(&self, invocation_id: InvocationId) -> HistoryFuture<'_, ()>;
    fn clear(&self, mode: ClearHistoryMode) -> HistoryFuture<'_, ()>;
    fn create_backup(&self) -> HistoryFuture<'_, PathBuf>;
    fn diagnostics(&self) -> DiagnosticsSnapshot;
}

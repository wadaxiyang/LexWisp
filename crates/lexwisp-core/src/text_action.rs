use std::{future::Future, pin::Pin};

use async_channel::Receiver;
use thiserror::Error;

use crate::{ExecutionStatus, InvocationId, QualifiedActionId, StorageState};

#[derive(Clone, Debug)]
pub struct TextActionSnapshot {
    pub action: QualifiedActionId,
    pub input: String,
    pub output: String,
    pub status: ExecutionStatus,
    pub invocation_id: Option<InvocationId>,
    pub active_invocation: Option<InvocationId>,
    pub generation: u64,
    pub status_text: String,
    pub storage: StorageState,
}

impl TextActionSnapshot {
    pub const fn can_stop(&self) -> bool {
        self.active_invocation.is_some()
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum TextActionError {
    #[error("the action is already running")]
    Busy,
    #[error("there is no active action")]
    NotRunning,
    #[error("text action failed: {0}")]
    Failed(String),
}

pub type TextActionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), TextActionError>> + Send + 'a>>;

pub trait TextActionUiPort: Send + Sync {
    fn action(&self) -> QualifiedActionId;
    fn snapshot(&self) -> TextActionSnapshot;
    fn subscribe(&self, capacity: usize) -> Receiver<TextActionSnapshot>;
    fn set_surface_visible(&self, visible: bool);
    fn stop(&self) -> Result<(), TextActionError>;
}

pub type FavoriteFuture<'a> = Pin<Box<dyn Future<Output = Result<bool, String>> + Send + 'a>>;

pub trait FavoriteUiPort: Send + Sync {
    fn toggle(&self, invocation_id: InvocationId) -> FavoriteFuture<'_>;
    fn contains(&self, invocation_id: &InvocationId) -> bool;
}

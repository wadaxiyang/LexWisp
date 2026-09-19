use std::{future::Future, pin::Pin};

use async_channel::Receiver;
use thiserror::Error;

use crate::{ConversationId, InvocationId, MessageId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatMessageStatus {
    Submitted,
    Generating,
    Completed,
    CancelledPartial,
    FailedPartial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessageSnapshot {
    pub id: MessageId,
    pub is_user: bool,
    pub content: String,
    pub status: ChatMessageStatus,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSnapshot {
    pub conversation_id: ConversationId,
    pub title: String,
    pub messages: Vec<ChatMessageSnapshot>,
    pub active_invocation: Option<InvocationId>,
    pub generation: u64,
    pub status_text: String,
    pub can_send: bool,
    pub can_stop: bool,
    pub can_retry: bool,
    pub has_unsaved_result: bool,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChatError {
    #[error("enter a message before sending")]
    EmptyInput,
    #[error("a response is already being generated")]
    Busy,
    #[error("there is no response to retry")]
    NothingToRetry,
    #[error("chat action failed: {0}")]
    Failed(String),
}

pub type ChatUiResultFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ChatError>> + Send + 'a>>;

pub trait ChatUiPort: Send + Sync {
    fn snapshot(&self) -> ChatSnapshot;
    fn subscribe(&self, capacity: usize) -> Receiver<ChatSnapshot>;
    fn set_surface_visible(&self, visible: bool);
    fn stop(&self) -> Result<(), ChatError>;
    fn retry(&self) -> ChatUiResultFuture<'_>;
}

use std::{future::Future, pin::Pin, sync::Arc};

use thiserror::Error;

use crate::{
    AttemptId, ChatAttachment, ChatModelPreference, ConversationId, ExecutionObserver,
    InvocationId, MessageId,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AiRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: String,
    pub attachments: Vec<ChatAttachment>,
}

#[derive(Clone, Debug)]
pub struct ChatInvocationRequest {
    pub conversation_id: ConversationId,
    pub user_message_id: MessageId,
    pub assistant_message_id: MessageId,
    pub attempt_id: AttemptId,
    pub reply_to_user_id: MessageId,
    pub conversation_title: String,
    pub model_preference: ChatModelPreference,
    pub user_ordinal: u64,
    pub assistant_ordinal: u64,
    pub input: String,
    pub messages: Vec<AiMessage>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChatRunError {
    #[error("chat is already generating in this conversation")]
    ConversationBusy,
    #[error("host is shutting down")]
    ShuttingDown,
    #[error("chat request failed: {0}")]
    Failed(String),
}

pub type ChatRunFuture<'a> =
    Pin<Box<dyn Future<Output = Result<crate::ExecutionSnapshot, ChatRunError>> + Send + 'a>>;

pub trait ChatRunPort: Send + Sync {
    fn run(
        &self,
        request: ChatInvocationRequest,
        observer: Arc<dyn ExecutionObserver>,
    ) -> ChatRunFuture<'_>;

    fn cancel(&self, invocation_id: &InvocationId) -> Result<(), ChatRunError>;

    fn context_budget(&self, preference: &ChatModelPreference) -> usize;
}

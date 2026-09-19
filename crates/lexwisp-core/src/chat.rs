use std::{future::Future, pin::Pin, sync::Arc};

use async_channel::Receiver;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{AttemptId, ConversationId, InvocationId, MessageId, SurfaceKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatMessageStatus {
    Submitted,
    Generating,
    Completed,
    CancelledPartial,
    FailedPartial,
}

impl ChatMessageStatus {
    pub const fn persistence_name(self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::CancelledPartial => "cancelled",
            Self::FailedPartial => "failed",
        }
    }

    pub fn from_persistence_name(value: &str) -> Option<Self> {
        match value {
            "submitted" => Some(Self::Submitted),
            "generating" => Some(Self::Generating),
            "completed" => Some(Self::Completed),
            "cancelled" => Some(Self::CancelledPartial),
            "failed" | "interrupted" => Some(Self::FailedPartial),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "model", rename_all = "kebab-case")]
pub enum ChatModelPreference {
    #[default]
    Fast,
    Smart,
    Model(String),
}

impl ChatModelPreference {
    pub fn persistence_name(&self) -> String {
        match self {
            Self::Fast => "profile:fast".into(),
            Self::Smart => "profile:smart".into(),
            Self::Model(model) => format!("model:{model}"),
        }
    }

    pub fn from_persistence_name(value: &str) -> Self {
        match value {
            "profile:smart" => Self::Smart,
            "profile:fast" => Self::Fast,
            value => value
                .strip_prefix("model:")
                .filter(|model| !model.is_empty())
                .map(|model| Self::Model(model.to_owned()))
                .unwrap_or(Self::Fast),
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            Self::Fast => "Fast",
            Self::Smart => "Smart",
            Self::Model(model) => model,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatMessageSnapshot {
    pub id: MessageId,
    pub is_user: bool,
    pub content: String,
    pub status: ChatMessageStatus,
    pub ordinal: u64,
    pub attempt_id: Option<AttemptId>,
    pub reply_to_user_id: Option<MessageId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatConversationSummary {
    id: ConversationId,
    title: String,
    model_preference: ChatModelPreference,
    is_generating: bool,
    updated_order: u64,
}

impl ChatConversationSummary {
    pub fn new(
        id: ConversationId,
        title: impl Into<String>,
        model_preference: ChatModelPreference,
        is_generating: bool,
        updated_order: u64,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            model_preference,
            is_generating,
            updated_order,
        }
    }

    pub const fn id(&self) -> &ConversationId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn model_preference(&self) -> &ChatModelPreference {
        &self.model_preference
    }

    pub const fn is_generating(&self) -> bool {
        self.is_generating
    }

    pub const fn updated_order(&self) -> u64 {
        self.updated_order
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChatSnapshot {
    pub conversation_id: ConversationId,
    pub title: String,
    pub messages: Arc<Vec<Arc<ChatMessageSnapshot>>>,
    pub conversations: Arc<Vec<ChatConversationSummary>>,
    pub model_preference: ChatModelPreference,
    pub active_invocation: Option<InvocationId>,
    pub generation: u64,
    pub status_text: String,
    pub can_send: bool,
    pub can_stop: bool,
    pub can_retry: bool,
    pub has_unsaved_result: bool,
    pub context_notice: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersistedChatConversation {
    id: ConversationId,
    title: String,
    model_preference: ChatModelPreference,
    messages: Vec<ChatMessageSnapshot>,
    updated_order: u64,
}

impl PersistedChatConversation {
    pub fn new(
        id: ConversationId,
        title: impl Into<String>,
        model_preference: ChatModelPreference,
        messages: Vec<ChatMessageSnapshot>,
        updated_order: u64,
    ) -> Self {
        Self {
            id,
            title: title.into(),
            model_preference,
            messages,
            updated_order,
        }
    }

    pub const fn id(&self) -> &ConversationId {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub const fn model_preference(&self) -> &ChatModelPreference {
        &self.model_preference
    }

    pub fn messages(&self) -> &[ChatMessageSnapshot] {
        &self.messages
    }

    pub const fn updated_order(&self) -> u64 {
        self.updated_order
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ChatError {
    #[error("enter a message before sending")]
    EmptyInput,
    #[error("a response is already being generated")]
    Busy,
    #[error("there is no response to retry")]
    NothingToRetry,
    #[error("conversation was not found")]
    ConversationNotFound,
    #[error("the current message does not fit the estimated context budget")]
    ContextBudgetExceeded,
    #[error("chat action failed: {0}")]
    Failed(String),
}

pub type ChatUiResultFuture<'a> = Pin<Box<dyn Future<Output = Result<(), ChatError>> + Send + 'a>>;

pub trait ChatHistoryPort: Send + Sync {
    fn restore(&self) -> Result<Vec<PersistedChatConversation>, ChatError>;
    fn save_conversation(
        &self,
        conversation_id: &ConversationId,
        title: &str,
        model_preference: &ChatModelPreference,
    ) -> Result<(), ChatError>;
    fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError>;
}

pub trait ChatUiPort: Send + Sync {
    fn snapshot(&self) -> ChatSnapshot;
    fn subscribe(&self, capacity: usize) -> Receiver<ChatSnapshot>;
    fn set_surface_visible(&self, surface: SurfaceKind, visible: bool);
    fn create_conversation(&self) -> Result<ConversationId, ChatError>;
    fn rename_conversation(&self, title: String) -> Result<(), ChatError>;
    fn switch_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError>;
    fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError>;
    fn set_model_preference(&self, preference: ChatModelPreference) -> Result<(), ChatError>;
    fn send(&self, input: String) -> ChatUiResultFuture<'_>;
    fn stop(&self) -> Result<(), ChatError>;
    fn retry(&self) -> ChatUiResultFuture<'_>;
}

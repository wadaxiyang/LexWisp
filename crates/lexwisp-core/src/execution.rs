use serde::{Deserialize, Serialize};

use crate::{
    AttemptId, ConversationId, InvocationId, MessageId, PluginId, ProviderId, QualifiedActionId,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionStatus {
    Queued,
    Running,
    Cancelling,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl ExecutionStatus {
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Interrupted
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageState {
    Pending,
    Saved,
    Unsaved,
    NotRecorded,
}

#[derive(Clone, Debug)]
pub struct ExecutionSnapshot {
    pub invocation_id: InvocationId,
    pub plugin_id: PluginId,
    pub action: QualifiedActionId,
    pub plugin_generation: u64,
    pub conversation_id: Option<ConversationId>,
    pub user_message_id: Option<MessageId>,
    pub assistant_message_id: Option<MessageId>,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub sequence: u64,
    pub text_version: u64,
    pub status: ExecutionStatus,
    pub output: String,
    pub error: Option<String>,
    pub storage: StorageState,
    pub retention_generation: u64,
}

pub trait ExecutionObserver: Send + Sync {
    fn on_execution(&self, snapshot: ExecutionSnapshot);
}

#[derive(Clone, Debug)]
pub struct ExecutionCheckpoint {
    pub snapshot: ExecutionSnapshot,
    pub input: String,
    pub chat: Option<ChatCheckpoint>,
}

#[derive(Clone, Debug)]
pub struct ChatCheckpoint {
    pub conversation_title: String,
    pub model_preference: crate::ChatModelPreference,
    pub user_ordinal: u64,
    pub assistant_ordinal: u64,
    pub attempt_id: AttemptId,
    pub reply_to_user_id: MessageId,
}

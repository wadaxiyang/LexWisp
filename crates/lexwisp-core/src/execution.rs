use serde::{Deserialize, Serialize};

use crate::{ConversationId, InvocationId, MessageId, PluginId, ProviderId};

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
}

#[derive(Clone, Debug)]
pub struct ExecutionSnapshot {
    pub invocation_id: InvocationId,
    pub plugin_id: PluginId,
    pub plugin_generation: u64,
    pub conversation_id: ConversationId,
    pub user_message_id: MessageId,
    pub assistant_message_id: MessageId,
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
    pub conversation_title: String,
    pub user_ordinal: u64,
    pub assistant_ordinal: u64,
}

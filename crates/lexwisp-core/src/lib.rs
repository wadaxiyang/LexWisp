mod ai;
mod chat;
mod command;
mod execution;
mod file;
mod history;
mod ids;
mod provider;
mod settings;
mod task;

pub use ai::{AiMessage, AiRole, ChatInvocationRequest, ChatRunError, ChatRunFuture, ChatRunPort};
pub use chat::{
    ChatAttachment, ChatAttachmentContent, ChatConversationSummary, ChatDraft, ChatError,
    ChatHistoryPort, ChatMessageSnapshot, ChatMessageStatus, ChatModelPreference, ChatSnapshot,
    ChatUiPort, ChatUiResultFuture, PersistedChatConversation,
};
pub use command::{HostUiCommand, SurfaceKind};
pub use execution::{
    ChatCheckpoint, ExecutionCheckpoint, ExecutionObserver, ExecutionSnapshot, ExecutionStatus,
    StorageState,
};
pub use file::{AtomicFileWriter, FileWriteError};
pub use history::{
    ClearHistoryMode, DiagnosticsSnapshot, HistoryCursor, HistoryDetail, HistoryError,
    HistoryFuture, HistoryItem, HistoryPage, HistoryQuery, HistoryUiPort,
};
pub use ids::{AttemptId, ConversationId, InvocationId, MessageId};
pub use provider::{
    CredentialError, CredentialStore, ModelProfile, ProviderConfig, ProviderDraft, ProviderError,
    ProviderId, ProviderTestResult, ProviderUiFuture, ProviderUiPort, ProviderUiSnapshot,
};
pub use settings::{
    AppSettings, GlobalHotkey, SettingsError, SettingsFuture, SettingsSnapshot, SettingsUiPort,
    ThemePreference,
};
pub use task::TaskOwner;

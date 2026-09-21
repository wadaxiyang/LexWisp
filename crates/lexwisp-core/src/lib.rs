mod ai;
mod chat;
mod command;
mod context;
mod execution;
mod file;
mod history;
mod ids;
mod plugin;
mod provider;
mod script;
mod settings;
mod task;
mod text_action;

pub use ai::{
    AiMessage, AiRole, ChatInvocationRequest, ChatRunError, ChatRunFuture, ChatRunPort,
    TextInvocationRequest, TextRunFuture, TextRunPort,
};
pub use chat::{
    ChatAttachment, ChatAttachmentContent, ChatConversationSummary, ChatDraft, ChatError,
    ChatHistoryPort, ChatMessageSnapshot, ChatMessageStatus, ChatModelPreference, ChatSnapshot,
    ChatUiPort, ChatUiResultFuture, PersistedChatConversation,
};
pub use command::{HostUiCommand, ShellPresentation, SurfaceKind};
pub use context::{
    CaptureStatus, ContextError, ContextFuture, ContextSnapshot, ContextUiPort, InputSource,
    ReplaceOutcome,
};
pub use execution::{
    ChatCheckpoint, ExecutionCheckpoint, ExecutionObserver, ExecutionSnapshot, ExecutionStatus,
    StorageState,
};
pub use file::{AtomicFileWriter, FileWriteError};
pub use history::{
    ClearHistoryMode, DiagnosticsSnapshot, HistoryCursor, HistoryDetail, HistoryError,
    HistoryFuture, HistoryItem, HistoryPage, HistoryQuery, HistoryUiPort,
};
pub use ids::{AttemptId, ContextToken, ConversationId, InvocationId, MessageId};
pub use plugin::{
    ActionDescriptor, ActionError, ActionFuture, ActionHandler, ActionId, ActionInputSource,
    ActionKind, ActionOutputPolicy, ActionParameter, ActionRequest, ActionResult, ActionUiPort,
    Capability, DeclarativeActionDefinition, ManagedActionSnapshot, ManagedPluginStatus,
    ManagedPluginSummary, ParameterKind, PluginDescriptor, PluginId, PluginImportPreview,
    PluginKind, PluginManagementFuture, PluginManagementUiPort, QualifiedActionId,
    ScriptActionDefinition,
};
pub use provider::{
    CredentialError, CredentialStore, ModelProfile, ProviderConfig, ProviderDraft, ProviderError,
    ProviderId, ProviderTestResult, ProviderUiFuture, ProviderUiPort, ProviderUiSnapshot,
};
pub use script::{
    ScriptActivation, ScriptFuture, ScriptHostCall, ScriptHttpRequest, ScriptInvocation,
    ScriptInvocationHost, ScriptNetworkRule, ScriptPackageDefinition, ScriptPackageFactory,
    ScriptPluginLifecycle,
};
pub use settings::{
    AppSettings, DismissPolicy, GlobalHotkey, LaunchMode, LaunchRoute, SettingsError,
    SettingsFuture, SettingsSnapshot, SettingsUiPort, ThemePreference,
};
pub use task::TaskOwner;
pub use text_action::{
    FavoriteFuture, FavoriteUiPort, TextActionError, TextActionFuture, TextActionSnapshot,
    TextActionUiPort,
};

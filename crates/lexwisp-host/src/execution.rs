use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use lexwisp_core::{
    ChatCheckpoint, ConversationId, ExecutionCheckpoint, ExecutionObserver, ExecutionSnapshot,
    ExecutionStatus, InvocationId, MessageId, PluginId, ProviderId, QualifiedActionId,
    StorageState,
};
use lexwisp_storage::ContentStore;

const UI_FLUSH_INTERVAL: Duration = Duration::from_millis(33);
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const CHECKPOINT_BYTES: usize = 16 * 1024;

pub(crate) struct ExecutionStart {
    pub invocation_id: InvocationId,
    pub plugin_id: PluginId,
    pub action: QualifiedActionId,
    pub plugin_generation: u64,
    pub conversation_id: Option<ConversationId>,
    pub user_message_id: Option<MessageId>,
    pub assistant_message_id: Option<MessageId>,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub input: String,
    pub chat: Option<ChatCheckpoint>,
    pub observer: Arc<dyn ExecutionObserver>,
}

struct ExecutionAccumulator {
    snapshot: ExecutionSnapshot,
    input: String,
    chat: Option<ChatCheckpoint>,
    observer: Arc<dyn ExecutionObserver>,
    dirty_bytes: usize,
    last_ui_flush: Instant,
    last_checkpoint: Instant,
}

impl ExecutionAccumulator {
    fn checkpoint(&self) -> ExecutionCheckpoint {
        ExecutionCheckpoint {
            snapshot: self.snapshot.clone(),
            input: self.input.clone(),
            chat: self.chat.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ExecutionStore {
    entries: Arc<Mutex<HashMap<InvocationId, ExecutionAccumulator>>>,
    storage: ContentStore,
}

impl ExecutionStore {
    pub fn new(storage: ContentStore) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            storage,
        }
    }

    pub(crate) fn start(&self, start: ExecutionStart) -> ExecutionSnapshot {
        let now = Instant::now();
        let snapshot = ExecutionSnapshot {
            invocation_id: start.invocation_id.clone(),
            plugin_id: start.plugin_id,
            action: start.action,
            plugin_generation: start.plugin_generation,
            conversation_id: start.conversation_id,
            user_message_id: start.user_message_id,
            assistant_message_id: start.assistant_message_id,
            provider_id: start.provider_id,
            model_id: start.model_id,
            sequence: 1,
            text_version: 0,
            status: ExecutionStatus::Running,
            output: String::new(),
            error: None,
            storage: StorageState::Pending,
            retention_generation: 0,
        };
        let accumulator = ExecutionAccumulator {
            snapshot: snapshot.clone(),
            input: start.input,
            chat: start.chat,
            observer: start.observer.clone(),
            dirty_bytes: 0,
            last_ui_flush: now.checked_sub(UI_FLUSH_INTERVAL).unwrap_or(now),
            last_checkpoint: now,
        };
        let checkpoint = accumulator.checkpoint();
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(snapshot.invocation_id.clone(), accumulator);
        start.observer.on_execution(snapshot.clone());
        if self.storage.enqueue(checkpoint, false).is_err() {
            self.mark_unsaved(&snapshot.invocation_id);
        }
        snapshot
    }

    pub(crate) fn append_text(
        &self,
        invocation_id: &InvocationId,
        plugin_generation: u64,
        delta: &str,
    ) -> Result<(), String> {
        if delta.is_empty() {
            return Ok(());
        }
        let now = Instant::now();
        let mut notify = None;
        let mut checkpoint = None;
        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = entries
                .get_mut(invocation_id)
                .ok_or_else(|| "execution is no longer active".to_string())?;
            if entry.snapshot.plugin_generation != plugin_generation {
                return Err("stale plugin generation".into());
            }
            if entry.snapshot.status.is_terminal() {
                return Err("execution already reached a terminal state".into());
            }
            entry.snapshot.output.push_str(delta);
            entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
            entry.snapshot.text_version = entry.snapshot.text_version.saturating_add(1);
            entry.dirty_bytes = entry.dirty_bytes.saturating_add(delta.len());
            if now.duration_since(entry.last_ui_flush) >= UI_FLUSH_INTERVAL {
                entry.last_ui_flush = now;
                notify = Some((entry.observer.clone(), entry.snapshot.clone()));
            }
            if now.duration_since(entry.last_checkpoint) >= CHECKPOINT_INTERVAL
                || entry.dirty_bytes >= CHECKPOINT_BYTES
            {
                entry.last_checkpoint = now;
                entry.dirty_bytes = 0;
                checkpoint = Some(entry.checkpoint());
            }
        }
        if let Some((observer, snapshot)) = notify {
            observer.on_execution(snapshot);
        }
        if let Some(checkpoint) = checkpoint
            && self.storage.enqueue(checkpoint, false).is_err()
        {
            self.mark_unsaved(invocation_id);
        }
        Ok(())
    }

    pub(crate) fn mark_cancelling(&self, invocation_id: &InvocationId) {
        let notification = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.get_mut(invocation_id).and_then(|entry| {
                (!entry.snapshot.status.is_terminal()).then(|| {
                    entry.snapshot.status = ExecutionStatus::Cancelling;
                    entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                    (entry.observer.clone(), entry.snapshot.clone())
                })
            })
        };
        if let Some((observer, snapshot)) = notification {
            observer.on_execution(snapshot);
        }
    }

    pub(crate) async fn commit_terminal(
        &self,
        invocation_id: &InvocationId,
        status: ExecutionStatus,
        error: Option<String>,
    ) -> Result<ExecutionSnapshot, String> {
        if !status.is_terminal() {
            return Err("terminal commit requires a terminal status".into());
        }
        let (observer, pending, checkpoint) = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = entries
                .get_mut(invocation_id)
                .ok_or_else(|| "execution is no longer active".to_string())?;
            if entry.snapshot.status.is_terminal() {
                return Ok(entry.snapshot.clone());
            }
            entry.snapshot.status = status;
            entry.snapshot.error = error;
            entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
            entry.snapshot.storage = StorageState::Pending;
            (
                entry.observer.clone(),
                entry.snapshot.clone(),
                entry.checkpoint(),
            )
        };
        observer.on_execution(pending);

        let storage = self.storage.clone();
        let persisted = tokio::task::spawn_blocking(move || {
            storage
                .enqueue(checkpoint, true)?
                .ok_or(lexwisp_storage::StorageError::Closed)?
                .wait()
        })
        .await
        .map_err(|error| error.to_string())
        .and_then(|result| result.map_err(|error| error.to_string()));

        let (observer, final_snapshot) = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = entries
                .get_mut(invocation_id)
                .ok_or_else(|| "execution disappeared during terminal persistence".to_string())?;
            entry.snapshot.storage = if persisted.is_ok() {
                StorageState::Saved
            } else {
                StorageState::Unsaved
            };
            entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
            (entry.observer.clone(), entry.snapshot.clone())
        };
        observer.on_execution(final_snapshot.clone());
        Ok(final_snapshot)
    }

    pub fn snapshot(&self, invocation_id: &InvocationId) -> Option<ExecutionSnapshot> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(invocation_id)
            .map(|entry| entry.snapshot.clone())
    }

    fn mark_unsaved(&self, invocation_id: &InvocationId) {
        let notification = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.get_mut(invocation_id).map(|entry| {
                entry.snapshot.storage = StorageState::Unsaved;
                entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                (entry.observer.clone(), entry.snapshot.clone())
            })
        };
        if let Some((observer, snapshot)) = notification {
            observer.on_execution(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use lexwisp_core::{ActionId, QualifiedActionId};
    use lexwisp_storage::ContentStoreOwner;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Sink;
    impl ExecutionObserver for Sink {
        fn on_execution(&self, _: ExecutionSnapshot) {}
    }

    fn fixture() -> (ContentStoreOwner, ExecutionStore, InvocationId) {
        let path: PathBuf = std::env::temp_dir()
            .join("lexwisp-stage2-execution")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("lexwisp.db");
        let (owner, content) = ContentStoreOwner::start(path).expect("content store starts");
        let store = ExecutionStore::new(content);
        let invocation_id = InvocationId::new();
        let plugin = PluginId::parse("org.lexwisp.chat").expect("plugin ID");
        let action =
            QualifiedActionId::new(plugin.clone(), ActionId::parse("ask").expect("action ID"));
        store.start(ExecutionStart {
            invocation_id: invocation_id.clone(),
            plugin_id: plugin,
            action,
            plugin_generation: 1,
            conversation_id: Some(ConversationId::new()),
            user_message_id: Some(MessageId::new()),
            assistant_message_id: Some(MessageId::new()),
            provider_id: ProviderId::parse("fixture").expect("provider ID"),
            model_id: "fixture".into(),
            input: "hello".into(),
            chat: Some(ChatCheckpoint {
                conversation_title: "hello".into(),
                model_preference: lexwisp_core::ChatModelPreference::Fast,
                user_ordinal: 0,
                assistant_ordinal: 1,
                attempt_id: lexwisp_core::AttemptId::new(),
                reply_to_user_id: MessageId::new(),
            }),
            observer: Arc::new(Sink),
        });
        (owner, store, invocation_id)
    }

    #[test]
    fn many_small_deltas_are_coalesced_for_storage() {
        let (owner, store, invocation_id) = fixture();
        for _ in 0..1_000 {
            store
                .append_text(&invocation_id, 1, "x")
                .expect("delta applies");
        }
        assert!(store.storage.enqueued_checkpoints() < 20);
        owner.shutdown();
    }
}

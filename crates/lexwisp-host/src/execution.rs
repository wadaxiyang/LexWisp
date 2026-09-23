use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use lexwisp_core::{
    ChatCheckpoint, ConversationId, ExecutionCheckpoint, ExecutionObserver, ExecutionSnapshot,
    ExecutionStatus, InvocationId, MessageId, ProviderId, StorageState,
};
use lexwisp_storage::ContentStore;

const UI_FLUSH_INTERVAL: Duration = Duration::from_millis(33);
const CHECKPOINT_INTERVAL: Duration = Duration::from_millis(500);
const CHECKPOINT_BYTES: usize = 16 * 1024;
const RECENT_TERMINAL_LIMIT: usize = 128;
const RECENT_TERMINAL_TTL: Duration = Duration::from_secs(60 * 60);

pub(crate) struct ExecutionStart {
    pub invocation_id: InvocationId,
    pub conversation_id: ConversationId,
    pub user_message_id: MessageId,
    pub assistant_message_id: MessageId,
    pub provider_id: ProviderId,
    pub model_id: String,
    pub input: String,
    pub chat: ChatCheckpoint,
    pub observer: Arc<dyn ExecutionObserver>,
}

struct ExecutionAccumulator {
    snapshot: ExecutionSnapshot,
    input: String,
    chat: ChatCheckpoint,
    observer: Arc<dyn ExecutionObserver>,
    dirty_bytes: usize,
    last_ui_flush: Instant,
    last_checkpoint: Instant,
    finished_at: Option<Instant>,
    persist: bool,
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
    recording_enabled: Arc<AtomicBool>,
    retention_generation: Arc<AtomicU64>,
}

impl ExecutionStore {
    pub fn new(storage: ContentStore, recording_enabled: bool, retention_generation: u64) -> Self {
        Self {
            entries: Arc::new(Mutex::new(HashMap::new())),
            storage,
            recording_enabled: Arc::new(AtomicBool::new(recording_enabled)),
            retention_generation: Arc::new(AtomicU64::new(retention_generation)),
        }
    }

    pub fn recording_enabled(&self) -> bool {
        self.recording_enabled.load(Ordering::Acquire)
    }

    pub fn retention_generation(&self) -> u64 {
        self.retention_generation.load(Ordering::Acquire)
    }

    pub fn set_recording_enabled(&self, enabled: bool) {
        self.recording_enabled.store(enabled, Ordering::Release);
        if !enabled {
            self.advance_retention_generation();
        }
    }

    pub fn advance_retention_generation(&self) -> u64 {
        let generation = self
            .retention_generation
            .fetch_add(1, Ordering::AcqRel)
            .saturating_add(1);
        let _ = self.storage.set_retention_generation(generation);
        let notifications = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries
                .values_mut()
                .filter(|entry| !entry.snapshot.status.is_terminal())
                .map(|entry| {
                    entry.persist = false;
                    entry.snapshot.storage = StorageState::NotRecorded;
                    entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                    (entry.observer.clone(), entry.snapshot.clone())
                })
                .collect::<Vec<_>>()
        };
        for (observer, snapshot) in notifications {
            observer.on_execution(snapshot);
        }
        generation
    }

    pub fn active_count(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .values()
            .filter(|entry| !entry.snapshot.status.is_terminal())
            .count()
    }

    pub fn revoke_persistence(&self, invocation_id: &InvocationId) {
        let notification = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            entries.get_mut(invocation_id).map(|entry| {
                entry.persist = false;
                entry.snapshot.storage = StorageState::NotRecorded;
                entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                (entry.observer.clone(), entry.snapshot.clone())
            })
        };
        if let Some((observer, snapshot)) = notification {
            observer.on_execution(snapshot);
        }
    }

    pub(crate) fn start(&self, start: ExecutionStart) -> ExecutionSnapshot {
        let now = Instant::now();
        let snapshot = ExecutionSnapshot {
            invocation_id: start.invocation_id.clone(),
            conversation_id: start.conversation_id,
            user_message_id: start.user_message_id,
            assistant_message_id: start.assistant_message_id,
            provider_id: start.provider_id,
            model_id: start.model_id,
            sequence: 1,
            text_version: 0,
            status: ExecutionStatus::Queued,
            output: String::new(),
            error: None,
            storage: if self.recording_enabled() {
                StorageState::Pending
            } else {
                StorageState::NotRecorded
            },
            retention_generation: self.retention_generation(),
        };
        let accumulator = ExecutionAccumulator {
            snapshot: snapshot.clone(),
            input: start.input,
            chat: start.chat,
            observer: start.observer.clone(),
            dirty_bytes: 0,
            last_ui_flush: now.checked_sub(UI_FLUSH_INTERVAL).unwrap_or(now),
            last_checkpoint: now,
            finished_at: None,
            persist: self.recording_enabled(),
        };
        let checkpoint = accumulator.checkpoint();
        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::prune_terminal_entries_locked(&mut entries, now, None);
            entries.insert(snapshot.invocation_id.clone(), accumulator);
        }
        start.observer.on_execution(snapshot.clone());
        if self.recording_enabled() && self.storage.enqueue(checkpoint, false).is_err() {
            self.mark_unsaved(&snapshot.invocation_id);
        }
        snapshot
    }

    pub(crate) fn mark_running(&self, invocation_id: &InvocationId) -> Result<(), String> {
        let notification = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = entries
                .get_mut(invocation_id)
                .ok_or_else(|| "execution is no longer active".to_string())?;
            if entry.snapshot.status.is_terminal() {
                return Err("execution already reached a terminal state".into());
            }
            if entry.snapshot.status != ExecutionStatus::Queued {
                return Err("execution is not queued".into());
            }
            entry.snapshot.status = ExecutionStatus::Running;
            entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
            (entry.observer.clone(), entry.snapshot.clone())
        };
        notification.0.on_execution(notification.1);
        Ok(())
    }

    pub(crate) fn append_text(
        &self,
        invocation_id: &InvocationId,
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
                if entry.persist {
                    checkpoint = Some(entry.checkpoint());
                }
            }
        }
        if let Some((observer, snapshot)) = notify {
            observer.on_execution(snapshot);
        }
        if self.recording_enabled()
            && let Some(checkpoint) = checkpoint
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
        let (observer, pending, checkpoint, persist) = {
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
            entry.snapshot.storage = if self.recording_enabled() {
                StorageState::Pending
            } else {
                StorageState::NotRecorded
            };
            (
                entry.observer.clone(),
                entry.snapshot.clone(),
                entry.checkpoint(),
                entry.persist,
            )
        };
        observer.on_execution(pending);

        let persisted = if persist
            && self.recording_enabled()
            && checkpoint.snapshot.retention_generation == self.retention_generation()
        {
            let storage = self.storage.clone();
            Some(
                tokio::task::spawn_blocking(move || {
                    storage
                        .enqueue(checkpoint, true)?
                        .ok_or(lexwisp_storage::StorageError::Closed)?
                        .wait()
                })
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string())),
            )
        } else {
            None
        };

        let (observer, final_snapshot) = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = entries
                .get_mut(invocation_id)
                .ok_or_else(|| "execution disappeared during terminal persistence".to_string())?;
            entry.snapshot.storage = match persisted {
                Some(Ok(())) => StorageState::Saved,
                Some(Err(_)) => StorageState::Unsaved,
                None => StorageState::NotRecorded,
            };
            entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
            entry.finished_at = Some(Instant::now());
            let notification = (entry.observer.clone(), entry.snapshot.clone());
            Self::prune_terminal_entries_locked(&mut entries, Instant::now(), Some(invocation_id));
            notification
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

    pub async fn persist_for_favorite(&self, invocation_id: &InvocationId) -> Result<(), String> {
        enum FavoriteLookup {
            Missing,
            Saved,
            Checkpoint(
                Box<(
                    ExecutionCheckpoint,
                    Arc<dyn ExecutionObserver>,
                    ExecutionSnapshot,
                )>,
            ),
        }

        let lookup = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            Self::prune_terminal_entries_locked(&mut entries, Instant::now(), Some(invocation_id));
            match entries.get_mut(invocation_id) {
                None => FavoriteLookup::Missing,
                Some(entry) if !entry.snapshot.status.is_terminal() => {
                    return Err("wait for the execution to finish before preserving it".into());
                }
                Some(entry) if entry.snapshot.storage == StorageState::Saved => {
                    FavoriteLookup::Saved
                }
                Some(entry) => {
                    entry.snapshot.retention_generation = self.retention_generation();
                    entry.snapshot.storage = StorageState::Pending;
                    entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                    FavoriteLookup::Checkpoint(Box::new((
                        entry.checkpoint(),
                        entry.observer.clone(),
                        entry.snapshot.clone(),
                    )))
                }
            }
        };
        let (checkpoint, observer, pending) = match lookup {
            FavoriteLookup::Missing => {
                let storage = self.storage.clone();
                let invocation_id = invocation_id.to_string();
                let exists =
                    tokio::task::spawn_blocking(move || storage.contains_execution(&invocation_id))
                        .await
                        .map_err(|error| error.to_string())?
                        .map_err(|error| error.to_string())?;
                return if exists {
                    Ok(())
                } else {
                    Err("result is no longer retained and was not persisted".into())
                };
            }
            FavoriteLookup::Saved => return Ok(()),
            FavoriteLookup::Checkpoint(checkpoint) => *checkpoint,
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
        let notification = {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let notification = entries.get_mut(invocation_id).map(|entry| {
                entry.snapshot.storage = if persisted.is_ok() {
                    StorageState::Saved
                } else {
                    StorageState::Unsaved
                };
                entry.snapshot.sequence = entry.snapshot.sequence.saturating_add(1);
                (entry.observer.clone(), entry.snapshot.clone())
            });
            Self::prune_terminal_entries_locked(&mut entries, Instant::now(), Some(invocation_id));
            notification
        };
        if let Some((observer, snapshot)) = notification {
            observer.on_execution(snapshot);
        }
        persisted
    }

    fn prune_terminal_entries_locked(
        entries: &mut HashMap<InvocationId, ExecutionAccumulator>,
        now: Instant,
        preserve: Option<&InvocationId>,
    ) {
        entries.retain(|invocation_id, entry| {
            !entry.snapshot.status.is_terminal()
                || entry.finished_at.is_none()
                || preserve == Some(invocation_id)
                || entry.finished_at.is_some_and(|finished_at| {
                    now.saturating_duration_since(finished_at) <= RECENT_TERMINAL_TTL
                })
        });

        let terminal_count = entries
            .values()
            .filter(|entry| entry.snapshot.status.is_terminal() && entry.finished_at.is_some())
            .count();
        let remove_count = terminal_count.saturating_sub(RECENT_TERMINAL_LIMIT);
        if remove_count == 0 {
            return;
        }
        let mut oldest = entries
            .iter()
            .filter(|(invocation_id, entry)| {
                entry.snapshot.status.is_terminal()
                    && entry.finished_at.is_some()
                    && preserve != Some(invocation_id)
            })
            .map(|(invocation_id, entry)| {
                (
                    invocation_id.clone(),
                    entry.finished_at.expect("checked above"),
                )
            })
            .collect::<Vec<_>>();
        oldest.sort_unstable_by_key(|(_, finished_at)| *finished_at);
        for (invocation_id, _) in oldest.into_iter().take(remove_count) {
            entries.remove(&invocation_id);
        }
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
        sync::{
            Mutex as StdMutex,
            atomic::{AtomicU64, Ordering},
        },
    };

    use lexwisp_storage::ContentStoreOwner;

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Sink;
    impl ExecutionObserver for Sink {
        fn on_execution(&self, _: ExecutionSnapshot) {}
    }

    struct RecordingSink(StdMutex<Vec<ExecutionStatus>>);

    impl ExecutionObserver for RecordingSink {
        fn on_execution(&self, snapshot: ExecutionSnapshot) {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(snapshot.status);
        }
    }

    fn empty_store(recording_enabled: bool) -> (ContentStoreOwner, ExecutionStore) {
        let path: PathBuf = std::env::temp_dir()
            .join("lexwisp-stage2-execution")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("lexwisp.db");
        let (owner, content) = ContentStoreOwner::start(path).expect("content store starts");
        (owner, ExecutionStore::new(content, recording_enabled, 0))
    }

    fn start_chat(
        store: &ExecutionStore,
        invocation_id: InvocationId,
        observer: Arc<dyn ExecutionObserver>,
    ) {
        let user_message_id = MessageId::new();
        store.start(ExecutionStart {
            invocation_id,
            conversation_id: ConversationId::new(),
            user_message_id: user_message_id.clone(),
            assistant_message_id: MessageId::new(),
            provider_id: ProviderId::parse("fixture").expect("provider ID"),
            model_id: "fixture".into(),
            input: "hello".into(),
            chat: ChatCheckpoint {
                conversation_title: "hello".into(),
                model_preference: lexwisp_core::ChatModelPreference::Fast,
                user_ordinal: 0,
                assistant_ordinal: 1,
                attempt_id: lexwisp_core::AttemptId::new(),
                reply_to_user_id: user_message_id,
            },
            observer,
        });
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
        let store = ExecutionStore::new(content, true, 0);
        let invocation_id = InvocationId::new();
        let user_message_id = MessageId::new();
        store.start(ExecutionStart {
            invocation_id: invocation_id.clone(),
            conversation_id: ConversationId::new(),
            user_message_id: user_message_id.clone(),
            assistant_message_id: MessageId::new(),
            provider_id: ProviderId::parse("fixture").expect("provider ID"),
            model_id: "fixture".into(),
            input: "hello".into(),
            chat: ChatCheckpoint {
                conversation_title: "hello".into(),
                model_preference: lexwisp_core::ChatModelPreference::Fast,
                user_ordinal: 0,
                assistant_ordinal: 1,
                attempt_id: lexwisp_core::AttemptId::new(),
                reply_to_user_id: user_message_id,
            },
            observer: Arc::new(Sink),
        });
        (owner, store, invocation_id)
    }

    #[test]
    fn many_small_deltas_are_coalesced_for_storage() {
        let (owner, store, invocation_id) = fixture();
        for _ in 0..1_000 {
            store
                .append_text(&invocation_id, "x")
                .expect("delta applies");
        }
        assert!(store.storage.enqueued_checkpoints() < 20);
        owner.shutdown();
    }

    #[test]
    fn disabling_recording_revokes_running_checkpoint_writes() {
        let (owner, store, invocation_id) = fixture();
        let before = store.storage.enqueued_checkpoints();
        store.set_recording_enabled(false);
        store
            .append_text(&invocation_id, &"x".repeat(CHECKPOINT_BYTES + 1))
            .expect("the in-memory result remains usable");
        assert_eq!(store.storage.enqueued_checkpoints(), before);
        assert_eq!(store.storage.retention_generation().expect("generation"), 1);
        assert_eq!(
            store.snapshot(&invocation_id).expect("snapshot").storage,
            StorageState::NotRecorded
        );
        owner.shutdown();
    }

    #[tokio::test]
    async fn explicit_favorite_can_preserve_a_terminal_unrecorded_result() {
        let (owner, store, invocation_id) = fixture();
        store.set_recording_enabled(false);
        store
            .append_text(&invocation_id, "answer")
            .expect("answer remains in memory");
        store
            .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
            .await
            .expect("terminal state commits in memory");
        assert_eq!(
            store.snapshot(&invocation_id).expect("snapshot").storage,
            StorageState::NotRecorded
        );
        store
            .persist_for_favorite(&invocation_id)
            .await
            .expect("explicit preservation writes the authoritative body");
        let detail = store
            .storage
            .history_detail(invocation_id.as_str())
            .expect("history query")
            .expect("preserved execution exists");
        assert_eq!(detail.output, "answer");
        owner.shutdown();
    }

    #[tokio::test]
    async fn execution_waiting_for_a_concurrency_permit_stays_queued() {
        let (owner, store) = empty_store(false);
        let invocation_id = InvocationId::new();
        let observer = Arc::new(RecordingSink(StdMutex::new(Vec::new())));
        start_chat(&store, invocation_id.clone(), observer.clone());
        let semaphore = Arc::new(tokio::sync::Semaphore::new(1));
        let first_permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("first invocation occupies the permit");
        let waiting_store = store.clone();
        let waiting_invocation = invocation_id.clone();
        let waiting_semaphore = semaphore.clone();
        let waiting = tokio::spawn(async move {
            let permit = waiting_semaphore
                .acquire_owned()
                .await
                .expect("second invocation acquires the released permit");
            waiting_store
                .mark_running(&waiting_invocation)
                .expect("permit transition succeeds");
            drop(permit);
        });
        tokio::task::yield_now().await;
        assert_eq!(
            store
                .snapshot(&invocation_id)
                .expect("queued snapshot")
                .status,
            ExecutionStatus::Queued
        );
        drop(first_permit);
        waiting.await.expect("waiting invocation completes");
        assert_eq!(
            store
                .snapshot(&invocation_id)
                .expect("running snapshot")
                .status,
            ExecutionStatus::Running
        );
        assert_eq!(
            observer
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            [ExecutionStatus::Queued, ExecutionStatus::Running]
        );
        store
            .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
            .await
            .expect("running invocation reaches terminal state");
        assert_eq!(
            store
                .snapshot(&invocation_id)
                .expect("terminal snapshot")
                .status,
            ExecutionStatus::Completed
        );
        owner.shutdown();
    }

    #[tokio::test]
    async fn terminal_retention_is_bounded_without_pruning_active_entries() {
        let (owner, store) = empty_store(false);
        let active = InvocationId::new();
        start_chat(&store, active.clone(), Arc::new(Sink));

        for _ in 0..500 {
            let invocation_id = InvocationId::new();
            start_chat(&store, invocation_id.clone(), Arc::new(Sink));
            store.mark_running(&invocation_id).expect("starts running");
            store
                .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
                .await
                .expect("terminal state commits");
        }

        let entries = store
            .entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(entries.contains_key(&active));
        assert_eq!(
            entries
                .values()
                .filter(|entry| !entry.snapshot.status.is_terminal())
                .count(),
            1
        );
        assert!(
            entries
                .values()
                .filter(|entry| entry.snapshot.status.is_terminal())
                .count()
                <= RECENT_TERMINAL_LIMIT
        );
        drop(entries);
        owner.shutdown();
    }

    #[tokio::test]
    async fn persisted_pruned_execution_can_still_be_favorited() {
        let (owner, store) = empty_store(true);
        let invocation_id = InvocationId::new();
        start_chat(&store, invocation_id.clone(), Arc::new(Sink));
        store.mark_running(&invocation_id).expect("starts running");
        store
            .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
            .await
            .expect("terminal state persists");
        {
            let mut entries = store
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ExecutionStore::prune_terminal_entries_locked(
                &mut entries,
                Instant::now() + RECENT_TERMINAL_TTL * 2,
                None,
            );
        }
        let trigger = InvocationId::new();
        start_chat(&store, trigger, Arc::new(Sink));
        assert!(store.snapshot(&invocation_id).is_none());

        store
            .persist_for_favorite(&invocation_id)
            .await
            .expect("persisted execution is found after pruning");
        assert!(
            store
                .storage
                .toggle_favorite(invocation_id.as_str())
                .expect("favorite toggles")
        );
        owner.shutdown();
    }

    #[tokio::test]
    async fn unrecorded_pruned_execution_reports_a_clear_error() {
        let (owner, store) = empty_store(false);
        let invocation_id = InvocationId::new();
        start_chat(&store, invocation_id.clone(), Arc::new(Sink));
        store.mark_running(&invocation_id).expect("starts running");
        store
            .commit_terminal(&invocation_id, ExecutionStatus::Completed, None)
            .await
            .expect("terminal state commits in memory");
        {
            let mut entries = store
                .entries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            ExecutionStore::prune_terminal_entries_locked(
                &mut entries,
                Instant::now() + RECENT_TERMINAL_TTL * 2,
                None,
            );
        }
        start_chat(&store, InvocationId::new(), Arc::new(Sink));
        let error = store
            .persist_for_favorite(&invocation_id)
            .await
            .expect_err("unrecorded pruned output cannot be recovered");
        assert_eq!(error, "result is no longer retained and was not persisted");
        owner.shutdown();
    }
}

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{ExecutionCheckpoint, ExecutionStatus};
use rusqlite::{Connection, OptionalExtension as _, params};
use thiserror::Error;

const STORAGE_QUEUE_CAPACITY: usize = 64;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum StorageError {
    #[error("could not start content storage: {0}")]
    Start(String),
    #[error("content storage queue is full")]
    QueueFull,
    #[error("content storage is closed")]
    Closed,
    #[error("content storage operation failed: {0}")]
    Sql(String),
}

enum StorageCommand {
    Checkpoint {
        checkpoint: Box<ExecutionCheckpoint>,
        receipt: Option<mpsc::Sender<Result<(), StorageError>>>,
    },
    Shutdown,
}

pub struct WriteReceipt(mpsc::Receiver<Result<(), StorageError>>);

impl WriteReceipt {
    pub fn wait(self) -> Result<(), StorageError> {
        self.0.recv().unwrap_or(Err(StorageError::Closed))
    }
}

#[derive(Clone)]
pub struct ContentStore {
    sender: Sender<StorageCommand>,
    enqueued_checkpoints: Arc<AtomicU64>,
}

impl ContentStore {
    pub fn enqueue(
        &self,
        checkpoint: ExecutionCheckpoint,
        terminal: bool,
    ) -> Result<Option<WriteReceipt>, StorageError> {
        let (receipt_sender, receipt) = if terminal {
            let (sender, receiver) = mpsc::channel();
            (Some(sender), Some(WriteReceipt(receiver)))
        } else {
            (None, None)
        };
        let command = StorageCommand::Checkpoint {
            checkpoint: Box::new(checkpoint),
            receipt: receipt_sender,
        };
        if terminal {
            self.sender
                .send_blocking(command)
                .map_err(|_| StorageError::Closed)?;
        } else {
            self.sender.try_send(command).map_err(|error| match error {
                TrySendError::Full(_) => StorageError::QueueFull,
                TrySendError::Closed(_) => StorageError::Closed,
            })?;
        }
        self.enqueued_checkpoints.fetch_add(1, Ordering::Relaxed);
        Ok(receipt)
    }

    pub fn enqueued_checkpoints(&self) -> u64 {
        self.enqueued_checkpoints.load(Ordering::Relaxed)
    }
}

pub struct ContentStoreOwner {
    sender: Sender<StorageCommand>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ContentStoreOwner {
    pub fn start(path: PathBuf) -> Result<(Self, ContentStore), StorageError> {
        let (sender, receiver) = async_channel::bounded(STORAGE_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("lexwisp-sqlite".into())
            .spawn(move || worker_main(path, receiver, ready_sender))
            .map_err(|error| StorageError::Start(error.to_string()))?;
        ready_receiver
            .recv()
            .map_err(|_| StorageError::Start("SQLite worker exited during startup".into()))??;
        let checkpoints = Arc::new(AtomicU64::new(0));
        Ok((
            Self {
                sender: sender.clone(),
                worker: Some(worker),
            },
            ContentStore {
                sender,
                enqueued_checkpoints: checkpoints,
            },
        ))
    }

    pub fn shutdown(mut self) {
        let _ = self.sender.send_blocking(StorageCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_main(
    path: PathBuf,
    receiver: Receiver<StorageCommand>,
    ready: mpsc::Sender<Result<(), StorageError>>,
) {
    let mut connection = match open(&path) {
        Ok(connection) => {
            let _ = ready.send(Ok(()));
            connection
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    while let Ok(command) = receiver.recv_blocking() {
        match command {
            StorageCommand::Checkpoint {
                checkpoint,
                receipt,
            } => {
                let result = write_checkpoint(&mut connection, &checkpoint);
                if let Some(receipt) = receipt {
                    let _ = receipt.send(result);
                }
            }
            StorageCommand::Shutdown => break,
        }
    }
}

fn open(path: &Path) -> Result<Connection, StorageError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| StorageError::Start(error.to_string()))?;
    }
    let connection = Connection::open(path).map_err(sql_error)?;
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS schema_version (
                 version INTEGER NOT NULL
             );
             INSERT INTO schema_version(version)
                 SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
             CREATE TABLE IF NOT EXISTS conversations (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS messages (
                 id TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                 role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
                 ordinal INTEGER NOT NULL,
                 content TEXT NOT NULL,
                 status TEXT NOT NULL,
                 invocation_id TEXT,
                 sequence INTEGER NOT NULL,
                 retention_generation INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 UNIQUE(conversation_id, ordinal)
             );
             CREATE TABLE IF NOT EXISTS executions (
                 id TEXT PRIMARY KEY,
                 plugin_id TEXT NOT NULL,
                 plugin_generation INTEGER NOT NULL,
                 provider_id TEXT NOT NULL,
                 model_id TEXT NOT NULL,
                 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                 user_message_id TEXT NOT NULL REFERENCES messages(id),
                 assistant_message_id TEXT NOT NULL REFERENCES messages(id),
                 status TEXT NOT NULL,
                 sequence INTEGER NOT NULL,
                 retention_generation INTEGER NOT NULL,
                 error TEXT,
                 started_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS messages_conversation_ordinal
                 ON messages(conversation_id, ordinal);
             CREATE INDEX IF NOT EXISTS executions_conversation_updated
                 ON executions(conversation_id, updated_at_ms DESC);
             UPDATE executions SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE status IN ('queued', 'running', 'cancelling');
             UPDATE messages SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE status IN ('submitted', 'generating');",
        )
        .map_err(sql_error)?;
    let version: i64 = connection
        .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
            row.get(0)
        })
        .map_err(sql_error)?;
    if version != 1 {
        return Err(StorageError::Start(format!(
            "database schema version {version} is not supported"
        )));
    }
    Ok(connection)
}

fn write_checkpoint(
    connection: &mut Connection,
    checkpoint: &ExecutionCheckpoint,
) -> Result<(), StorageError> {
    let snapshot = &checkpoint.snapshot;
    let user_ordinal = sqlite_u64(checkpoint.user_ordinal, "user ordinal")?;
    let assistant_ordinal = sqlite_u64(checkpoint.assistant_ordinal, "assistant ordinal")?;
    let plugin_generation = sqlite_u64(snapshot.plugin_generation, "plugin generation")?;
    let sequence = sqlite_u64(snapshot.sequence, "execution sequence")?;
    let retention_generation = sqlite_u64(snapshot.retention_generation, "retention generation")?;
    let now = now_ms();
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO conversations(id, title, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(id) DO UPDATE SET title = excluded.title, updated_at_ms = excluded.updated_at_ms",
            params![snapshot.conversation_id.as_str(), checkpoint.conversation_title, now],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO messages(
                 id, conversation_id, role, ordinal, content, status, invocation_id,
                 sequence, retention_generation, updated_at_ms
             ) VALUES (?1, ?2, 'user', ?3, ?4, 'submitted', NULL, 0, ?5, ?6)
             ON CONFLICT(id) DO NOTHING",
            params![
                snapshot.user_message_id.as_str(),
                snapshot.conversation_id.as_str(),
                user_ordinal,
                checkpoint.input,
                retention_generation,
                now,
            ],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO messages(
                 id, conversation_id, role, ordinal, content, status, invocation_id,
                 sequence, retention_generation, updated_at_ms
             ) VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET
                 content = excluded.content,
                 status = excluded.status,
                 sequence = excluded.sequence,
                 updated_at_ms = excluded.updated_at_ms
             WHERE excluded.sequence > messages.sequence
               AND excluded.retention_generation = messages.retention_generation",
            params![
                snapshot.assistant_message_id.as_str(),
                snapshot.conversation_id.as_str(),
                assistant_ordinal,
                snapshot.output,
                status_name(snapshot.status),
                snapshot.invocation_id.as_str(),
                sequence,
                retention_generation,
                now,
            ],
        )
        .map_err(sql_error)?;
    let started_at = transaction
        .query_row(
            "SELECT started_at_ms FROM executions WHERE id = ?1",
            [snapshot.invocation_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(sql_error)?
        .unwrap_or(now);
    transaction
        .execute(
            "INSERT INTO executions(
                 id, plugin_id, plugin_generation, provider_id, model_id, conversation_id,
                 user_message_id, assistant_message_id, status, sequence, retention_generation,
                 error, started_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
             ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 sequence = excluded.sequence,
                 error = excluded.error,
                 updated_at_ms = excluded.updated_at_ms
             WHERE excluded.sequence > executions.sequence
               AND excluded.retention_generation = executions.retention_generation",
            params![
                snapshot.invocation_id.as_str(),
                snapshot.plugin_id.as_str(),
                plugin_generation,
                snapshot.provider_id.as_str(),
                snapshot.model_id,
                snapshot.conversation_id.as_str(),
                snapshot.user_message_id.as_str(),
                snapshot.assistant_message_id.as_str(),
                status_name(snapshot.status),
                sequence,
                retention_generation,
                snapshot.error,
                started_at,
                now,
            ],
        )
        .map_err(sql_error)?;
    transaction.commit().map_err(sql_error)
}

fn status_name(status: ExecutionStatus) -> &'static str {
    match status {
        ExecutionStatus::Queued => "queued",
        ExecutionStatus::Running => "running",
        ExecutionStatus::Cancelling => "cancelling",
        ExecutionStatus::Completed => "completed",
        ExecutionStatus::Failed => "failed",
        ExecutionStatus::Cancelled => "cancelled",
        ExecutionStatus::Interrupted => "interrupted",
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn sqlite_u64(value: u64, field: &str) -> Result<i64, StorageError> {
    value
        .try_into()
        .map_err(|_| StorageError::Sql(format!("{field} exceeds SQLite's integer range")))
}

fn sql_error(error: rusqlite::Error) -> StorageError {
    StorageError::Sql(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::atomic::AtomicU64};

    use lexwisp_core::{
        ConversationId, ExecutionSnapshot, InvocationId, MessageId, PluginId, ProviderId,
        StorageState,
    };

    use super::*;

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> PathBuf {
        std::env::temp_dir()
            .join("lexwisp-stage2-storage")
            .join(format!(
                "{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("lexwisp.db")
    }

    fn checkpoint(sequence: u64, output: &str, status: ExecutionStatus) -> ExecutionCheckpoint {
        ExecutionCheckpoint {
            snapshot: ExecutionSnapshot {
                invocation_id: InvocationId::new(),
                plugin_id: PluginId::parse("org.lexwisp.chat").expect("valid plugin ID"),
                plugin_generation: 1,
                conversation_id: ConversationId::new(),
                user_message_id: MessageId::new(),
                assistant_message_id: MessageId::new(),
                provider_id: ProviderId::parse("default").expect("valid provider ID"),
                model_id: "fixture".into(),
                sequence,
                text_version: sequence,
                status,
                output: output.into(),
                error: None,
                storage: StorageState::Pending,
                retention_generation: 0,
            },
            input: "hello".into(),
            conversation_title: "hello".into(),
            user_ordinal: 0,
            assistant_ordinal: 1,
        }
    }

    #[test]
    fn old_checkpoint_cannot_overwrite_a_terminal_message() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let terminal = checkpoint(3, "complete", ExecutionStatus::Completed);
        let mut stale = terminal.clone();
        stale.snapshot.sequence = 2;
        stale.snapshot.output = "stale".into();
        stale.snapshot.status = ExecutionStatus::Running;
        store
            .enqueue(terminal.clone(), true)
            .expect("terminal enqueues")
            .expect("terminal has receipt")
            .wait()
            .expect("terminal persists");
        store.enqueue(stale, false).expect("stale enqueues");
        owner.shutdown();

        let connection = Connection::open(&path).expect("database opens");
        let (content, status): (String, String) = connection
            .query_row(
                "SELECT content, status FROM messages WHERE id = ?1",
                [terminal.snapshot.assistant_message_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("assistant message exists");
        assert_eq!(content, "complete");
        assert_eq!(status, "completed");
        drop(connection);
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }
}

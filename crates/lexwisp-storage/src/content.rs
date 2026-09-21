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
use lexwisp_core::{
    AttemptId, ChatError, ChatHistoryPort, ChatMessageSnapshot, ChatMessageStatus,
    ChatModelPreference, ClearHistoryMode, ConversationId, ExecutionCheckpoint, ExecutionStatus,
    HistoryCursor, HistoryDetail, HistoryItem, HistoryPage, HistoryQuery, InvocationId, MessageId,
    PersistedChatConversation,
};
use rusqlite::{Connection, MAIN_DB, OptionalExtension as _, params};
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
    ToggleFavorite {
        invocation_id: String,
        reply: mpsc::Sender<Result<bool, StorageError>>,
    },
    ContainsFavorite {
        invocation_id: String,
        reply: mpsc::Sender<Result<bool, StorageError>>,
    },
    ContainsExecution {
        invocation_id: String,
        reply: mpsc::Sender<Result<bool, StorageError>>,
    },
    FavoriteIds {
        reply: mpsc::Sender<Result<Vec<String>, StorageError>>,
    },
    SetFavorite {
        invocation_id: String,
        favorite: bool,
        note: String,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    QueryHistory {
        query: HistoryQuery,
        reply: mpsc::Sender<Result<HistoryPage, StorageError>>,
    },
    HistoryDetail {
        invocation_id: String,
        reply: mpsc::Sender<Result<Option<HistoryDetail>, StorageError>>,
    },
    DeleteHistory {
        invocation_id: String,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    ClearHistory {
        mode: ClearHistoryMode,
        generation: u64,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    Backup {
        path: PathBuf,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    RetentionGeneration {
        reply: mpsc::Sender<Result<u64, StorageError>>,
    },
    SetRetentionGeneration {
        generation: u64,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    RestoreConversations {
        reply: mpsc::Sender<Result<Vec<PersistedChatConversation>, StorageError>>,
    },
    SaveConversation {
        conversation_id: String,
        title: String,
        model_preference: String,
        reply: Option<mpsc::Sender<Result<(), StorageError>>>,
    },
    DeleteConversation {
        conversation_id: String,
        reply: Option<mpsc::Sender<Result<(), StorageError>>>,
    },
    SetConversationFavorite {
        conversation_id: String,
        favorite: bool,
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

    pub fn toggle_favorite(&self, invocation_id: &str) -> Result<bool, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::ToggleFavorite {
                invocation_id: invocation_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn contains_favorite(&self, invocation_id: &str) -> Result<bool, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::ContainsFavorite {
                invocation_id: invocation_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn contains_execution(&self, invocation_id: &str) -> Result<bool, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::ContainsExecution {
                invocation_id: invocation_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn favorite_ids(&self) -> Result<Vec<String>, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::FavoriteIds { reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn set_favorite(
        &self,
        invocation_id: &str,
        favorite: bool,
        note: &str,
    ) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::SetFavorite {
                invocation_id: invocation_id.to_owned(),
                favorite,
                note: note.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn query_history(&self, query: HistoryQuery) -> Result<HistoryPage, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::QueryHistory { query, reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn history_detail(
        &self,
        invocation_id: &str,
    ) -> Result<Option<HistoryDetail>, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::HistoryDetail {
                invocation_id: invocation_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn delete_history(&self, invocation_id: &str) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::DeleteHistory {
                invocation_id: invocation_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn clear_history(
        &self,
        mode: ClearHistoryMode,
        generation: u64,
    ) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::ClearHistory {
                mode,
                generation,
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn backup(&self, path: PathBuf) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::Backup { path, reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn retention_generation(&self) -> Result<u64, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::RetentionGeneration { reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn set_retention_generation(&self, generation: u64) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::SetRetentionGeneration { generation, reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    fn restore_conversations(&self) -> Result<Vec<PersistedChatConversation>, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::RestoreConversations { reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    fn save_conversation_metadata(
        &self,
        conversation_id: &ConversationId,
        title: &str,
        model_preference: &ChatModelPreference,
    ) -> Result<(), StorageError> {
        self.sender
            .try_send(StorageCommand::SaveConversation {
                conversation_id: conversation_id.to_string(),
                title: title.to_owned(),
                model_preference: model_preference.persistence_name(),
                reply: None,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => StorageError::QueueFull,
                TrySendError::Closed(_) => StorageError::Closed,
            })
    }

    fn delete_chat_conversation(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<(), StorageError> {
        self.sender
            .try_send(StorageCommand::DeleteConversation {
                conversation_id: conversation_id.to_string(),
                reply: None,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => StorageError::QueueFull,
                TrySendError::Closed(_) => StorageError::Closed,
            })
    }

    fn set_chat_conversation_favorite(
        &self,
        conversation_id: &ConversationId,
        favorite: bool,
    ) -> Result<(), StorageError> {
        self.sender
            .try_send(StorageCommand::SetConversationFavorite {
                conversation_id: conversation_id.to_string(),
                favorite,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => StorageError::QueueFull,
                TrySendError::Closed(_) => StorageError::Closed,
            })
    }
}

impl ChatHistoryPort for ContentStore {
    fn restore(&self) -> Result<Vec<PersistedChatConversation>, ChatError> {
        self.restore_conversations()
            .map_err(|error| ChatError::Failed(error.to_string()))
    }

    fn save_conversation(
        &self,
        conversation_id: &ConversationId,
        title: &str,
        model_preference: &ChatModelPreference,
    ) -> Result<(), ChatError> {
        self.save_conversation_metadata(conversation_id, title, model_preference)
            .map_err(|error| ChatError::Failed(error.to_string()))
    }

    fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError> {
        self.delete_chat_conversation(conversation_id)
            .map_err(|error| ChatError::Failed(error.to_string()))
    }

    fn set_conversation_favorite(
        &self,
        conversation_id: &ConversationId,
        favorite: bool,
    ) -> Result<(), ChatError> {
        self.set_chat_conversation_favorite(conversation_id, favorite)
            .map_err(|error| ChatError::Failed(error.to_string()))
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
            StorageCommand::ToggleFavorite {
                invocation_id,
                reply,
            } => {
                let _ = reply.send(toggle_favorite(&mut connection, &invocation_id));
            }
            StorageCommand::ContainsFavorite {
                invocation_id,
                reply,
            } => {
                let _ = reply.send(contains_favorite(&connection, &invocation_id));
            }
            StorageCommand::ContainsExecution {
                invocation_id,
                reply,
            } => {
                let result = connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM executions WHERE id = ?1)",
                        [invocation_id],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(sql_error);
                let _ = reply.send(result);
            }
            StorageCommand::FavoriteIds { reply } => {
                let result = (|| {
                    let mut statement = connection
                        .prepare("SELECT invocation_id FROM favorites ORDER BY invocation_id")
                        .map_err(sql_error)?;
                    statement
                        .query_map([], |row| row.get::<_, String>(0))
                        .map_err(sql_error)?
                        .map(|row| row.map_err(sql_error))
                        .collect()
                })();
                let _ = reply.send(result);
            }
            StorageCommand::SetFavorite {
                invocation_id,
                favorite,
                note,
                reply,
            } => {
                let _ = reply.send(set_favorite(
                    &mut connection,
                    &invocation_id,
                    favorite,
                    &note,
                ));
            }
            StorageCommand::QueryHistory { query, reply } => {
                let _ = reply.send(query_history(&connection, &query));
            }
            StorageCommand::HistoryDetail {
                invocation_id,
                reply,
            } => {
                let _ = reply.send(history_detail(&connection, &invocation_id));
            }
            StorageCommand::DeleteHistory {
                invocation_id,
                reply,
            } => {
                let _ = reply.send(delete_history(&mut connection, &invocation_id));
            }
            StorageCommand::ClearHistory {
                mode,
                generation,
                reply,
            } => {
                let _ = reply.send(clear_history(&mut connection, mode, generation));
            }
            StorageCommand::Backup { path, reply } => {
                let result = path
                    .parent()
                    .map(std::fs::create_dir_all)
                    .transpose()
                    .map_err(|error| StorageError::Sql(error.to_string()))
                    .and_then(|_| connection.backup(MAIN_DB, path, None).map_err(sql_error));
                let _ = reply.send(result);
            }
            StorageCommand::RetentionGeneration { reply } => {
                let result = connection
                    .query_row(
                        "SELECT generation FROM retention_state WHERE singleton = 1",
                        [],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(sql_error)
                    .and_then(|value| {
                        value.try_into().map_err(|_| {
                            StorageError::Sql("retention generation is negative".into())
                        })
                    });
                let _ = reply.send(result);
            }
            StorageCommand::SetRetentionGeneration { generation, reply } => {
                let result = sqlite_u64(generation, "retention generation").and_then(|value| {
                    connection
                        .execute(
                            "UPDATE retention_state SET generation = ?1 WHERE singleton = 1",
                            [value],
                        )
                        .map(|_| ())
                        .map_err(sql_error)
                });
                let _ = reply.send(result);
            }
            StorageCommand::RestoreConversations { reply } => {
                let _ = reply.send(load_conversations(&connection));
            }
            StorageCommand::SaveConversation {
                conversation_id,
                title,
                model_preference,
                reply,
            } => {
                let result =
                    save_conversation(&connection, &conversation_id, &title, &model_preference);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            StorageCommand::DeleteConversation {
                conversation_id,
                reply,
            } => {
                let result = delete_conversation(&mut connection, &conversation_id);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            StorageCommand::SetConversationFavorite {
                conversation_id,
                favorite,
            } => {
                let _ = connection.execute(
                    "UPDATE conversations SET favorite = ?2 WHERE id = ?1",
                    params![conversation_id, favorite],
                );
            }
            StorageCommand::Shutdown => break,
        }
    }
}

fn open(path: &Path) -> Result<Connection, StorageError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| StorageError::Start(error.to_string()))?;
    }
    let mut connection = Connection::open(path).map_err(sql_error)?;
    connection
        .execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")
        .map_err(sql_error)?;
    let has_schema: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version')",
            [],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let existing_version: Option<i64> = if has_schema {
        Some(
            connection
                .query_row("SELECT version FROM schema_version LIMIT 1", [], |row| {
                    row.get(0)
                })
                .map_err(sql_error)?,
        )
    } else {
        None
    };
    if let Some(version) = existing_version
        && version != 1
        && version != 2
        && version != 3
        && version != 4
        && version != 5
        && version != 6
        && version != 7
    {
        return Err(StorageError::Start(format!(
            "database schema version {version} is not supported"
        )));
    }
    if existing_version == Some(7) {
        connection.execute_batch(
            "UPDATE executions SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE status IN ('queued', 'running', 'cancelling');
             UPDATE messages SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE role = 'assistant' AND status = 'generating';
             UPDATE messages SET status = 'submitted'
                 WHERE role = 'user' AND status = 'interrupted';"
        ).map_err(sql_error)?;
        return Ok(connection);
    }
    if existing_version.is_none() {
        connection.execute_batch(
            "CREATE TABLE schema_version (version INTEGER NOT NULL);
             INSERT INTO schema_version(version) VALUES (7);
             CREATE TABLE conversations (
                 id TEXT PRIMARY KEY, title TEXT NOT NULL,
                 model_preference TEXT NOT NULL DEFAULT 'profile:fast',
                 favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0, 1)),
                 created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE messages (
                 id TEXT PRIMARY KEY,
                 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                 role TEXT NOT NULL CHECK(role IN ('user', 'assistant')),
                 ordinal INTEGER NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL,
                 invocation_id TEXT, attempt_id TEXT, reply_to_message_id TEXT,
                 sequence INTEGER NOT NULL, retention_generation INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL, UNIQUE(conversation_id, ordinal)
             );
             CREATE TABLE executions (
                 id TEXT PRIMARY KEY, provider_id TEXT NOT NULL, model_id TEXT NOT NULL,
                 conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                 user_message_id TEXT NOT NULL REFERENCES messages(id),
                 assistant_message_id TEXT NOT NULL REFERENCES messages(id),
                 status TEXT NOT NULL, sequence INTEGER NOT NULL,
                 retention_generation INTEGER NOT NULL, error TEXT,
                 started_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL
             );
             CREATE INDEX messages_conversation_ordinal ON messages(conversation_id, ordinal);
             CREATE INDEX executions_conversation_updated ON executions(conversation_id, updated_at_ms DESC);
             CREATE TABLE conversation_deletions (conversation_id TEXT PRIMARY KEY, deleted_at_ms INTEGER NOT NULL);
             CREATE TABLE favorites (invocation_id TEXT PRIMARY KEY, created_at_ms INTEGER NOT NULL, note TEXT NOT NULL DEFAULT '');
             CREATE TABLE execution_deletions (invocation_id TEXT PRIMARY KEY, deleted_at_ms INTEGER NOT NULL);
             CREATE TABLE retention_state (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), generation INTEGER NOT NULL);
             INSERT INTO retention_state(singleton, generation) VALUES (1, 0);"
        ).map_err(sql_error)?;
        return Ok(connection);
    }
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_version (
                 version INTEGER NOT NULL
             );
             INSERT INTO schema_version(version)
                 SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM schema_version);
             CREATE TABLE IF NOT EXISTS conversations (
                 id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 model_preference TEXT NOT NULL DEFAULT 'profile:fast',
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
                 attempt_id TEXT,
                 reply_to_message_id TEXT,
                 sequence INTEGER NOT NULL,
                 retention_generation INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 UNIQUE(conversation_id, ordinal)
             );
             CREATE TABLE IF NOT EXISTS executions (
                 id TEXT PRIMARY KEY,
                 plugin_id TEXT NOT NULL,
                 action_id TEXT NOT NULL DEFAULT 'org.lexwisp.chat/ask',
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
             CREATE TABLE IF NOT EXISTS conversation_deletions (
                 conversation_id TEXT PRIMARY KEY,
                 deleted_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS action_executions (
                 id TEXT PRIMARY KEY,
                 plugin_id TEXT NOT NULL,
                 action_id TEXT NOT NULL,
                 plugin_generation INTEGER NOT NULL,
                 provider_id TEXT NOT NULL,
                 model_id TEXT NOT NULL,
                 input TEXT NOT NULL,
                 output TEXT NOT NULL,
                 status TEXT NOT NULL,
                 sequence INTEGER NOT NULL,
                 retention_generation INTEGER NOT NULL,
                 error TEXT,
                 started_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS action_executions_updated
                 ON action_executions(updated_at_ms DESC);
             CREATE TABLE IF NOT EXISTS favorites (
                 invocation_id TEXT PRIMARY KEY,
                 created_at_ms INTEGER NOT NULL,
                 note TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS execution_deletions (
                 invocation_id TEXT PRIMARY KEY,
                 deleted_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS retention_state (
                 singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
                 generation INTEGER NOT NULL
             );
             INSERT OR IGNORE INTO retention_state(singleton, generation) VALUES (1, 0);
             CREATE TABLE IF NOT EXISTS installed_plugins (
                 id TEXT PRIMARY KEY,
                 name TEXT NOT NULL,
                 version TEXT NOT NULL,
                 kind TEXT NOT NULL DEFAULT 'declarative',
                 package_hash TEXT NOT NULL,
                 source_path TEXT NOT NULL,
                 install_path TEXT NOT NULL,
                 enabled INTEGER NOT NULL CHECK(enabled IN (0, 1)),
                 generation INTEGER NOT NULL,
                 last_error TEXT,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS plugin_grants (
                 plugin_id TEXT NOT NULL REFERENCES installed_plugins(id) ON DELETE CASCADE,
                 package_hash TEXT NOT NULL,
                 generation INTEGER NOT NULL,
                 capability TEXT NOT NULL,
                 PRIMARY KEY(plugin_id, capability)
             );
             CREATE TABLE IF NOT EXISTS plugin_kv (
                 plugin_id TEXT NOT NULL,
                 key TEXT NOT NULL,
                 value TEXT NOT NULL,
                 size_bytes INTEGER NOT NULL,
                 schema_version INTEGER NOT NULL DEFAULT 1,
                 updated_at_ms INTEGER NOT NULL,
                 PRIMARY KEY(plugin_id, key)
             );
             UPDATE executions SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE status IN ('queued', 'running', 'cancelling');
             UPDATE action_executions SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE status IN ('queued', 'running', 'cancelling');
             UPDATE messages SET status = 'interrupted', updated_at_ms = unixepoch('subsec') * 1000
                 WHERE role = 'assistant' AND status = 'generating';
             UPDATE messages SET status = 'submitted'
                 WHERE role = 'user' AND status = 'interrupted';",
        )
        .map_err(sql_error)?;
    if existing_version.is_some_and(|version| version < 3) {
        connection
            .execute_batch(
                "ALTER TABLE conversations ADD COLUMN model_preference TEXT NOT NULL DEFAULT 'profile:fast';
                 ALTER TABLE messages ADD COLUMN attempt_id TEXT;
                 ALTER TABLE messages ADD COLUMN reply_to_message_id TEXT;",
            )
            .map_err(sql_error)?;
    }
    if existing_version.is_some_and(|version| version < 4) {
        connection
            .execute_batch(
                "ALTER TABLE favorites ADD COLUMN note TEXT NOT NULL DEFAULT '';
                 ALTER TABLE executions ADD COLUMN action_id TEXT NOT NULL DEFAULT 'org.lexwisp.chat/ask';",
            )
            .map_err(sql_error)?;
    }
    if existing_version == Some(5) {
        connection
            .execute_batch(
                "ALTER TABLE installed_plugins ADD COLUMN kind TEXT NOT NULL DEFAULT 'declarative';
                 CREATE TABLE IF NOT EXISTS plugin_kv (
                     plugin_id TEXT NOT NULL,
                     key TEXT NOT NULL,
                     value TEXT NOT NULL,
                     size_bytes INTEGER NOT NULL,
                     schema_version INTEGER NOT NULL DEFAULT 1,
                     updated_at_ms INTEGER NOT NULL,
                     PRIMARY KEY(plugin_id, key)
                 );",
            )
            .map_err(sql_error)?;
    }
    // Legacy package and text-action tables are used only while upgrading older databases.
    // Chat messages and runs remain intact.
    let migration = connection.transaction().map_err(sql_error)?;
    migration.execute_batch(
        "ALTER TABLE conversations ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0 CHECK(favorite IN (0, 1));
         DELETE FROM favorites WHERE invocation_id NOT IN (SELECT id FROM executions);
         DROP TABLE IF EXISTS plugin_grants;
         DROP TABLE IF EXISTS plugin_kv;
         DROP TABLE IF EXISTS installed_plugins;
         DROP TABLE IF EXISTS action_executions;
         ALTER TABLE executions DROP COLUMN plugin_id;
         ALTER TABLE executions DROP COLUMN action_id;
         ALTER TABLE executions DROP COLUMN plugin_generation;
         UPDATE schema_version SET version = 7;"
    ).map_err(sql_error)?;
    migration.commit().map_err(sql_error)?;
    Ok(connection)
}

fn write_checkpoint(
    connection: &mut Connection,
    checkpoint: &ExecutionCheckpoint,
) -> Result<(), StorageError> {
    let snapshot = &checkpoint.snapshot;
    let sequence = sqlite_u64(snapshot.sequence, "execution sequence")?;
    let retention_generation = sqlite_u64(snapshot.retention_generation, "retention generation")?;
    let now = now_ms();
    let transaction = connection.transaction().map_err(sql_error)?;
    let current_generation: i64 = transaction
        .query_row(
            "SELECT generation FROM retention_state WHERE singleton = 1",
            [],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    let deleted: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM execution_deletions WHERE invocation_id = ?1)",
            [snapshot.invocation_id.as_str()],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if current_generation != retention_generation || deleted {
        return transaction.commit().map_err(sql_error);
    }
    let chat = &checkpoint.chat;
    let conversation_id = &snapshot.conversation_id;
    let user_message_id = &snapshot.user_message_id;
    let assistant_message_id = &snapshot.assistant_message_id;
    let user_ordinal = sqlite_u64(chat.user_ordinal, "user ordinal")?;
    let assistant_ordinal = sqlite_u64(chat.assistant_ordinal, "assistant ordinal")?;
    let deleted: bool = transaction
        .query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM conversation_deletions WHERE conversation_id = ?1
             )",
            [conversation_id.as_str()],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if deleted {
        return transaction.commit().map_err(sql_error);
    }
    transaction
        .execute(
            "INSERT INTO conversations(id, title, model_preference, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 model_preference = excluded.model_preference,
                 updated_at_ms = excluded.updated_at_ms",
            params![
                conversation_id.as_str(),
                chat.conversation_title,
                chat.model_preference.persistence_name(),
                now
            ],
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
                user_message_id.as_str(),
                conversation_id.as_str(),
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
                 attempt_id, reply_to_message_id, sequence, retention_generation, updated_at_ms
             ) VALUES (?1, ?2, 'assistant', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 content = excluded.content,
                 status = excluded.status,
                 sequence = excluded.sequence,
                 retention_generation = excluded.retention_generation,
                 updated_at_ms = excluded.updated_at_ms
             WHERE excluded.sequence > messages.sequence
               AND excluded.retention_generation >= messages.retention_generation",
            params![
                assistant_message_id.as_str(),
                conversation_id.as_str(),
                assistant_ordinal,
                snapshot.output,
                status_name(snapshot.status),
                snapshot.invocation_id.as_str(),
                chat.attempt_id.as_str(),
                chat.reply_to_user_id.as_str(),
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
                 id, provider_id, model_id, conversation_id,
                 user_message_id, assistant_message_id, status, sequence, retention_generation,
                 error, started_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
             ON CONFLICT(id) DO UPDATE SET
                 status = excluded.status,
                 sequence = excluded.sequence,
                 retention_generation = excluded.retention_generation,
                 error = excluded.error,
                 updated_at_ms = excluded.updated_at_ms
             WHERE excluded.sequence > executions.sequence
               AND excluded.retention_generation >= executions.retention_generation",
            params![
                snapshot.invocation_id.as_str(),
                snapshot.provider_id.as_str(),
                snapshot.model_id,
                conversation_id.as_str(),
                user_message_id.as_str(),
                assistant_message_id.as_str(),
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

fn toggle_favorite(connection: &mut Connection, invocation_id: &str) -> Result<bool, StorageError> {
    let transaction = connection.transaction().map_err(sql_error)?;
    let exists: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM favorites WHERE invocation_id = ?1)",
            [invocation_id],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if exists {
        transaction
            .execute(
                "DELETE FROM favorites WHERE invocation_id = ?1",
                [invocation_id],
            )
            .map_err(sql_error)?;
    } else {
        let execution_exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM executions WHERE id = ?1)",
                [invocation_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if !execution_exists {
            return Err(StorageError::Sql(
                "cannot favorite an execution that has not been saved".into(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO favorites(invocation_id, created_at_ms, note) VALUES (?1, ?2, '')",
                params![invocation_id, now_ms()],
            )
            .map_err(sql_error)?;
    }
    transaction.commit().map_err(sql_error)?;
    Ok(!exists)
}

fn set_favorite(
    connection: &mut Connection,
    invocation_id: &str,
    favorite: bool,
    note: &str,
) -> Result<(), StorageError> {
    if note.chars().count() > 1_000 {
        return Err(StorageError::Sql(
            "favorite annotation must not exceed 1000 characters".into(),
        ));
    }
    let transaction = connection.transaction().map_err(sql_error)?;
    if favorite {
        let exists: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM executions WHERE id = ?1)",
                [invocation_id],
                |row| row.get(0),
            )
            .map_err(sql_error)?;
        if !exists {
            return Err(StorageError::Sql(
                "cannot favorite an execution that has not been saved".into(),
            ));
        }
        transaction
            .execute(
                "INSERT INTO favorites(invocation_id, created_at_ms, note)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(invocation_id) DO UPDATE SET note = excluded.note",
                params![invocation_id, now_ms(), note],
            )
            .map_err(sql_error)?;
    } else {
        transaction
            .execute(
                "DELETE FROM favorites WHERE invocation_id = ?1",
                [invocation_id],
            )
            .map_err(sql_error)?;
    }
    transaction.commit().map_err(sql_error)
}

fn query_history(
    connection: &Connection,
    query: &HistoryQuery,
) -> Result<HistoryPage, StorageError> {
    let limit = query.limit.clamp(1, 100);
    let search = format!("%{}%", query.search.trim());
    let status = query.status.map(status_name);
    let cursor_time = query.cursor.as_ref().map(|cursor| cursor.updated_at_ms);
    let cursor_id = query
        .cursor
        .as_ref()
        .map(|cursor| cursor.invocation_id.as_str());
    let mut statement = connection
        .prepare(
            "SELECT e.id, c.title, substr(am.content, 1, 240), e.status,
                e.provider_id, e.model_id, e.updated_at_ms,
                CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END,
                coalesce(f.note, '')
         FROM executions e
         JOIN conversations c ON c.id = e.conversation_id
         JOIN messages um ON um.id = e.user_message_id
         JOIN messages am ON am.id = e.assistant_message_id
         LEFT JOIN favorites f ON f.invocation_id = e.id
         WHERE (?1 = '%%' OR c.title LIKE ?1 OR um.content LIKE ?1 OR am.content LIKE ?1)
           AND (?2 IS NULL OR e.status = ?2)
           AND (?3 = 0 OR f.invocation_id IS NOT NULL)
           AND (?4 IS NULL OR e.updated_at_ms < ?4 OR (e.updated_at_ms = ?4 AND e.id > ?5))
         ORDER BY e.updated_at_ms DESC, e.id ASC LIMIT ?6",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                search,
                status,
                query.favorites_only,
                cursor_time,
                cursor_id,
                (limit + 1) as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(sql_error)?;
    let mut items = rows
        .map(|row| history_item(row.map_err(sql_error)?))
        .collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    items.truncate(limit);
    let next_cursor = has_more
        .then(|| {
            items.last().map(|item| HistoryCursor {
                updated_at_ms: item.updated_at_ms,
                invocation_id: item.invocation_id.to_string(),
            })
        })
        .flatten();
    Ok(HistoryPage { items, next_cursor })
}

fn history_detail(
    connection: &Connection,
    invocation_id: &str,
) -> Result<Option<HistoryDetail>, StorageError> {
    let row = connection
        .query_row(
            "SELECT e.id, c.title, substr(am.content, 1, 240), e.status,
                e.provider_id, e.model_id, e.updated_at_ms,
                CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END,
                coalesce(f.note, ''), um.content, am.content
         FROM executions e
         JOIN conversations c ON c.id = e.conversation_id
         JOIN messages um ON um.id = e.user_message_id
         JOIN messages am ON am.id = e.assistant_message_id
         LEFT JOIN favorites f ON f.invocation_id = e.id
         WHERE e.id = ?1",
            [invocation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    row.map(
        |(id, title, preview, status, provider, model, updated, favorite, note, input, output)| {
            let item = history_item((
                id, title, preview, status, provider, model, updated, favorite, note,
            ))?;
            Ok(HistoryDetail {
                item,
                input,
                output,
            })
        },
    )
    .transpose()
}

#[allow(clippy::type_complexity)]
fn history_item(
    (id, title, preview, status, provider_id, model_id, updated_at_ms, favorite, favorite_note): (
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        bool,
        String,
    ),
) -> Result<HistoryItem, StorageError> {
    Ok(HistoryItem {
        invocation_id: InvocationId::parse(id).map_err(StorageError::Sql)?,
        title,
        preview,
        status: parse_status(&status),
        provider_id,
        model_id,
        updated_at_ms,
        favorite,
        favorite_note,
    })
}

fn delete_history(connection: &mut Connection, invocation_id: &str) -> Result<(), StorageError> {
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO execution_deletions(invocation_id, deleted_at_ms) VALUES (?1, ?2)
         ON CONFLICT(invocation_id) DO UPDATE SET deleted_at_ms = excluded.deleted_at_ms",
            params![invocation_id, now_ms()],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "DELETE FROM favorites WHERE invocation_id = ?1",
            [invocation_id],
        )
        .map_err(sql_error)?;
    let chat: Option<(String, String, String)> = transaction.query_row(
        "SELECT conversation_id, user_message_id, assistant_message_id FROM executions WHERE id = ?1",
        [invocation_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).optional().map_err(sql_error)?;
    transaction
        .execute("DELETE FROM executions WHERE id = ?1", [invocation_id])
        .map_err(sql_error)?;
    if let Some((conversation, user, assistant)) = chat {
        transaction
            .execute("DELETE FROM messages WHERE id = ?1", [assistant])
            .map_err(sql_error)?;
        transaction.execute(
            "DELETE FROM messages WHERE id = ?1 AND NOT EXISTS(SELECT 1 FROM executions WHERE user_message_id = ?1)",
            [user],
        ).map_err(sql_error)?;
        transaction.execute(
            "DELETE FROM conversations WHERE id = ?1 AND NOT EXISTS(SELECT 1 FROM executions WHERE conversation_id = ?1)",
            [conversation],
        ).map_err(sql_error)?;
    }
    transaction.commit().map_err(sql_error)
}

fn clear_history(
    connection: &mut Connection,
    mode: ClearHistoryMode,
    generation: u64,
) -> Result<(), StorageError> {
    let generation = sqlite_u64(generation, "retention generation")?;
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute(
            "UPDATE retention_state SET generation = ?1 WHERE singleton = 1",
            [generation],
        )
        .map_err(sql_error)?;
    match mode {
        ClearHistoryMode::PreserveFavorites => {
            transaction
                .execute(
                    "DELETE FROM executions WHERE id NOT IN (SELECT invocation_id FROM favorites)",
                    [],
                )
                .map_err(sql_error)?;
            transaction.execute("DELETE FROM conversations WHERE id NOT IN (SELECT conversation_id FROM executions)", []).map_err(sql_error)?;
        }
        ClearHistoryMode::IncludeFavorites => {
            transaction
                .execute("DELETE FROM favorites", [])
                .map_err(sql_error)?;
            transaction
                .execute("DELETE FROM conversations", [])
                .map_err(sql_error)?;
        }
    }
    transaction
        .execute("DELETE FROM execution_deletions", [])
        .map_err(sql_error)?;
    transaction
        .execute("DELETE FROM conversation_deletions", [])
        .map_err(sql_error)?;
    transaction.commit().map_err(sql_error)
}

fn contains_favorite(connection: &Connection, invocation_id: &str) -> Result<bool, StorageError> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM favorites WHERE invocation_id = ?1)",
            [invocation_id],
            |row| row.get(0),
        )
        .map_err(sql_error)
}

fn save_conversation(
    connection: &Connection,
    conversation_id: &str,
    title: &str,
    model_preference: &str,
) -> Result<(), StorageError> {
    let now = now_ms();
    connection
        .execute(
            "DELETE FROM conversation_deletions WHERE conversation_id = ?1",
            [conversation_id],
        )
        .map_err(sql_error)?;
    connection
        .execute(
            "INSERT INTO conversations(id, title, model_preference, created_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 title = excluded.title,
                 model_preference = excluded.model_preference,
                 updated_at_ms = excluded.updated_at_ms",
            params![conversation_id, title, model_preference, now],
        )
        .map_err(sql_error)?;
    Ok(())
}

fn delete_conversation(
    connection: &mut Connection,
    conversation_id: &str,
) -> Result<(), StorageError> {
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO conversation_deletions(conversation_id, deleted_at_ms)
             VALUES (?1, ?2)
             ON CONFLICT(conversation_id) DO UPDATE SET deleted_at_ms = excluded.deleted_at_ms",
            params![conversation_id, now_ms()],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "DELETE FROM favorites WHERE invocation_id IN (
                 SELECT id FROM executions WHERE conversation_id = ?1
             )",
            [conversation_id],
        )
        .map_err(sql_error)?;
    transaction
        .execute("DELETE FROM conversations WHERE id = ?1", [conversation_id])
        .map_err(sql_error)?;
    transaction.commit().map_err(sql_error)
}

fn load_conversations(
    connection: &Connection,
) -> Result<Vec<PersistedChatConversation>, StorageError> {
    let mut conversations = connection
        .prepare(
            "SELECT id, title, model_preference, favorite, updated_at_ms
             FROM conversations ORDER BY updated_at_ms DESC, id ASC",
        )
        .map_err(sql_error)?;
    let rows = conversations
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .map_err(sql_error)?;
    let mut restored = Vec::new();
    for row in rows {
        let (id, title, model_preference, favorite, updated_at_ms) = row.map_err(sql_error)?;
        let conversation_id = ConversationId::parse(id).map_err(StorageError::Sql)?;
        let mut statement = connection
            .prepare(
                "SELECT id, role, ordinal, content, status, attempt_id, reply_to_message_id
                 FROM messages WHERE conversation_id = ?1 ORDER BY ordinal ASC, id ASC",
            )
            .map_err(sql_error)?;
        let messages = statement
            .query_map([conversation_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(sql_error)?
            .map(|row| {
                let (id, role, ordinal, content, status, attempt_id, reply_to_message_id) =
                    row.map_err(sql_error)?;
                Ok(ChatMessageSnapshot {
                    id: MessageId::parse(id).map_err(StorageError::Sql)?,
                    is_user: role == "user",
                    content,
                    attachments: Arc::new(Vec::new()),
                    status: ChatMessageStatus::from_persistence_name(&status)
                        .unwrap_or(ChatMessageStatus::FailedPartial),
                    ordinal: ordinal
                        .try_into()
                        .map_err(|_| StorageError::Sql("message ordinal is negative".into()))?,
                    attempt_id: attempt_id
                        .map(AttemptId::parse)
                        .transpose()
                        .map_err(StorageError::Sql)?,
                    reply_to_user_id: reply_to_message_id
                        .map(MessageId::parse)
                        .transpose()
                        .map_err(StorageError::Sql)?,
                })
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        restored.push(PersistedChatConversation::new(
            conversation_id,
            title,
            ChatModelPreference::from_persistence_name(&model_preference),
            favorite,
            messages,
            updated_at_ms.max(0) as u64,
        ));
    }
    Ok(restored)
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

fn parse_status(status: &str) -> ExecutionStatus {
    match status {
        "queued" => ExecutionStatus::Queued,
        "running" => ExecutionStatus::Running,
        "cancelling" => ExecutionStatus::Cancelling,
        "completed" => ExecutionStatus::Completed,
        "failed" => ExecutionStatus::Failed,
        "cancelled" => ExecutionStatus::Cancelled,
        _ => ExecutionStatus::Interrupted,
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
    use super::*;
    use lexwisp_core::{
        AttemptId, ChatCheckpoint, ChatHistoryPort, ChatModelPreference, ConversationId,
        ExecutionSnapshot, InvocationId, MessageId, ProviderId, StorageState,
    };
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn path() -> PathBuf {
        std::env::temp_dir()
            .join("lexwisp-storage-tests")
            .join(format!(
                "{}-{}.db",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
    }

    fn checkpoint(
        conversation: ConversationId,
        user: MessageId,
        assistant: MessageId,
        run: InvocationId,
        sequence: u64,
        status: ExecutionStatus,
        output: &str,
    ) -> ExecutionCheckpoint {
        ExecutionCheckpoint {
            snapshot: ExecutionSnapshot {
                invocation_id: run,
                conversation_id: conversation,
                user_message_id: user.clone(),
                assistant_message_id: assistant,
                provider_id: ProviderId::parse("fixture").expect("provider ID"),
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
            chat: ChatCheckpoint {
                conversation_title: "hello".into(),
                model_preference: ChatModelPreference::Fast,
                user_ordinal: 0,
                assistant_ordinal: 1,
                attempt_id: AttemptId::new(),
                reply_to_user_id: user,
            },
        }
    }

    #[test]
    fn fresh_database_has_only_chat_schema() {
        let path = path();
        let connection = open(&path).expect("new database opens");
        let version: i64 = connection
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, 7);
        for retired in [
            "installed_plugins",
            "plugin_grants",
            "plugin_kv",
            "action_executions",
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
                    [retired],
                    |row| row.get(0),
                )
                .expect("table lookup");
            assert!(!exists, "{retired} must not be created");
        }
        drop(connection);
        std::fs::remove_file(path).expect("test database is removable");
    }

    #[test]
    fn checkpoints_restore_chat_and_preserve_terminal_sequence() {
        let path = path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("storage starts");
        let conversation = ConversationId::new();
        let user = MessageId::new();
        let assistant = MessageId::new();
        let run = InvocationId::new();
        let partial = checkpoint(
            conversation.clone(),
            user.clone(),
            assistant.clone(),
            run.clone(),
            2,
            ExecutionStatus::Running,
            "partial",
        );
        let terminal = checkpoint(
            conversation.clone(),
            user,
            assistant,
            run.clone(),
            3,
            ExecutionStatus::Completed,
            "complete",
        );
        store
            .enqueue(partial.clone(), true)
            .expect("checkpoint queues")
            .expect("receipt")
            .wait()
            .expect("partial saves");
        store
            .enqueue(terminal, true)
            .expect("checkpoint queues")
            .expect("receipt")
            .wait()
            .expect("terminal saves");
        store
            .enqueue(partial, true)
            .expect("checkpoint queues")
            .expect("receipt")
            .wait()
            .expect("stale write ignored");
        let restored = store.restore().expect("conversation restores");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].messages().len(), 2);
        assert_eq!(restored[0].messages()[1].content, "complete");
        store
            .set_conversation_favorite(&conversation, true)
            .expect("favorite queues");
        let restored = store.restore().expect("favorite restores");
        assert!(restored[0].favorite());
        let detail = store
            .history_detail(run.as_str())
            .expect("history loads")
            .expect("run exists");
        assert_eq!(detail.output, "complete");
        owner.shutdown();
        let reopened = open(&path).expect("completed conversation reopens");
        let user_status: String = reopened
            .query_row(
                "SELECT status FROM messages WHERE role = 'user'",
                [],
                |row| row.get(0),
            )
            .expect("user message remains available");
        assert_eq!(user_status, "submitted");
        reopened
            .execute(
                "UPDATE messages SET status = 'interrupted' WHERE role = 'user'",
                [],
            )
            .expect("simulate an older startup marking a user message interrupted");
        drop(reopened);
        let repaired = open(&path).expect("older user status is repaired");
        let user_status: String = repaired
            .query_row(
                "SELECT status FROM messages WHERE role = 'user'",
                [],
                |row| row.get(0),
            )
            .expect("user message remains available");
        assert_eq!(user_status, "submitted");
        drop(repaired);
        std::fs::remove_file(path).expect("test database is removable");
    }

    #[test]
    fn v6_upgrade_keeps_chat_rows_and_retires_package_tables() {
        let path = path();
        std::fs::create_dir_all(path.parent().expect("bounded test path")).expect("directory");
        let connection = Connection::open(&path).expect("fixture opens");
        connection.execute_batch(
            "CREATE TABLE schema_version(version INTEGER NOT NULL);
             INSERT INTO schema_version VALUES (6);
             CREATE TABLE conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL,
               model_preference TEXT NOT NULL, created_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             CREATE TABLE messages(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL,
               role TEXT NOT NULL, ordinal INTEGER NOT NULL, content TEXT NOT NULL, status TEXT NOT NULL,
               invocation_id TEXT, attempt_id TEXT, reply_to_message_id TEXT,
               sequence INTEGER NOT NULL, retention_generation INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL, UNIQUE(conversation_id, ordinal));
             CREATE TABLE executions(id TEXT PRIMARY KEY, plugin_id TEXT NOT NULL,
               action_id TEXT NOT NULL, plugin_generation INTEGER NOT NULL,
               provider_id TEXT NOT NULL, model_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
               user_message_id TEXT NOT NULL, assistant_message_id TEXT NOT NULL, status TEXT NOT NULL,
               sequence INTEGER NOT NULL, retention_generation INTEGER NOT NULL,
               error TEXT, started_at_ms INTEGER NOT NULL, updated_at_ms INTEGER NOT NULL);
             INSERT INTO conversations VALUES
               ('11111111-1111-4111-8111-111111111111', 'saved chat', 'profile:fast', 1, 2);
             INSERT INTO messages VALUES
               ('22222222-2222-4222-8222-222222222222', '11111111-1111-4111-8111-111111111111',
                'user', 0, 'hello', 'submitted', NULL, NULL, NULL, 0, 0, 2),
               ('33333333-3333-4333-8333-333333333333', '11111111-1111-4111-8111-111111111111',
                'assistant', 1, 'saved answer', 'completed',
                '44444444-4444-4444-8444-444444444444', NULL,
                '22222222-2222-4222-8222-222222222222', 2, 0, 2);
             INSERT INTO executions VALUES
               ('44444444-4444-4444-8444-444444444444', 'org.lexwisp.chat',
                'org.lexwisp.chat/ask', 1, 'default', 'model',
                '11111111-1111-4111-8111-111111111111',
                '22222222-2222-4222-8222-222222222222',
                '33333333-3333-4333-8333-333333333333',
                'completed', 2, 0, NULL, 1, 2);"
        ).expect("legacy fixture schema");
        drop(connection);
        let connection = open(&path).expect("legacy database migrates");
        let version: i64 = connection
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .expect("version");
        assert_eq!(version, 7);
        let answer: String = connection
            .query_row(
                "SELECT content FROM messages WHERE role = 'assistant'",
                [],
                |row| row.get(0),
            )
            .expect("chat answer kept");
        assert_eq!(answer, "saved answer");
        let run_count: i64 = connection
            .query_row("SELECT count(*) FROM executions", [], |row| row.get(0))
            .expect("run count");
        assert_eq!(run_count, 1);
        let retired: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = 'action_executions')",
                [],
                |row| row.get(0),
            )
            .expect("retired table lookup");
        assert!(!retired);
        drop(connection);
        std::fs::remove_file(path).expect("test database is removable");
    }

    #[test]
    fn deleted_conversation_rejects_late_checkpoint() {
        let path = path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("storage starts");
        let conversation = ConversationId::new();
        let late = checkpoint(
            conversation.clone(),
            MessageId::new(),
            MessageId::new(),
            InvocationId::new(),
            1,
            ExecutionStatus::Running,
            "late",
        );
        store
            .delete_conversation(&conversation)
            .expect("deletion queues");
        store
            .enqueue(late, true)
            .expect("checkpoint queues")
            .expect("receipt")
            .wait()
            .expect("late checkpoint ignored");
        assert!(store.restore().expect("restore").is_empty());
        owner.shutdown();
        std::fs::remove_file(path).expect("test database is removable");
    }
}

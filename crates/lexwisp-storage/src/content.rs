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
    PersistedChatConversation, QualifiedActionId,
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
    ListPlugins {
        reply: mpsc::Sender<Result<Vec<StoredPlugin>, StorageError>>,
    },
    SavePlugin {
        plugin: StoredPlugin,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    RemovePlugin {
        plugin_id: String,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    PluginKvGet {
        plugin_id: String,
        key: String,
        reply: mpsc::Sender<Result<Option<String>, StorageError>>,
    },
    PluginKvSet {
        plugin_id: String,
        key: String,
        value: String,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
    PluginKvDelete {
        plugin_id: String,
        key: String,
        reply: mpsc::Sender<Result<(), StorageError>>,
    },
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
    Shutdown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredPlugin {
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: String,
    pub package_hash: String,
    pub source_path: String,
    pub install_path: String,
    pub enabled: bool,
    pub generation: u64,
    pub capabilities: Vec<String>,
    pub last_error: Option<String>,
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
    pub fn list_plugins(&self) -> Result<Vec<StoredPlugin>, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::ListPlugins { reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn save_plugin(&self, plugin: StoredPlugin) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::SavePlugin { plugin, reply })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn remove_plugin(&self, plugin_id: &str) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::RemovePlugin {
                plugin_id: plugin_id.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn plugin_kv_get(
        &self,
        plugin_id: &str,
        key: &str,
    ) -> Result<Option<String>, StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::PluginKvGet {
                plugin_id: plugin_id.to_owned(),
                key: key.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn plugin_kv_set(
        &self,
        plugin_id: &str,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::PluginKvSet {
                plugin_id: plugin_id.to_owned(),
                key: key.to_owned(),
                value: value.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

    pub fn plugin_kv_delete(&self, plugin_id: &str, key: &str) -> Result<(), StorageError> {
        let (reply, response) = mpsc::channel();
        self.sender
            .send_blocking(StorageCommand::PluginKvDelete {
                plugin_id: plugin_id.to_owned(),
                key: key.to_owned(),
                reply,
            })
            .map_err(|_| StorageError::Closed)?;
        response.recv().unwrap_or(Err(StorageError::Closed))
    }

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
            StorageCommand::ListPlugins { reply } => {
                let _ = reply.send(list_plugins(&connection));
            }
            StorageCommand::SavePlugin { plugin, reply } => {
                let _ = reply.send(save_plugin(&mut connection, &plugin));
            }
            StorageCommand::RemovePlugin { plugin_id, reply } => {
                let _ = reply.send(remove_plugin(&mut connection, &plugin_id));
            }
            StorageCommand::PluginKvGet {
                plugin_id,
                key,
                reply,
            } => {
                let result = connection
                    .query_row(
                        "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                        params![plugin_id, key],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(sql_error);
                let _ = reply.send(result);
            }
            StorageCommand::PluginKvSet {
                plugin_id,
                key,
                value,
                reply,
            } => {
                let _ = reply.send(set_plugin_kv(&mut connection, &plugin_id, &key, &value));
            }
            StorageCommand::PluginKvDelete {
                plugin_id,
                key,
                reply,
            } => {
                let result = connection
                    .execute(
                        "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                        params![plugin_id, key],
                    )
                    .map(|_| ())
                    .map_err(sql_error);
                let _ = reply.send(result);
            }
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
                        "SELECT EXISTS(
                             SELECT 1 FROM executions WHERE id = ?1
                             UNION ALL
                             SELECT 1 FROM action_executions WHERE id = ?1
                         )",
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
    {
        return Err(StorageError::Start(format!(
            "database schema version {version} is not supported"
        )));
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
                 WHERE status IN ('submitted', 'generating');",
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
    connection
        .execute("UPDATE schema_version SET version = 6", [])
        .map_err(sql_error)?;
    Ok(connection)
}

fn list_plugins(connection: &Connection) -> Result<Vec<StoredPlugin>, StorageError> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, version, kind, package_hash, source_path, install_path, enabled,
                    generation, last_error
             FROM installed_plugins ORDER BY name COLLATE NOCASE, id",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, bool>(7)?,
                row.get::<_, i64>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })
        .map_err(sql_error)?;
    let mut plugins = Vec::new();
    for row in rows {
        let (
            id,
            name,
            version,
            kind,
            package_hash,
            source_path,
            install_path,
            enabled,
            generation,
            last_error,
        ) = row.map_err(sql_error)?;
        let mut grants = connection
            .prepare(
                "SELECT capability FROM plugin_grants
                 WHERE plugin_id = ?1 AND package_hash = ?2 AND generation = ?3
                 ORDER BY capability",
            )
            .map_err(sql_error)?;
        let capabilities = grants
            .query_map(params![id, package_hash, generation], |row| {
                row.get::<_, String>(0)
            })
            .map_err(sql_error)?
            .map(|value| value.map_err(sql_error))
            .collect::<Result<Vec<_>, _>>()?;
        plugins.push(StoredPlugin {
            id,
            name,
            version,
            kind,
            package_hash,
            source_path,
            install_path,
            enabled,
            generation: u64::try_from(generation)
                .map_err(|_| StorageError::Sql("negative plugin generation".into()))?,
            capabilities,
            last_error,
        });
    }
    Ok(plugins)
}

fn save_plugin(connection: &mut Connection, plugin: &StoredPlugin) -> Result<(), StorageError> {
    let generation = sqlite_u64(plugin.generation, "plugin generation")?;
    let transaction = connection.transaction().map_err(sql_error)?;
    transaction
        .execute(
            "INSERT INTO installed_plugins(
                 id, name, version, kind, package_hash, source_path, install_path, enabled,
                 generation, last_error, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
                 name = excluded.name,
                 version = excluded.version,
                 kind = excluded.kind,
                 package_hash = excluded.package_hash,
                 source_path = excluded.source_path,
                 install_path = excluded.install_path,
                 enabled = excluded.enabled,
                 generation = excluded.generation,
                 last_error = excluded.last_error,
                 updated_at_ms = excluded.updated_at_ms",
            params![
                plugin.id,
                plugin.name,
                plugin.version,
                plugin.kind,
                plugin.package_hash,
                plugin.source_path,
                plugin.install_path,
                plugin.enabled,
                generation,
                plugin.last_error,
                now_ms(),
            ],
        )
        .map_err(sql_error)?;
    transaction
        .execute(
            "DELETE FROM plugin_grants WHERE plugin_id = ?1",
            [&plugin.id],
        )
        .map_err(sql_error)?;
    for capability in &plugin.capabilities {
        transaction
            .execute(
                "INSERT INTO plugin_grants(plugin_id, package_hash, generation, capability)
                 VALUES (?1, ?2, ?3, ?4)",
                params![plugin.id, plugin.package_hash, generation, capability],
            )
            .map_err(sql_error)?;
    }
    transaction.commit().map_err(sql_error)
}

fn remove_plugin(connection: &mut Connection, plugin_id: &str) -> Result<(), StorageError> {
    connection
        .execute("DELETE FROM installed_plugins WHERE id = ?1", [plugin_id])
        .map_err(sql_error)?;
    Ok(())
}

fn set_plugin_kv(
    connection: &mut Connection,
    plugin_id: &str,
    key: &str,
    value: &str,
) -> Result<(), StorageError> {
    const PLUGIN_QUOTA: i64 = 5 * 1024 * 1024;
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(StorageError::Sql("plugin storage key is invalid".into()));
    }
    let value_size = i64::try_from(value.len())
        .map_err(|_| StorageError::Sql("plugin storage value is too large".into()))?;
    let transaction = connection.transaction().map_err(sql_error)?;
    let current_size: i64 = transaction
        .query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM plugin_kv
             WHERE plugin_id = ?1 AND key != ?2",
            params![plugin_id, key],
            |row| row.get(0),
        )
        .map_err(sql_error)?;
    if current_size.saturating_add(value_size) > PLUGIN_QUOTA {
        return Err(StorageError::Sql(
            "plugin storage quota exceeds 5 MiB".into(),
        ));
    }
    transaction
        .execute(
            "INSERT INTO plugin_kv(plugin_id, key, value, size_bytes, schema_version, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, 1, ?5)
             ON CONFLICT(plugin_id, key) DO UPDATE SET
                 value = excluded.value,
                 size_bytes = excluded.size_bytes,
                 schema_version = excluded.schema_version,
                 updated_at_ms = excluded.updated_at_ms",
            params![plugin_id, key, value, value_size, now_ms()],
        )
        .map_err(sql_error)?;
    transaction.commit().map_err(sql_error)
}

fn write_checkpoint(
    connection: &mut Connection,
    checkpoint: &ExecutionCheckpoint,
) -> Result<(), StorageError> {
    let snapshot = &checkpoint.snapshot;
    let plugin_generation = sqlite_u64(snapshot.plugin_generation, "plugin generation")?;
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
    let Some(chat) = &checkpoint.chat else {
        let started_at = transaction
            .query_row(
                "SELECT started_at_ms FROM action_executions WHERE id = ?1",
                [snapshot.invocation_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(sql_error)?
            .unwrap_or(now);
        transaction
            .execute(
                "INSERT INTO action_executions(
                     id, plugin_id, action_id, plugin_generation, provider_id, model_id,
                     input, output, status, sequence, retention_generation, error,
                     started_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(id) DO UPDATE SET
                     output = excluded.output,
                     status = excluded.status,
                     sequence = excluded.sequence,
                     retention_generation = excluded.retention_generation,
                     error = excluded.error,
                     updated_at_ms = excluded.updated_at_ms
                 WHERE excluded.sequence > action_executions.sequence
                   AND excluded.retention_generation >= action_executions.retention_generation",
                params![
                    snapshot.invocation_id.as_str(),
                    snapshot.plugin_id.as_str(),
                    snapshot.action.to_string(),
                    plugin_generation,
                    snapshot.provider_id.as_str(),
                    snapshot.model_id,
                    checkpoint.input,
                    snapshot.output,
                    status_name(snapshot.status),
                    sequence,
                    retention_generation,
                    snapshot.error,
                    started_at,
                    now,
                ],
            )
            .map_err(sql_error)?;
        return transaction.commit().map_err(sql_error);
    };
    let conversation_id = snapshot
        .conversation_id
        .as_ref()
        .ok_or_else(|| StorageError::Sql("chat checkpoint has no conversation ID".into()))?;
    let user_message_id = snapshot
        .user_message_id
        .as_ref()
        .ok_or_else(|| StorageError::Sql("chat checkpoint has no user message ID".into()))?;
    let assistant_message_id = snapshot
        .assistant_message_id
        .as_ref()
        .ok_or_else(|| StorageError::Sql("chat checkpoint has no assistant message ID".into()))?;
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
                 id, plugin_id, action_id, plugin_generation, provider_id, model_id, conversation_id,
                 user_message_id, assistant_message_id, status, sequence, retention_generation,
                 error, started_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
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
                snapshot.plugin_id.as_str(),
                snapshot.action.to_string(),
                plugin_generation,
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
                "SELECT EXISTS(
                    SELECT 1 FROM executions WHERE id = ?1
                    UNION ALL
                    SELECT 1 FROM action_executions WHERE id = ?1
                 )",
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
                "SELECT EXISTS(
                    SELECT 1 FROM executions WHERE id = ?1
                    UNION ALL SELECT 1 FROM action_executions WHERE id = ?1
                 )",
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
    let plugin = query.plugin_id.as_deref();
    let status = query.status.map(status_name);
    let cursor_time = query.cursor.as_ref().map(|cursor| cursor.updated_at_ms);
    let cursor_id = query
        .cursor
        .as_ref()
        .map(|cursor| cursor.invocation_id.as_str());
    let mut statement = connection
        .prepare(
            "WITH history AS (
                SELECT e.id, e.action_id, c.title, substr(am.content, 1, 240) AS preview,
                       e.status, e.provider_id, e.model_id, e.updated_at_ms,
                       CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END AS favorite,
                       coalesce(f.note, '') AS favorite_note, um.content AS input, am.content AS output,
                       e.plugin_id
                FROM executions e
                JOIN conversations c ON c.id = e.conversation_id
                JOIN messages um ON um.id = e.user_message_id
                JOIN messages am ON am.id = e.assistant_message_id
                LEFT JOIN favorites f ON f.invocation_id = e.id
                UNION ALL
                SELECT a.id, a.action_id, a.action_id, substr(a.output, 1, 240),
                       a.status, a.provider_id, a.model_id, a.updated_at_ms,
                       CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END,
                       coalesce(f.note, ''), a.input, a.output, a.plugin_id
                FROM action_executions a
                LEFT JOIN favorites f ON f.invocation_id = a.id
            )
            SELECT id, action_id, title, preview, status, provider_id, model_id,
                   updated_at_ms, favorite, favorite_note
            FROM history
            WHERE (?1 = '%%' OR title LIKE ?1 ESCAPE '\\' OR input LIKE ?1 ESCAPE '\\' OR output LIKE ?1 ESCAPE '\\')
              AND (?2 IS NULL OR plugin_id = ?2)
              AND (?3 IS NULL OR status = ?3)
              AND (?4 = 0 OR favorite = 1)
              AND (?5 IS NULL OR updated_at_ms < ?5 OR (updated_at_ms = ?5 AND id > ?6))
            ORDER BY updated_at_ms DESC, id ASC
            LIMIT ?7",
        )
        .map_err(sql_error)?;
    let rows = statement
        .query_map(
            params![
                search,
                plugin,
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
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, String>(9)?,
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
            "WITH history AS (
                SELECT e.id, e.action_id, c.title, substr(am.content, 1, 240), e.status,
                       e.provider_id, e.model_id, e.updated_at_ms,
                       CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END,
                       coalesce(f.note, ''), um.content, am.content
                FROM executions e JOIN conversations c ON c.id = e.conversation_id
                JOIN messages um ON um.id = e.user_message_id
                JOIN messages am ON am.id = e.assistant_message_id
                LEFT JOIN favorites f ON f.invocation_id = e.id WHERE e.id = ?1
                UNION ALL
                SELECT a.id, a.action_id, a.action_id, substr(a.output, 1, 240), a.status,
                       a.provider_id, a.model_id, a.updated_at_ms,
                       CASE WHEN f.invocation_id IS NULL THEN 0 ELSE 1 END,
                       coalesce(f.note, ''), a.input, a.output
                FROM action_executions a LEFT JOIN favorites f ON f.invocation_id = a.id
                WHERE a.id = ?1
            )
            SELECT * FROM history LIMIT 1",
            [invocation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .optional()
        .map_err(sql_error)?;
    row.map(
        |(
            id,
            action,
            title,
            preview,
            status,
            provider,
            model,
            updated,
            favorite,
            note,
            input,
            output,
        )| {
            let item = history_item((
                id, action, title, preview, status, provider, model, updated, favorite, note,
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
    row: (
        String,
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
    let (
        id,
        action,
        title,
        preview,
        status,
        provider_id,
        model_id,
        updated_at_ms,
        favorite,
        favorite_note,
    ) = row;
    Ok(HistoryItem {
        invocation_id: InvocationId::parse(id).map_err(StorageError::Sql)?,
        action: action
            .parse::<QualifiedActionId>()
            .map_err(StorageError::Sql)?,
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
    transaction
        .execute(
            "DELETE FROM action_executions WHERE id = ?1",
            [invocation_id],
        )
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
            transaction.execute("DELETE FROM action_executions WHERE id NOT IN (SELECT invocation_id FROM favorites)", []).map_err(sql_error)?;
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
                .execute("DELETE FROM action_executions", [])
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
            "SELECT id, title, model_preference, updated_at_ms
             FROM conversations ORDER BY updated_at_ms DESC, id ASC",
        )
        .map_err(sql_error)?;
    let rows = conversations
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(sql_error)?;
    let mut restored = Vec::new();
    for row in rows {
        let (id, title, model_preference, updated_at_ms) = row.map_err(sql_error)?;
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
    use std::{fs, sync::atomic::AtomicU64};

    use lexwisp_core::{
        ActionId, AttemptId, ChatCheckpoint, ChatModelPreference, ConversationId,
        ExecutionSnapshot, InvocationId, MessageId, PluginId, ProviderId, QualifiedActionId,
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
        let plugin_id = PluginId::parse("org.lexwisp.chat").expect("valid plugin ID");
        let conversation_id = ConversationId::new();
        let user_message_id = MessageId::new();
        let assistant_message_id = MessageId::new();
        ExecutionCheckpoint {
            snapshot: ExecutionSnapshot {
                invocation_id: InvocationId::new(),
                plugin_id: plugin_id.clone(),
                action: QualifiedActionId::new(
                    plugin_id,
                    ActionId::parse("ask").expect("action ID"),
                ),
                plugin_generation: 1,
                conversation_id: Some(conversation_id),
                user_message_id: Some(user_message_id.clone()),
                assistant_message_id: Some(assistant_message_id),
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
            chat: Some(ChatCheckpoint {
                conversation_title: "hello".into(),
                model_preference: ChatModelPreference::Fast,
                user_ordinal: 0,
                assistant_ordinal: 1,
                attempt_id: AttemptId::new(),
                reply_to_user_id: user_message_id,
            }),
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
                [terminal
                    .snapshot
                    .assistant_message_id
                    .as_ref()
                    .expect("assistant ID")
                    .as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("assistant message exists");
        assert_eq!(content, "complete");
        assert_eq!(status, "completed");
        drop(connection);
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn text_execution_and_favorite_are_persisted_without_chat_rows() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let mut text = checkpoint(2, "processed", ExecutionStatus::Completed);
        text.snapshot.action = QualifiedActionId::new(
            PluginId::parse("org.example.action").expect("plugin ID"),
            ActionId::parse("run").expect("action ID"),
        );
        text.snapshot.plugin_id = text.snapshot.action.plugin_id().clone();
        text.snapshot.conversation_id = None;
        text.snapshot.user_message_id = None;
        text.snapshot.assistant_message_id = None;
        text.chat = None;
        let invocation = text.snapshot.invocation_id.clone();
        store
            .enqueue(text, true)
            .expect("terminal enqueues")
            .expect("terminal receipt")
            .wait()
            .expect("text execution persists");
        assert!(
            store
                .toggle_favorite(invocation.as_str())
                .expect("favorite toggles")
        );
        assert!(
            store
                .contains_favorite(invocation.as_str())
                .expect("favorite can be read")
        );
        assert!(
            store
                .contains_execution(invocation.as_str())
                .expect("persisted action execution can be found")
        );
        assert!(
            !store
                .contains_execution(InvocationId::new().as_str())
                .expect("unknown execution is absent")
        );
        owner.shutdown();

        let connection = Connection::open(&path).expect("database opens");
        let (input, output): (String, String) = connection
            .query_row(
                "SELECT input, output FROM action_executions WHERE id = ?1",
                [invocation.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("action execution exists");
        assert_eq!(input, "hello");
        assert_eq!(output, "processed");
        drop(connection);
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn conversations_restore_attempts_and_model_preference() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let mut terminal = checkpoint(2, "restored answer", ExecutionStatus::Completed);
        terminal
            .chat
            .as_mut()
            .expect("chat checkpoint")
            .model_preference = ChatModelPreference::Smart;
        let expected_conversation = terminal
            .snapshot
            .conversation_id
            .as_ref()
            .expect("conversation ID")
            .clone();
        let expected_attempt = terminal
            .chat
            .as_ref()
            .expect("chat checkpoint")
            .attempt_id
            .clone();
        store
            .enqueue(terminal, true)
            .expect("terminal enqueues")
            .expect("terminal receipt")
            .wait()
            .expect("terminal persists");
        owner.shutdown();

        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store reopens");
        let restored = ChatHistoryPort::restore(&store).expect("conversation restores");
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id(), &expected_conversation);
        assert_eq!(restored[0].model_preference(), &ChatModelPreference::Smart);
        assert_eq!(restored[0].messages().len(), 2);
        assert_eq!(
            restored[0].messages()[1].attempt_id.as_ref(),
            Some(&expected_attempt)
        );
        assert_eq!(restored[0].messages()[1].content, "restored answer");
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn deletion_barrier_rejects_late_chat_checkpoints() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let terminal = checkpoint(2, "answer", ExecutionStatus::Completed);
        let conversation_id = terminal
            .snapshot
            .conversation_id
            .as_ref()
            .expect("conversation ID")
            .clone();
        store
            .enqueue(terminal.clone(), true)
            .expect("terminal enqueues")
            .expect("terminal receipt")
            .wait()
            .expect("terminal persists");
        ChatHistoryPort::delete_conversation(&store, &conversation_id)
            .expect("conversation deletes");
        let mut late = terminal;
        late.snapshot.sequence = 3;
        late.snapshot.output = "late".into();
        store
            .enqueue(late, true)
            .expect("late checkpoint enqueues")
            .expect("late checkpoint receipt")
            .wait()
            .expect("late checkpoint is safely ignored");
        assert!(
            ChatHistoryPort::restore(&store)
                .expect("restore succeeds")
                .is_empty()
        );
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn newer_database_is_rejected_before_schema_changes() {
        let path = test_path();
        fs::create_dir_all(path.parent().expect("database parent")).expect("parent exists");
        let connection = Connection::open(&path).expect("fixture database opens");
        connection
            .execute_batch(
                "CREATE TABLE schema_version(version INTEGER NOT NULL);
                 INSERT INTO schema_version(version) VALUES (99);",
            )
            .expect("fixture schema is created");
        drop(connection);

        assert!(matches!(
            ContentStoreOwner::start(path.clone()),
            Err(StorageError::Start(_))
        ));
        let connection = Connection::open(&path).expect("fixture database reopens");
        let version: i64 = connection
            .query_row("SELECT version FROM schema_version", [], |row| row.get(0))
            .expect("version remains");
        let action_table: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'action_executions')",
                [],
                |row| row.get(0),
            )
            .expect("catalog query works");
        assert_eq!(version, 99);
        assert!(!action_table);
        drop(connection);
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn ten_thousand_history_rows_use_stable_cursor_pages() {
        use std::collections::HashSet;

        let path = test_path();
        let mut connection = open(&path).expect("database opens");
        let transaction = connection.transaction().expect("transaction starts");
        for index in 0..10_005_i64 {
            let id = InvocationId::new();
            transaction
                .execute(
                    "INSERT INTO action_executions(
                        id, plugin_id, action_id, plugin_generation, provider_id, model_id,
                        input, output, status, sequence, retention_generation, error,
                        started_at_ms, updated_at_ms
                     ) VALUES (?1, 'org.example.action', 'org.example.action/run', 1,
                               'fixture', 'fixture', ?2, ?3, 'completed', 2, 0, NULL, ?4, ?4)",
                    params![
                        id.as_str(),
                        format!("input {index}"),
                        format!("output {index}"),
                        index
                    ],
                )
                .expect("row inserts");
        }
        transaction.commit().expect("fixture commits");
        drop(connection);

        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let mut cursor = None;
        let mut seen = HashSet::new();
        loop {
            let page = store
                .query_history(HistoryQuery {
                    cursor: cursor.clone(),
                    limit: 73,
                    ..HistoryQuery::default()
                })
                .expect("page loads");
            assert!(page.items.len() <= 73);
            for item in page.items {
                assert!(seen.insert(item.invocation_id));
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen.len(), 10_005);
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn delete_and_clear_barriers_reject_late_action_checkpoints() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let mut first = checkpoint(2, "first", ExecutionStatus::Completed);
        first.chat = None;
        first.snapshot.conversation_id = None;
        first.snapshot.user_message_id = None;
        first.snapshot.assistant_message_id = None;
        let first_id = first.snapshot.invocation_id.clone();
        store
            .enqueue(first.clone(), true)
            .expect("checkpoint enqueues")
            .expect("receipt")
            .wait()
            .expect("checkpoint persists");
        store
            .delete_history(first_id.as_str())
            .expect("history deletes");
        first.snapshot.sequence = 3;
        first.snapshot.output = "late delete".into();
        store
            .enqueue(first, true)
            .expect("late checkpoint enqueues")
            .expect("receipt")
            .wait()
            .expect("late checkpoint is ignored");
        assert!(
            store
                .history_detail(first_id.as_str())
                .expect("detail query")
                .is_none()
        );

        let mut second = checkpoint(2, "second", ExecutionStatus::Completed);
        second.chat = None;
        second.snapshot.conversation_id = None;
        second.snapshot.user_message_id = None;
        second.snapshot.assistant_message_id = None;
        let second_id = second.snapshot.invocation_id.clone();
        store
            .enqueue(second.clone(), true)
            .expect("checkpoint enqueues")
            .expect("receipt")
            .wait()
            .expect("checkpoint persists");
        store
            .set_favorite(second_id.as_str(), true, "keep")
            .expect("favorite saves");
        store
            .clear_history(ClearHistoryMode::PreserveFavorites, 1)
            .expect("history clears");
        let page = store
            .query_history(HistoryQuery {
                limit: 10,
                ..HistoryQuery::default()
            })
            .expect("history loads");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].favorite_note, "keep");
        second.snapshot.sequence = 3;
        second.snapshot.output = "late clear".into();
        store
            .enqueue(second, true)
            .expect("late checkpoint enqueues")
            .expect("receipt")
            .wait()
            .expect("old generation is ignored");
        let detail = store
            .history_detail(second_id.as_str())
            .expect("detail loads")
            .expect("favorite remains");
        assert_eq!(detail.output, "second");
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn plugin_grants_round_trip_with_hash_and_generation() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        let expected = StoredPlugin {
            id: "org.example.fixture".into(),
            name: "Fixture".into(),
            version: "1.2.3".into(),
            kind: "declarative".into(),
            package_hash: "abc123".into(),
            source_path: r"C:\fixtures\插件".into(),
            install_path: r"C:\data\plugins\fixture".into(),
            enabled: true,
            generation: 9,
            capabilities: vec!["ai.invoke".into()],
            last_error: None,
        };
        store.save_plugin(expected.clone()).expect("plugin saves");
        assert_eq!(store.list_plugins().expect("plugins load"), vec![expected]);
        store
            .remove_plugin("org.example.fixture")
            .expect("plugin removes");
        assert!(store.list_plugins().expect("plugins load").is_empty());
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }

    #[test]
    fn plugin_kv_is_namespaced_and_quota_bounded() {
        let path = test_path();
        let (owner, store) = ContentStoreOwner::start(path.clone()).expect("store starts");
        for id in ["org.example.one", "org.example.two"] {
            store
                .save_plugin(StoredPlugin {
                    id: id.into(),
                    name: id.into(),
                    version: "1.0.0".into(),
                    kind: "script".into(),
                    package_hash: "hash".into(),
                    source_path: "source".into(),
                    install_path: "install".into(),
                    enabled: true,
                    generation: 1,
                    capabilities: vec!["storage.read".into(), "storage.write".into()],
                    last_error: None,
                })
                .expect("plugin saves");
        }
        store
            .plugin_kv_set("org.example.one", "shared", "one")
            .expect("first namespace writes");
        store
            .plugin_kv_set("org.example.two", "shared", "two")
            .expect("second namespace writes");
        assert_eq!(
            store
                .plugin_kv_get("org.example.one", "shared")
                .expect("first namespace reads")
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            store
                .plugin_kv_get("org.example.two", "shared")
                .expect("second namespace reads")
                .as_deref(),
            Some("two")
        );
        let oversized = "x".repeat(5 * 1024 * 1024 + 1);
        assert!(
            store
                .plugin_kv_set("org.example.one", "too-large", &oversized)
                .is_err()
        );
        owner.shutdown();
        fs::remove_dir_all(path.parent().expect("bounded test directory"))
            .expect("test directory is removable");
    }
}

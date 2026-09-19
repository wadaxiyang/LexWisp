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
    ChatModelPreference, ConversationId, ExecutionCheckpoint, ExecutionStatus, MessageId,
    PersistedChatConversation,
};
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
    ToggleFavorite {
        invocation_id: String,
        reply: mpsc::Sender<Result<bool, StorageError>>,
    },
    ContainsFavorite {
        invocation_id: String,
        reply: mpsc::Sender<Result<bool, StorageError>>,
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
                 created_at_ms INTEGER NOT NULL
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
    connection
        .execute("UPDATE schema_version SET version = 3", [])
        .map_err(sql_error)?;
    Ok(connection)
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
                     error = excluded.error,
                     updated_at_ms = excluded.updated_at_ms
                 WHERE excluded.sequence > action_executions.sequence
                   AND excluded.retention_generation = action_executions.retention_generation",
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
                 updated_at_ms = excluded.updated_at_ms
             WHERE excluded.sequence > messages.sequence
               AND excluded.retention_generation = messages.retention_generation",
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
                "INSERT INTO favorites(invocation_id, created_at_ms) VALUES (?1, ?2)",
                params![invocation_id, now_ms()],
            )
            .map_err(sql_error)?;
    }
    transaction.commit().map_err(sql_error)?;
    Ok(!exists)
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
        let mut text = checkpoint(2, "translated", ExecutionStatus::Completed);
        text.snapshot.action = QualifiedActionId::new(
            PluginId::parse("org.lexwisp.translate").expect("plugin ID"),
            ActionId::parse("translate").expect("action ID"),
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
        assert_eq!(output, "translated");
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
}

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, Weak},
};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{
    AiMessage, AiRole, AttemptId, ChatAttachmentContent, ChatConversationSummary, ChatDraft,
    ChatError, ChatHistoryPort, ChatInvocationRequest, ChatMessageSnapshot, ChatMessageStatus,
    ChatModelPreference, ChatRunPort, ChatSnapshot, ChatUiPort, ChatUiResultFuture, ConversationId,
    ExecutionObserver, ExecutionSnapshot, ExecutionStatus, InvocationId, MessageId,
    PersistedChatConversation, StorageState, SurfaceKind,
};

const ESTIMATED_CHARS_PER_TOKEN: usize = 4;

struct ConversationState {
    id: ConversationId,
    title: String,
    model_preference: ChatModelPreference,
    favorite: bool,
    messages: Arc<Vec<Arc<ChatMessageSnapshot>>>,
    active_invocation: Option<InvocationId>,
    active_assistant: Option<MessageId>,
    last_execution_sequence: u64,
    starting: bool,
    status_text: String,
    has_unsaved_result: bool,
    context_notice: Option<String>,
    updated_order: u64,
}

impl ConversationState {
    fn empty(id: ConversationId, updated_order: u64) -> Self {
        Self {
            id,
            title: "New chat".into(),
            model_preference: ChatModelPreference::Fast,
            favorite: false,
            messages: Arc::new(Vec::new()),
            active_invocation: None,
            active_assistant: None,
            last_execution_sequence: 0,
            starting: false,
            status_text: "Ready".into(),
            has_unsaved_result: false,
            context_notice: None,
            updated_order,
        }
    }

    fn restored(record: PersistedChatConversation) -> Self {
        Self {
            id: record.id().clone(),
            title: record.title().to_owned(),
            model_preference: record.model_preference().clone(),
            favorite: record.favorite(),
            messages: Arc::new(record.messages().iter().cloned().map(Arc::new).collect()),
            active_invocation: None,
            active_assistant: None,
            last_execution_sequence: 0,
            starting: false,
            status_text: "Restored".into(),
            has_unsaved_result: false,
            context_notice: None,
            updated_order: record.updated_order(),
        }
    }

    fn summary(&self) -> ChatConversationSummary {
        ChatConversationSummary::new(
            self.id.clone(),
            self.title.clone(),
            self.model_preference.clone(),
            self.starting || self.active_invocation.is_some(),
            self.favorite,
            self.updated_order,
        )
    }
}

struct ChatState {
    active_conversation: ConversationId,
    conversations: HashMap<ConversationId, ConversationState>,
    next_updated_order: u64,
    generation: u64,
    visible_surfaces: HashSet<SurfaceKind>,
    subscribers: Vec<ChatSubscriber>,
}

struct ChatSubscriber {
    sender: Sender<ChatSnapshot>,
    stale_receiver: Receiver<ChatSnapshot>,
}

impl ChatState {
    fn snapshot(&self) -> ChatSnapshot {
        let conversation = self
            .conversations
            .get(&self.active_conversation)
            .expect("the active conversation is always present");
        let active = conversation.starting || conversation.active_invocation.is_some();
        let mut summaries = self
            .conversations
            .values()
            .map(ConversationState::summary)
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .updated_order()
                .cmp(&left.updated_order())
                .then_with(|| left.id().cmp(right.id()))
        });
        ChatSnapshot {
            conversation_id: conversation.id.clone(),
            title: conversation.title.clone(),
            messages: conversation.messages.clone(),
            conversations: Arc::new(summaries),
            model_preference: conversation.model_preference.clone(),
            active_invocation: conversation.active_invocation.clone(),
            generation: self.generation,
            status_text: conversation.status_text.clone(),
            can_send: !active,
            can_stop: conversation.active_invocation.is_some(),
            can_retry: !active && last_user_message(conversation).is_some(),
            has_unsaved_result: conversation.has_unsaved_result,
            context_notice: conversation.context_notice.clone(),
        }
    }

    fn publish(&mut self) {
        self.generation = self.generation.saturating_add(1);
        if self.visible_surfaces.is_empty() {
            return;
        }
        self.broadcast(self.snapshot());
    }

    fn broadcast(&mut self, snapshot: ChatSnapshot) {
        self.subscribers.retain(|subscriber| {
            match subscriber.sender.try_send(snapshot.clone()) {
                Ok(()) => true,
                Err(TrySendError::Closed(_)) => false,
                Err(TrySendError::Full(_)) => {
                    // Keep a bounded latest-snapshot channel: evict one stale projection before
                    // retrying so a terminal state cannot be stranded behind streaming updates.
                    let _ = subscriber.stale_receiver.try_recv();
                    !matches!(
                        subscriber.sender.try_send(snapshot.clone()),
                        Err(TrySendError::Closed(_))
                    )
                }
            }
        });
    }

    fn touch(&mut self, conversation_id: &ConversationId) {
        self.next_updated_order = self.next_updated_order.saturating_add(1);
        if let Some(conversation) = self.conversations.get_mut(conversation_id) {
            conversation.updated_order = self.next_updated_order;
        }
    }
}

pub struct ChatController {
    self_weak: Weak<Self>,
    runner: Arc<dyn ChatRunPort>,
    history: Arc<dyn ChatHistoryPort>,
    state: Mutex<ChatState>,
}

impl ChatController {
    pub fn new(
        runner: Arc<dyn ChatRunPort>,
        history: Arc<dyn ChatHistoryPort>,
    ) -> Result<Arc<Self>, ChatError> {
        let restored = history.restore()?;
        let mut conversations = restored
            .into_iter()
            .map(|record| {
                let conversation = ConversationState::restored(record);
                (conversation.id.clone(), conversation)
            })
            .collect::<HashMap<_, _>>();
        let next_updated_order = conversations
            .values()
            .map(|conversation| conversation.updated_order)
            .max()
            .unwrap_or(0);
        let active_conversation = conversations
            .values()
            .max_by_key(|conversation| conversation.updated_order)
            .map(|conversation| conversation.id.clone())
            .unwrap_or_else(ConversationId::new);
        if conversations.is_empty() {
            let conversation =
                ConversationState::empty(active_conversation.clone(), next_updated_order + 1);
            history.save_conversation(
                &conversation.id,
                &conversation.title,
                &conversation.model_preference,
            )?;
            conversations.insert(active_conversation.clone(), conversation);
        }
        Ok(Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            runner,
            history,
            state: Mutex::new(ChatState {
                active_conversation,
                conversations,
                next_updated_order: next_updated_order.saturating_add(1),
                generation: 0,
                visible_surfaces: HashSet::new(),
                subscribers: Vec::new(),
            }),
        }))
    }

    fn prepare(
        &self,
        conversation_id: &ConversationId,
        draft: ChatDraft,
        regenerate: bool,
    ) -> Result<ChatInvocationRequest, ChatError> {
        let (input, draft_attachments) = draft.into_parts();
        let input = input.trim().to_owned();
        if input.is_empty() && draft_attachments.is_empty() {
            return Err(ChatError::EmptyInput);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let next_order = state.next_updated_order.saturating_add(1);
        let conversation = state
            .conversations
            .get_mut(conversation_id)
            .ok_or(ChatError::ConversationNotFound)?;
        if conversation.starting || conversation.active_invocation.is_some() {
            return Err(ChatError::Busy);
        }

        let (user_message_id, user_ordinal, new_user) = if regenerate {
            let user = last_user_message(conversation).ok_or(ChatError::NothingToRetry)?;
            (user.id.clone(), user.ordinal, None)
        } else {
            let ordinal = next_ordinal(conversation);
            let id = MessageId::new();
            let message = Arc::new(ChatMessageSnapshot {
                id: id.clone(),
                is_user: true,
                content: input.clone(),
                attachments: Arc::new(draft_attachments),
                status: ChatMessageStatus::Submitted,
                ordinal,
                attempt_id: None,
                reply_to_user_id: None,
            });
            (id, ordinal, Some(message))
        };
        let assistant_message_id = MessageId::new();
        let attempt_id = AttemptId::new();
        let assistant_ordinal = if new_user.is_some() {
            user_ordinal.saturating_add(1)
        } else {
            next_ordinal(conversation)
        };
        let assistant = Arc::new(ChatMessageSnapshot {
            id: assistant_message_id.clone(),
            is_user: false,
            content: String::new(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Generating,
            ordinal: assistant_ordinal,
            attempt_id: Some(attempt_id.clone()),
            reply_to_user_id: Some(user_message_id.clone()),
        });
        let mut proposed = conversation.messages.as_ref().clone();
        if let Some(user) = new_user {
            proposed.push(user);
        }
        proposed.push(assistant);
        let context_budget = self.runner.context_budget(&conversation.model_preference);
        let (messages, removed_rounds) =
            assemble_context(&proposed, &user_message_id, context_budget)?;

        if conversation.messages.is_empty() && !regenerate {
            conversation.title = if input.is_empty() {
                proposed
                    .iter()
                    .rev()
                    .find(|message| message.is_user)
                    .and_then(|message| message.attachments.first())
                    .map(|attachment| attachment.name().chars().take(48).collect())
                    .unwrap_or_else(|| "New chat".into())
            } else {
                input.chars().take(48).collect()
            };
        }
        conversation.messages = Arc::new(proposed);
        conversation.active_assistant = Some(assistant_message_id.clone());
        conversation.last_execution_sequence = 0;
        conversation.starting = true;
        conversation.status_text = "Starting…".into();
        conversation.has_unsaved_result = false;
        conversation.context_notice = (removed_rounds > 0).then(|| {
            format!("Estimated context budget removed {removed_rounds} older complete round(s).")
        });
        conversation.updated_order = next_order;
        let request = ChatInvocationRequest {
            conversation_id: conversation.id.clone(),
            user_message_id,
            assistant_message_id,
            attempt_id,
            reply_to_user_id: conversation
                .active_assistant
                .as_ref()
                .and_then(|assistant_id| {
                    conversation
                        .messages
                        .iter()
                        .find(|message| &message.id == assistant_id)
                })
                .and_then(|message| message.reply_to_user_id.clone())
                .expect("a prepared assistant attempt has a source user message"),
            conversation_title: conversation.title.clone(),
            model_preference: conversation.model_preference.clone(),
            user_ordinal,
            assistant_ordinal,
            input,
            messages,
        };
        state.next_updated_order = next_order;
        state.publish();
        Ok(request)
    }

    fn fail_to_start(&self, assistant_id: &MessageId, message: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(conversation) = state
            .conversations
            .values_mut()
            .find(|conversation| conversation.active_assistant.as_ref() == Some(assistant_id))
        {
            conversation.starting = false;
            conversation.active_invocation = None;
            replace_message(conversation, assistant_id, |message_snapshot| {
                message_snapshot.status = ChatMessageStatus::FailedPartial;
            });
            conversation.status_text = message;
        }
        state.publish();
    }

    async fn execute_draft_for(
        &self,
        conversation_id: ConversationId,
        draft: ChatDraft,
        regenerate: bool,
    ) -> Result<(), ChatError> {
        let request = self.prepare(&conversation_id, draft, regenerate)?;
        let assistant_id = request.assistant_message_id.clone();
        let observer: Arc<dyn ExecutionObserver> = Arc::new(ChatExecutionObserver {
            controller: self.self_weak.clone(),
        });
        match self.runner.run(request, observer).await {
            Ok(snapshot) if snapshot.status == ExecutionStatus::Completed => Ok(()),
            Ok(snapshot) if snapshot.status == ExecutionStatus::Cancelled => {
                Err(ChatError::Failed("request was cancelled".into()))
            }
            Ok(snapshot) => Err(ChatError::Failed(
                snapshot.error.unwrap_or_else(|| "request failed".into()),
            )),
            Err(error) => {
                self.fail_to_start(&assistant_id, error.to_string());
                Err(ChatError::Failed(error.to_string()))
            }
        }
    }
}

impl ChatUiPort for ChatController {
    fn snapshot(&self) -> ChatSnapshot {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
    }

    fn subscribe(&self, capacity: usize) -> Receiver<ChatSnapshot> {
        let (sender, receiver) = async_channel::bounded(capacity.max(1));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = sender.try_send(state.snapshot());
        state.subscribers.push(ChatSubscriber {
            sender,
            stale_receiver: receiver.clone(),
        });
        receiver
    }

    fn set_surface_visible(&self, surface: SurfaceKind, visible: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if visible {
            state.visible_surfaces.insert(surface);
            let snapshot = state.snapshot();
            state.broadcast(snapshot);
        } else {
            state.visible_surfaces.remove(&surface);
        }
    }

    fn create_conversation(&self) -> Result<ConversationId, ChatError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.next_updated_order = state.next_updated_order.saturating_add(1);
        let id = ConversationId::new();
        let conversation = ConversationState::empty(id.clone(), state.next_updated_order);
        self.history.save_conversation(
            &conversation.id,
            &conversation.title,
            &conversation.model_preference,
        )?;
        state.conversations.insert(id.clone(), conversation);
        state.active_conversation = id.clone();
        state.publish();
        Ok(id)
    }

    fn rename_conversation(&self, title: String) -> Result<(), ChatError> {
        let title = title.trim().chars().take(120).collect::<String>();
        if title.is_empty() {
            return Err(ChatError::Failed(
                "conversation title cannot be empty".into(),
            ));
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = state.active_conversation.clone();
        let conversation = state
            .conversations
            .get_mut(&id)
            .ok_or(ChatError::ConversationNotFound)?;
        self.history
            .save_conversation(&conversation.id, &title, &conversation.model_preference)?;
        conversation.title = title;
        state.touch(&id);
        state.publish();
        Ok(())
    }

    fn switch_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.conversations.contains_key(conversation_id) {
            return Err(ChatError::ConversationNotFound);
        }
        state.active_conversation = conversation_id.clone();
        state.publish();
        Ok(())
    }

    fn delete_conversation(&self, conversation_id: &ConversationId) -> Result<(), ChatError> {
        let invocation = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .conversations
            .get(conversation_id)
            .and_then(|conversation| conversation.active_invocation.clone());
        if let Some(invocation) = invocation {
            self.runner.cancel(&invocation).map_err(|error| {
                ChatError::Failed(format!("could not stop the conversation: {error}"))
            })?;
        }
        self.history.delete_conversation(conversation_id)?;
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.conversations.remove(conversation_id);
        if state.conversations.is_empty() {
            state.next_updated_order = state.next_updated_order.saturating_add(1);
            let id = ConversationId::new();
            let conversation = ConversationState::empty(id.clone(), state.next_updated_order);
            self.history.save_conversation(
                &conversation.id,
                &conversation.title,
                &conversation.model_preference,
            )?;
            state.conversations.insert(id.clone(), conversation);
            state.active_conversation = id;
        } else if &state.active_conversation == conversation_id {
            state.active_conversation = state
                .conversations
                .values()
                .max_by_key(|conversation| conversation.updated_order)
                .expect("a non-empty conversation index has an item")
                .id
                .clone();
        }
        state.publish();
        Ok(())
    }

    fn set_model_preference(&self, preference: ChatModelPreference) -> Result<(), ChatError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = state.active_conversation.clone();
        let conversation = state
            .conversations
            .get_mut(&id)
            .ok_or(ChatError::ConversationNotFound)?;
        self.history
            .save_conversation(&conversation.id, &conversation.title, &preference)?;
        conversation.model_preference = preference;
        state.touch(&id);
        state.publish();
        Ok(())
    }

    fn set_conversation_favorite(
        &self,
        conversation_id: &ConversationId,
        favorite: bool,
    ) -> Result<(), ChatError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let conversation = state
            .conversations
            .get_mut(conversation_id)
            .ok_or(ChatError::ConversationNotFound)?;
        self.history
            .set_conversation_favorite(conversation_id, favorite)?;
        conversation.favorite = favorite;
        state.publish();
        Ok(())
    }

    fn search_conversations(
        &self,
        query: &str,
        favorites_only: bool,
    ) -> Vec<ChatConversationSummary> {
        let query = query.trim().to_lowercase();
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut matches = state
            .conversations
            .values()
            .filter(|conversation| !favorites_only || conversation.favorite)
            .filter(|conversation| {
                query.is_empty()
                    || conversation.title.to_lowercase().contains(&query)
                    || conversation
                        .messages
                        .iter()
                        .any(|message| message.content.to_lowercase().contains(&query))
            })
            .map(ConversationState::summary)
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .updated_order()
                .cmp(&left.updated_order())
                .then_with(|| left.id().cmp(right.id()))
        });
        matches
    }

    fn send_draft(&self, draft: ChatDraft) -> ChatUiResultFuture<'_> {
        let conversation_id = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active_conversation
            .clone();
        Box::pin(async move { self.execute_draft_for(conversation_id, draft, false).await })
    }

    fn stop(&self) -> Result<(), ChatError> {
        let invocation = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state
                .conversations
                .get(&state.active_conversation)
                .and_then(|conversation| conversation.active_invocation.clone())
                .ok_or(ChatError::Busy)?
        };
        self.runner
            .cancel(&invocation)
            .map_err(|error| ChatError::Failed(error.to_string()))
    }

    fn retry(&self) -> ChatUiResultFuture<'_> {
        let (conversation_id, draft) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let conversation = state
                .conversations
                .get(&state.active_conversation)
                .ok_or(ChatError::ConversationNotFound);
            match conversation {
                Ok(conversation) => (
                    Ok(conversation.id.clone()),
                    last_user_message(conversation).map(|message| {
                        ChatDraft::new(
                            message.content.clone(),
                            message.attachments.as_ref().clone(),
                        )
                    }),
                ),
                Err(error) => (Err(error), None),
            }
        };
        Box::pin(async move {
            let conversation_id = conversation_id?;
            let draft = draft.ok_or(ChatError::NothingToRetry)?;
            self.execute_draft_for(conversation_id, draft, true).await
        })
    }
}

struct ChatExecutionObserver {
    controller: Weak<ChatController>,
}

impl ExecutionObserver for ChatExecutionObserver {
    fn on_execution(&self, snapshot: ExecutionSnapshot) {
        let Some(controller) = self.controller.upgrade() else {
            return;
        };
        let conversation_id = &snapshot.conversation_id;
        let mut state = controller
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(conversation) = state.conversations.get_mut(conversation_id) else {
            return;
        };
        if conversation
            .active_assistant
            .as_ref()
            .is_some_and(|id| &snapshot.assistant_message_id != id)
            || snapshot.sequence <= conversation.last_execution_sequence
        {
            return;
        }
        conversation.last_execution_sequence = snapshot.sequence;
        conversation.starting = false;
        conversation.active_invocation =
            (!snapshot.status.is_terminal()).then(|| snapshot.invocation_id.clone());
        replace_message(conversation, &snapshot.assistant_message_id, |message| {
            message.content = snapshot.output.clone();
            message.status = match snapshot.status {
                ExecutionStatus::Completed => ChatMessageStatus::Completed,
                ExecutionStatus::Cancelled => ChatMessageStatus::CancelledPartial,
                ExecutionStatus::Failed | ExecutionStatus::Interrupted => {
                    ChatMessageStatus::FailedPartial
                }
                ExecutionStatus::Queued
                | ExecutionStatus::Running
                | ExecutionStatus::Cancelling => ChatMessageStatus::Generating,
            };
        });
        conversation.has_unsaved_result = snapshot.storage == StorageState::Unsaved;
        conversation.status_text = match snapshot.status {
            ExecutionStatus::Queued => "Queued".into(),
            ExecutionStatus::Running => "Generating…".into(),
            ExecutionStatus::Cancelling => "Stopping…".into(),
            ExecutionStatus::Completed if snapshot.storage == StorageState::Pending => {
                "Answer complete · saving…".into()
            }
            ExecutionStatus::Completed => "Answer complete".into(),
            ExecutionStatus::Cancelled => "Stopped · partial answer kept".into(),
            ExecutionStatus::Failed | ExecutionStatus::Interrupted => snapshot
                .error
                .unwrap_or_else(|| "Request failed · partial answer kept".into()),
        };
        state.publish();
    }
}

fn replace_message(
    conversation: &mut ConversationState,
    message_id: &MessageId,
    update: impl FnOnce(&mut ChatMessageSnapshot),
) {
    let messages = Arc::make_mut(&mut conversation.messages);
    if let Some(message) = messages
        .iter_mut()
        .find(|message| &message.id == message_id)
    {
        update(Arc::make_mut(message));
    }
}

fn next_ordinal(conversation: &ConversationState) -> u64 {
    conversation
        .messages
        .iter()
        .map(|message| message.ordinal)
        .max()
        .map_or(0, |ordinal| ordinal.saturating_add(1))
}

fn last_user_message(conversation: &ConversationState) -> Option<&ChatMessageSnapshot> {
    conversation
        .messages
        .iter()
        .rev()
        .find(|message| message.is_user)
        .map(AsRef::as_ref)
}

fn assemble_context(
    messages: &[Arc<ChatMessageSnapshot>],
    current_user_id: &MessageId,
    context_budget_tokens: usize,
) -> Result<(Vec<AiMessage>, usize), ChatError> {
    let mut rounds = Vec::<Vec<AiMessage>>::new();
    for user in messages.iter().filter(|message| message.is_user) {
        let mut round = vec![AiMessage {
            role: AiRole::User,
            content: user.content.clone(),
            attachments: user.attachments.as_ref().clone(),
        }];
        let latest_complete = messages
            .iter()
            .filter(|message| {
                !message.is_user
                    && message.reply_to_user_id.as_ref() == Some(&user.id)
                    && message.status == ChatMessageStatus::Completed
            })
            .max_by_key(|message| message.ordinal);
        if let Some(assistant) = latest_complete {
            round.push(AiMessage {
                role: AiRole::Assistant,
                content: assistant.content.clone(),
                attachments: Vec::new(),
            });
        }
        rounds.push(round);
        if &user.id == current_user_id {
            break;
        }
    }

    let budget_chars = context_budget_tokens.saturating_mul(ESTIMATED_CHARS_PER_TOKEN);
    let current_chars = rounds
        .last()
        .map(|round| round.iter().map(estimated_message_chars).sum())
        .unwrap_or(0);
    if current_chars > budget_chars {
        return Err(ChatError::ContextBudgetExceeded);
    }
    let mut removed = 0;
    while rounds
        .iter()
        .flatten()
        .map(estimated_message_chars)
        .sum::<usize>()
        > budget_chars
        && rounds.len() > 1
    {
        rounds.remove(0);
        removed += 1;
    }
    Ok((rounds.into_iter().flatten().collect(), removed))
}

fn estimated_message_chars(message: &AiMessage) -> usize {
    message.content.chars().count()
        + message
            .attachments
            .iter()
            .map(|attachment| match attachment.content() {
                ChatAttachmentContent::Text { text } => text.chars().count(),
                ChatAttachmentContent::Image { .. } => 4_096,
            })
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use std::{future, sync::Mutex};

    use lexwisp_core::{ChatRunFuture, ProviderId};

    use super::*;

    #[derive(Default)]
    struct MemoryHistory(Mutex<Vec<PersistedChatConversation>>);

    impl ChatHistoryPort for MemoryHistory {
        fn restore(&self) -> Result<Vec<PersistedChatConversation>, ChatError> {
            Ok(self.0.lock().expect("history lock").clone())
        }

        fn save_conversation(
            &self,
            _: &ConversationId,
            _: &str,
            _: &ChatModelPreference,
        ) -> Result<(), ChatError> {
            Ok(())
        }

        fn delete_conversation(&self, _: &ConversationId) -> Result<(), ChatError> {
            Ok(())
        }

        fn set_conversation_favorite(&self, _: &ConversationId, _: bool) -> Result<(), ChatError> {
            Ok(())
        }
    }

    struct ImmediateRun;
    impl ChatRunPort for ImmediateRun {
        fn run(
            &self,
            request: ChatInvocationRequest,
            observer: Arc<dyn ExecutionObserver>,
        ) -> ChatRunFuture<'_> {
            let snapshot = ExecutionSnapshot {
                invocation_id: InvocationId::new(),
                conversation_id: request.conversation_id,
                user_message_id: request.user_message_id,
                assistant_message_id: request.assistant_message_id,
                provider_id: ProviderId::parse("fixture").expect("provider ID"),
                model_id: "fixture".into(),
                sequence: 1,
                text_version: 1,
                status: ExecutionStatus::Completed,
                output: "answer".into(),
                error: None,
                storage: StorageState::Saved,
                retention_generation: 0,
            };
            observer.on_execution(snapshot.clone());
            Box::pin(future::ready(Ok(snapshot)))
        }

        fn cancel(&self, _: &InvocationId) -> Result<(), lexwisp_core::ChatRunError> {
            Ok(())
        }

        fn context_budget(&self, _: &ChatModelPreference) -> usize {
            8_192
        }
    }

    fn controller() -> Arc<ChatController> {
        ChatController::new(Arc::new(ImmediateRun), Arc::new(MemoryHistory::default()))
            .expect("controller starts")
    }

    #[test]
    fn controller_uses_stable_ids_and_projects_the_answer() {
        let controller = controller();
        let conversation = controller.snapshot().conversation_id;
        futures_lite::future::block_on(controller.send("hello".into())).expect("chat succeeds");
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.conversation_id, conversation);
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[1].content, "answer");
    }

    #[test]
    fn conversations_are_independent_and_regeneration_keeps_attempts() {
        let controller = controller();
        futures_lite::future::block_on(controller.send("first".into())).expect("first send");
        futures_lite::future::block_on(controller.retry()).expect("regenerate");
        let first = controller.snapshot();
        assert_eq!(first.messages.len(), 3);
        assert_ne!(
            first.messages[1].attempt_id, first.messages[2].attempt_id,
            "regeneration keeps a distinct attempt"
        );

        let second_id = controller
            .create_conversation()
            .expect("second conversation");
        futures_lite::future::block_on(controller.send("second".into())).expect("second send");
        assert_eq!(controller.snapshot().conversation_id, second_id);
        assert_eq!(controller.snapshot().messages[0].content, "second");
        controller
            .switch_conversation(&first.conversation_id)
            .expect("switch back");
        assert_eq!(controller.snapshot().messages[0].content, "first");
    }

    #[test]
    fn one_conversation_is_serial_while_two_can_prepare_in_parallel() {
        let controller = controller();
        let first = controller.snapshot().conversation_id;
        controller
            .prepare(&first, ChatDraft::text_only("first request"), false)
            .expect("first conversation prepares");
        assert!(matches!(
            controller.prepare(&first, ChatDraft::text_only("competing request"), false,),
            Err(ChatError::Busy)
        ));

        let second = controller
            .create_conversation()
            .expect("second conversation");
        controller
            .prepare(&second, ChatDraft::text_only("independent request"), false)
            .expect("another conversation prepares independently");
    }

    #[test]
    fn partial_attempts_do_not_enter_the_next_context() {
        let user = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: true,
            content: "question".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Submitted,
            ordinal: 0,
            attempt_id: None,
            reply_to_user_id: None,
        });
        let partial = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: false,
            content: "partial answer".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::CancelledPartial,
            ordinal: 1,
            attempt_id: Some(AttemptId::new()),
            reply_to_user_id: Some(user.id.clone()),
        });
        let next = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: true,
            content: "next question".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Submitted,
            ordinal: 2,
            attempt_id: None,
            reply_to_user_id: None,
        });
        let (context, _) = assemble_context(&[user, partial, next.clone()], &next.id, 8_192)
            .expect("context assembles");
        assert_eq!(context.len(), 2);
        assert!(context.iter().all(|message| message.role == AiRole::User));
    }

    #[test]
    fn context_budget_removes_whole_old_rounds_and_never_truncates_current_input() {
        let old_user = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: true,
            content: "old".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Submitted,
            ordinal: 0,
            attempt_id: None,
            reply_to_user_id: None,
        });
        let old_answer = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: false,
            content: "answer".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Completed,
            ordinal: 1,
            attempt_id: Some(AttemptId::new()),
            reply_to_user_id: Some(old_user.id.clone()),
        });
        let current = Arc::new(ChatMessageSnapshot {
            id: MessageId::new(),
            is_user: true,
            content: "current".into(),
            attachments: Arc::new(Vec::new()),
            status: ChatMessageStatus::Submitted,
            ordinal: 2,
            attempt_id: None,
            reply_to_user_id: None,
        });

        let (context, removed) =
            assemble_context(&[old_user, old_answer, current.clone()], &current.id, 2)
                .expect("the current input fits after removing the old round");
        assert_eq!(removed, 1);
        assert_eq!(context.len(), 1);
        assert_eq!(context[0].content, "current");
        assert!(matches!(
            assemble_context(std::slice::from_ref(&current), &current.id, 1),
            Err(ChatError::ContextBudgetExceeded)
        ));
    }

    #[test]
    fn bounded_subscriber_recovers_to_the_latest_snapshot() {
        let controller = controller();
        controller.set_surface_visible(SurfaceKind::MainShell, true);
        let receiver = controller.subscribe(1);
        controller
            .create_conversation()
            .expect("second conversation");
        let latest = controller
            .create_conversation()
            .expect("third conversation");

        let snapshot = receiver.try_recv().expect("latest snapshot is queued");
        assert_eq!(snapshot.conversation_id, latest);
        assert_eq!(snapshot.conversations.len(), 3);
    }
}

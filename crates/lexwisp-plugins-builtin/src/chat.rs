use std::sync::{Arc, Mutex, Weak};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{
    ActionDescriptor, ActionError, ActionFuture, ActionHandler, ActionId, ActionRequest,
    ActionResult, AiMessage, AiRole, Capability, ChatError, ChatInvocationRequest,
    ChatMessageSnapshot, ChatMessageStatus, ChatRunPort, ChatSnapshot, ChatUiPort,
    ChatUiResultFuture, ConversationId, ExecutionObserver, ExecutionSnapshot, ExecutionStatus,
    InvocationId, MessageId, PluginDescriptor, PluginId, QualifiedActionId, StorageState,
};

pub fn chat_plugin() -> PluginDescriptor {
    PluginDescriptor::new(
        PluginId::parse("org.lexwisp.chat").expect("built-in Chat plugin ID is valid"),
        "Chat",
        vec![Capability::AiInvoke, Capability::StorageWrite],
    )
}

pub fn chat_action() -> ActionDescriptor {
    let plugin = chat_plugin();
    ActionDescriptor::new(
        plugin.id().clone(),
        ActionId::parse("ask").expect("built-in Chat action ID is valid"),
        "Ask",
    )
}

struct ChatState {
    conversation_id: ConversationId,
    title: String,
    messages: Vec<ChatMessageSnapshot>,
    active_invocation: Option<InvocationId>,
    active_assistant: Option<MessageId>,
    last_execution_sequence: u64,
    generation: u64,
    starting: bool,
    visible: bool,
    status_text: String,
    has_unsaved_result: bool,
    subscribers: Vec<Sender<ChatSnapshot>>,
}

impl ChatState {
    fn snapshot(&self) -> ChatSnapshot {
        let active = self.starting || self.active_invocation.is_some();
        ChatSnapshot {
            conversation_id: self.conversation_id.clone(),
            title: self.title.clone(),
            messages: self.messages.clone(),
            active_invocation: self.active_invocation.clone(),
            generation: self.generation,
            status_text: self.status_text.clone(),
            can_send: !active,
            can_stop: self.active_invocation.is_some(),
            can_retry: !active
                && self
                    .messages
                    .iter()
                    .any(|message| message.is_user && !message.content.trim().is_empty()),
            has_unsaved_result: self.has_unsaved_result,
        }
    }

    fn publish(&mut self) {
        self.generation = self.generation.saturating_add(1);
        if !self.visible {
            return;
        }
        let snapshot = self.snapshot();
        self.subscribers.retain(|subscriber| {
            matches!(
                subscriber.try_send(snapshot.clone()),
                Ok(()) | Err(TrySendError::Full(_))
            )
        });
    }
}

pub struct ChatController {
    self_weak: Weak<Self>,
    action: QualifiedActionId,
    runner: Arc<dyn ChatRunPort>,
    state: Mutex<ChatState>,
}

impl ChatController {
    pub fn new(runner: Arc<dyn ChatRunPort>) -> Arc<Self> {
        let descriptor = chat_action();
        let action =
            QualifiedActionId::new(descriptor.plugin_id().clone(), descriptor.id().clone());
        Arc::new_cyclic(|weak| Self {
            self_weak: weak.clone(),
            action,
            runner,
            state: Mutex::new(ChatState {
                conversation_id: ConversationId::new(),
                title: "New conversation".into(),
                messages: Vec::new(),
                active_invocation: None,
                active_assistant: None,
                last_execution_sequence: 0,
                generation: 0,
                starting: false,
                visible: false,
                status_text: "Ready".into(),
                has_unsaved_result: false,
                subscribers: Vec::new(),
            }),
        })
    }

    pub fn action_id(&self) -> QualifiedActionId {
        self.action.clone()
    }

    fn prepare(&self, input: String) -> Result<ChatInvocationRequest, ChatError> {
        let input = input.trim().to_owned();
        if input.is_empty() {
            return Err(ChatError::EmptyInput);
        }
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.starting || state.active_invocation.is_some() {
            return Err(ChatError::Busy);
        }
        let user_message_id = MessageId::new();
        let assistant_message_id = MessageId::new();
        let user_ordinal = state.messages.len() as u64;
        let assistant_ordinal = user_ordinal.saturating_add(1);
        if state.messages.is_empty() {
            state.title = input.chars().take(48).collect();
        }
        state.messages.push(ChatMessageSnapshot {
            id: user_message_id.clone(),
            is_user: true,
            content: input.clone(),
            status: ChatMessageStatus::Submitted,
        });
        state.messages.push(ChatMessageSnapshot {
            id: assistant_message_id.clone(),
            is_user: false,
            content: String::new(),
            status: ChatMessageStatus::Generating,
        });
        state.active_assistant = Some(assistant_message_id.clone());
        state.last_execution_sequence = 0;
        state.starting = true;
        state.status_text = "Starting…".into();
        state.has_unsaved_result = false;

        let messages = state
            .messages
            .iter()
            .filter(|message| message.id != assistant_message_id)
            .filter(|message| !message.content.is_empty())
            .map(|message| AiMessage {
                role: if message.is_user {
                    AiRole::User
                } else {
                    AiRole::Assistant
                },
                content: message.content.clone(),
            })
            .collect();
        let request = ChatInvocationRequest {
            action: self.action.clone(),
            conversation_id: state.conversation_id.clone(),
            user_message_id,
            assistant_message_id,
            conversation_title: state.title.clone(),
            user_ordinal,
            assistant_ordinal,
            messages,
        };
        state.publish();
        Ok(request)
    }

    fn fail_to_start(&self, message: String) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.starting = false;
        state.active_invocation = None;
        let active_assistant = state.active_assistant.clone();
        if let Some(message_id) = active_assistant
            && let Some(message) = state
                .messages
                .iter_mut()
                .find(|message| message.id == message_id)
        {
            message.status = ChatMessageStatus::FailedPartial;
        }
        state.status_text = message;
        state.publish();
    }

    async fn execute_input(&self, input: String) -> Result<ActionResult, ActionError> {
        let request = self
            .prepare(input)
            .map_err(|error| ActionError::Failed(error.to_string()))?;
        let observer: Arc<dyn ExecutionObserver> = Arc::new(ChatExecutionObserver {
            controller: self.self_weak.clone(),
        });
        match self.runner.run(request, observer).await {
            Ok(snapshot) if snapshot.status == ExecutionStatus::Completed => Ok(ActionResult {
                output: snapshot.output,
            }),
            Ok(snapshot) if snapshot.status == ExecutionStatus::Cancelled => {
                Err(ActionError::Cancelled)
            }
            Ok(snapshot) => Err(ActionError::Failed(
                snapshot.error.unwrap_or_else(|| "request failed".into()),
            )),
            Err(error) => {
                self.fail_to_start(error.to_string());
                Err(ActionError::Failed(error.to_string()))
            }
        }
    }
}

impl ActionHandler for ChatController {
    fn execute<'a>(&'a self, request: ActionRequest) -> ActionFuture<'a> {
        Box::pin(self.execute_input(request.input))
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
        state.subscribers.push(sender);
        receiver
    }

    fn set_surface_visible(&self, visible: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.visible = visible;
        if visible {
            let snapshot = state.snapshot();
            state.subscribers.retain(|subscriber| {
                matches!(
                    subscriber.try_send(snapshot.clone()),
                    Ok(()) | Err(TrySendError::Full(_))
                )
            });
        }
    }

    fn stop(&self) -> Result<(), ChatError> {
        let invocation = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .active_invocation
            .clone()
            .ok_or(ChatError::Busy)?;
        self.runner
            .cancel(&invocation)
            .map_err(|error| ChatError::Failed(error.to_string()))
    }

    fn retry(&self) -> ChatUiResultFuture<'_> {
        let input = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messages
            .iter()
            .rev()
            .find(|message| message.is_user)
            .map(|message| message.content.clone());
        Box::pin(async move {
            let input = input.ok_or(ChatError::NothingToRetry)?;
            self.execute_input(input)
                .await
                .map(|_| ())
                .map_err(|error| ChatError::Failed(error.to_string()))
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
        let mut state = controller
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .active_assistant
            .as_ref()
            .is_some_and(|id| snapshot.assistant_message_id.as_ref() != Some(id))
            || snapshot.sequence <= state.last_execution_sequence
        {
            return;
        }
        state.last_execution_sequence = snapshot.sequence;
        state.starting = false;
        state.active_invocation =
            (!snapshot.status.is_terminal()).then(|| snapshot.invocation_id.clone());
        if let Some(message) = state
            .messages
            .iter_mut()
            .find(|message| Some(&message.id) == snapshot.assistant_message_id.as_ref())
        {
            message.content = snapshot.output;
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
        }
        state.has_unsaved_result = snapshot.storage == StorageState::Unsaved;
        state.status_text = match snapshot.status {
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

#[cfg(test)]
mod tests {
    use std::future;

    use lexwisp_core::{ChatRunFuture, ProviderId};

    use super::*;

    struct ImmediateRun;
    impl ChatRunPort for ImmediateRun {
        fn run(
            &self,
            request: ChatInvocationRequest,
            observer: Arc<dyn ExecutionObserver>,
        ) -> ChatRunFuture<'_> {
            let snapshot = ExecutionSnapshot {
                invocation_id: InvocationId::new(),
                plugin_id: request.action.plugin_id().clone(),
                action: request.action.clone(),
                plugin_generation: 1,
                conversation_id: Some(request.conversation_id),
                user_message_id: Some(request.user_message_id),
                assistant_message_id: Some(request.assistant_message_id),
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
    }

    #[test]
    fn controller_uses_stable_ids_and_projects_the_answer() {
        let controller = ChatController::new(Arc::new(ImmediateRun));
        let conversation = controller.snapshot().conversation_id;
        let result =
            futures_lite::future::block_on(controller.execute(ActionRequest::manual("hello")))
                .expect("chat succeeds");
        assert_eq!(result.output, "answer");
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.conversation_id, conversation);
        assert_eq!(snapshot.messages.len(), 2);
        assert_eq!(snapshot.messages[1].content, "answer");
    }
}

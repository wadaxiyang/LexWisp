use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    bubble::{Bubble, BubbleVariant},
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader},
    message_scroller::{MessageScroller, MessageScrollerState},
    radio::Radio,
    scroll::ScrollableElement,
    switch::Switch,
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, WeakEntity, Window, div, px,
};
use lexwisp_core::{
    ActionDescriptor, ActionKind, ActionRequest, ActionUiPort, CaptureStatus, ChatMessageSnapshot,
    ChatMessageStatus, ChatSnapshot, ChatUiPort, ContextSnapshot, ContextUiPort, FavoriteUiPort,
    InputSource, LaunchRoute, ParameterKind, QualifiedActionId, SettingsUiPort, StorageState,
    TextActionSnapshot, TextActionUiPort,
};
use lexwisp_ui::SurfaceController;

pub struct QuickShellChat {
    controller: WeakEntity<SurfaceController>,
    chat: Arc<dyn ChatUiPort>,
    actions: Arc<dyn ActionUiPort>,
    action: QualifiedActionId,
    snapshot: ChatSnapshot,
    composer: Entity<TextareaState>,
    scroller: Entity<MessageScrollerState>,
    transient_status: Option<SharedString>,
    request_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _snapshot_task: Task<()>,
}

impl QuickShellChat {
    pub fn new(
        controller: WeakEntity<SurfaceController>,
        chat: Arc<dyn ChatUiPort>,
        actions: Arc<dyn ActionUiPort>,
        action: QualifiedActionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = chat.snapshot();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask anything…")
                .submit_on_enter(true)
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(snapshot.messages.len(), cx));
        let submit_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        shift: false,
                        secondary: false
                    }
                ) {
                    let input = state.read(cx).value().to_string();
                    if this.begin_send(input, cx) {
                        state.update(cx, |state, cx| state.set_value("", window, cx));
                    }
                }
            },
        );
        let receiver = chat.subscribe(8);
        let snapshot_task = cx.spawn(async move |view, cx| {
            while let Ok(snapshot) = receiver.recv().await {
                if view
                    .update(cx, |view, cx| view.apply_snapshot(snapshot, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            controller,
            chat,
            actions,
            action,
            snapshot,
            composer,
            scroller,
            transient_status: None,
            request_task: None,
            _subscriptions: vec![submit_subscription],
            _snapshot_task: snapshot_task,
        }
    }

    fn begin_send(&mut self, input: String, cx: &mut Context<Self>) -> bool {
        if input.trim().is_empty() || !self.snapshot.can_send {
            return false;
        }
        self.transient_status = None;
        let actions = self.actions.clone();
        let action = self.action.clone();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            let result = actions.invoke(action, ActionRequest::manual(input)).await;
            if let Err(error) = result {
                let _ = view.update(cx, |view, cx| {
                    view.transient_status = Some(error.to_string().into());
                    cx.notify();
                });
            }
        }));
        true
    }

    fn begin_retry(&mut self, cx: &mut Context<Self>) {
        if !self.snapshot.can_retry {
            return;
        }
        self.transient_status = None;
        let chat = self.chat.clone();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            if let Err(error) = chat.retry().await {
                let _ = view.update(cx, |view, cx| {
                    view.transient_status = Some(error.to_string().into());
                    cx.notify();
                });
            }
        }));
    }

    fn apply_snapshot(&mut self, snapshot: ChatSnapshot, cx: &mut Context<Self>) {
        if snapshot.generation <= self.snapshot.generation {
            return;
        }
        let old_count = self.snapshot.messages.len();
        let new_count = snapshot.messages.len();
        self.snapshot = snapshot;
        self.scroller.update(cx, |scroller, cx| {
            if new_count > old_count {
                let _ = scroller.append(new_count - old_count, cx);
            } else if new_count != old_count {
                scroller.reset(new_count, cx);
            } else if new_count > 0 {
                let _ = scroller.remeasure_items(new_count - 1..new_count, cx);
            }
        });
        cx.notify();
    }
}

impl Render for QuickShellChat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let messages = Arc::new(self.snapshot.messages.clone());
        let scroller = self.scroller.clone();
        let settings_controller = self.controller.clone();
        let hide_controller = self.controller.clone();
        let send_enabled = self.snapshot.can_send;
        let retry_enabled = self.snapshot.can_retry;
        let stop_enabled = self.snapshot.can_stop;
        let status = self
            .transient_status
            .clone()
            .unwrap_or_else(|| self.snapshot.status_text.clone().into());

        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_lg().font_semibold().child("LexWisp Chat"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.snapshot.title.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("chat-settings")
                                    .ghost()
                                    .label("Settings")
                                    .on_click(move |_, _, cx| {
                                        let _ = settings_controller.update(cx, |controller, cx| {
                                            let _ = controller.show_control_center(cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("chat-hide")
                                    .ghost()
                                    .label("Hide")
                                    .on_click(move |_, window, cx| {
                                        let _ = hide_controller.update(cx, |controller, cx| {
                                            controller.hide_quick_shell(window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .when(self.snapshot.messages.is_empty(), |this| {
                        this.child(
                            div()
                                .size_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("Configure a provider in Settings, then ask your first question."),
                        )
                    })
                    .when(!self.snapshot.messages.is_empty(), |this| {
                        this.child(
                            MessageScroller::new(
                                "chat-transcript",
                                scroller,
                                move |index, _, _| render_message(messages[index].clone()),
                            )
                            .w_full()
                            .h_full()
                            .with_jump_button_label("Latest"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Textarea::new(&self.composer)
                            .h(px(92.))
                            .disabled(!send_enabled)
                            .aria_label("Message composer"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if self.snapshot.has_unsaved_result {
                                        cx.theme().danger
                                    } else {
                                        cx.theme().muted_foreground
                                    })
                                    .child(status),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(
                                        Button::new("chat-retry")
                                            .label("Retry")
                                            .disabled(!retry_enabled)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.begin_retry(cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("chat-stop")
                                            .label("Stop")
                                            .disabled(!stop_enabled)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                if let Err(error) = this.chat.stop() {
                                                    this.transient_status =
                                                        Some(error.to_string().into());
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("chat-send")
                                            .primary()
                                            .label("Send")
                                            .disabled(!send_enabled)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                let input =
                                                    this.composer.read(cx).value().to_string();
                                                if this.begin_send(input, cx) {
                                                    this.composer.update(cx, |composer, cx| {
                                                        composer.set_value("", window, cx);
                                                    });
                                                }
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Enter sends · Shift+Enter adds a line · hiding this window does not stop Chat"),
                    ),
            )
    }
}

fn render_message(message: ChatMessageSnapshot) -> impl IntoElement {
    let id = message.id.as_str().to_owned();
    let content = if message.content.is_empty() {
        "Waiting for the provider…".to_owned()
    } else {
        message.content.clone()
    };
    let status = match message.status {
        ChatMessageStatus::Submitted => "Submitted",
        ChatMessageStatus::Generating => "Generating",
        ChatMessageStatus::Completed => "Complete",
        ChatMessageStatus::CancelledPartial => "Stopped · partial",
        ChatMessageStatus::FailedPartial => "Failed · partial kept",
    };
    let alignment = if message.is_user {
        MessageAlignment::End
    } else {
        MessageAlignment::Start
    };
    let variant = if message.is_user {
        BubbleVariant::Filled
    } else if message.status == ChatMessageStatus::FailedPartial {
        BubbleVariant::Destructive
    } else {
        BubbleVariant::Muted
    };
    let body = TextView::markdown(format!("message-body-{id}"), content.clone()).selectable(true);
    let footer = if message.is_user {
        MessageFooter::new().child(status)
    } else {
        MessageFooter::new().child(status).child(
            Clipboard::new(format!("copy-message-{id}"))
                .value(content)
                .tooltip("Copy answer"),
        )
    };
    Message::new()
        .alignment(alignment)
        .header(MessageHeader::new().child(if message.is_user { "You" } else { "LexWisp" }))
        .content(
            MessageContent::new().bubble(
                Bubble::new()
                    .alignment(alignment)
                    .with_variant(variant)
                    .child(body),
            ),
        )
        .footer(footer)
}

pub struct QuickShell {
    controller: WeakEntity<SurfaceController>,
    settings: Arc<dyn SettingsUiPort>,
    context: Arc<dyn ContextUiPort>,
    favorites: Arc<dyn FavoriteUiPort>,
    actions: Arc<dyn ActionUiPort>,
    chat: Arc<dyn ChatUiPort>,
    chat_action: QualifiedActionId,
    descriptors: Vec<ActionDescriptor>,
    text_actions: Vec<Arc<dyn TextActionUiPort>>,
    selected_action: QualifiedActionId,
    launch_context: ContextSnapshot,
    candidate_confirmed: bool,
    explicit_input: Option<(String, InputSource)>,
    composer: Entity<TextareaState>,
    parameter_inputs: HashMap<(QualifiedActionId, String), Entity<InputState>>,
    parameter_values: BTreeMap<(QualifiedActionId, String), String>,
    chat_snapshot: ChatSnapshot,
    text_snapshots: HashMap<QualifiedActionId, TextActionSnapshot>,
    result_tokens: HashMap<QualifiedActionId, lexwisp_core::ContextToken>,
    transient_status: Option<SharedString>,
    request_task: Option<Task<()>>,
    side_effect_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _listener_tasks: Vec<Task<()>>,
}

impl QuickShell {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controller: WeakEntity<SurfaceController>,
        settings: Arc<dyn SettingsUiPort>,
        context: Arc<dyn ContextUiPort>,
        favorites: Arc<dyn FavoriteUiPort>,
        actions: Arc<dyn ActionUiPort>,
        chat: Arc<dyn ChatUiPort>,
        chat_action: QualifiedActionId,
        descriptors: Vec<ActionDescriptor>,
        text_actions: Vec<Arc<dyn TextActionUiPort>>,
        launches: async_channel::Receiver<ContextSnapshot>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask, translate, or polish…")
                .submit_on_enter(true)
        });
        let submit_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, _, cx| {
                if matches!(
                    event,
                    InputEvent::PressEnter {
                        shift: false,
                        secondary: false
                    }
                ) {
                    let manual = state.read(cx).value().to_string();
                    this.begin_selected(Some(manual), cx);
                }
            },
        );
        let mut parameter_inputs = HashMap::new();
        let mut parameter_values = BTreeMap::new();
        for descriptor in &descriptors {
            let ActionKind::Declarative(definition) = descriptor.kind() else {
                continue;
            };
            let action = descriptor.qualified_id();
            for parameter in &definition.parameters {
                let default = parameter.default_value.clone().unwrap_or_default();
                match parameter.kind {
                    ParameterKind::Text | ParameterKind::Number => {
                        let placeholder = parameter.label.clone();
                        let default_value = default.clone();
                        let input = cx.new(|cx| {
                            InputState::new(window, cx)
                                .placeholder(placeholder)
                                .default_value(default_value)
                        });
                        parameter_inputs.insert((action.clone(), parameter.key.clone()), input);
                    }
                    ParameterKind::Enum | ParameterKind::Boolean => {
                        parameter_values.insert((action.clone(), parameter.key.clone()), default);
                    }
                }
            }
        }

        let chat_snapshot = chat.snapshot();
        let chat_receiver = chat.subscribe(8);
        let mut listener_tasks = vec![cx.spawn(async move |view, cx| {
            while let Ok(snapshot) = chat_receiver.recv().await {
                if view
                    .update(cx, |view, cx| {
                        view.chat_snapshot = snapshot;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })];
        let mut text_snapshots = HashMap::new();
        for action in &text_actions {
            text_snapshots.insert(action.action(), action.snapshot());
            let receiver = action.subscribe(8);
            listener_tasks.push(cx.spawn(async move |view, cx| {
                while let Ok(snapshot) = receiver.recv().await {
                    if view
                        .update(cx, |view, cx| {
                            view.text_snapshots
                                .insert(snapshot.action.clone(), snapshot);
                            cx.notify();
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        listener_tasks.push(cx.spawn(async move |view, cx| {
            while let Ok(snapshot) = launches.recv().await {
                if view
                    .update(cx, |view, cx| view.apply_launch(snapshot, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));

        let selected_action = chat_action.clone();
        Self {
            controller,
            settings,
            context: context.clone(),
            favorites,
            actions,
            chat,
            chat_action,
            descriptors,
            text_actions,
            selected_action,
            launch_context: context.latest(),
            candidate_confirmed: false,
            explicit_input: None,
            composer,
            parameter_inputs,
            parameter_values,
            chat_snapshot,
            text_snapshots,
            result_tokens: HashMap::new(),
            transient_status: None,
            request_task: None,
            side_effect_task: None,
            _subscriptions: vec![submit_subscription],
            _listener_tasks: listener_tasks,
        }
    }

    fn apply_launch(&mut self, snapshot: ContextSnapshot, cx: &mut Context<Self>) {
        self.launch_context = snapshot;
        self.candidate_confirmed = false;
        self.explicit_input = None;
        self.transient_status = None;
        let settings = self.settings.snapshot();
        if !self.launch_context.is_verified() {
            self.selected_action = self.chat_action.clone();
            cx.notify();
            return;
        }
        let automatic = match settings.settings().launch_route(true) {
            LaunchRoute::Automatic(action) => action,
            LaunchRoute::MissingDefaultAction => {
                self.transient_status = Some(
                    "No default action is configured. Choose an action before sending.".into(),
                );
                cx.notify();
                return;
            }
            LaunchRoute::ActionPalette | LaunchRoute::QuickAsk => {
                cx.notify();
                return;
            }
        };
        let action = automatic.parse::<QualifiedActionId>();
        let Some(descriptor) = action.ok().and_then(|action| {
            self.descriptors
                .iter()
                .find(|descriptor| descriptor.qualified_id() == action)
                .cloned()
        }) else {
            self.transient_status = Some(
                format!("Configured action '{automatic}' is unavailable. Choose an action.").into(),
            );
            cx.notify();
            return;
        };
        self.selected_action = descriptor.qualified_id();
        if let Err(reason) = self.validate_parameters(&descriptor, cx) {
            self.transient_status = Some(
                format!("Automatic action needs attention: {reason}. Choose or complete it.")
                    .into(),
            );
            cx.notify();
            return;
        }
        self.begin_selected(None, cx);
    }

    fn selected_descriptor(&self) -> Option<&ActionDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.qualified_id() == self.selected_action)
    }

    fn parameters(
        &self,
        descriptor: &ActionDescriptor,
        cx: &Context<Self>,
    ) -> BTreeMap<String, String> {
        let mut values = BTreeMap::new();
        let ActionKind::Declarative(definition) = descriptor.kind() else {
            return values;
        };
        let action = descriptor.qualified_id();
        for parameter in &definition.parameters {
            let key = (action.clone(), parameter.key.clone());
            let value = match parameter.kind {
                ParameterKind::Text | ParameterKind::Number => self
                    .parameter_inputs
                    .get(&key)
                    .map(|input| input.read(cx).value().to_string())
                    .unwrap_or_default(),
                ParameterKind::Enum | ParameterKind::Boolean => {
                    self.parameter_values.get(&key).cloned().unwrap_or_default()
                }
            };
            if !value.is_empty() {
                values.insert(parameter.key.clone(), value);
            }
        }
        values
    }

    fn validate_parameters(
        &self,
        descriptor: &ActionDescriptor,
        cx: &Context<Self>,
    ) -> Result<(), String> {
        let ActionKind::Declarative(definition) = descriptor.kind() else {
            return Ok(());
        };
        let values = self.parameters(descriptor, cx);
        for parameter in &definition.parameters {
            if parameter.required
                && values
                    .get(&parameter.key)
                    .is_none_or(|value| value.trim().is_empty())
            {
                return Err(format!("{} is required", parameter.label));
            }
        }
        Ok(())
    }

    fn begin_selected(&mut self, manual: Option<String>, cx: &mut Context<Self>) {
        let Some(descriptor) = self.selected_descriptor().cloned() else {
            self.transient_status = Some("Choose an available action.".into());
            cx.notify();
            return;
        };
        if let Err(reason) = self.validate_parameters(&descriptor, cx) {
            self.transient_status = Some(reason.into());
            cx.notify();
            return;
        }
        let context_ready = self.launch_context.status == CaptureStatus::VerifiedSelection
            || (self.launch_context.status == CaptureStatus::CandidateText
                && self.candidate_confirmed);
        let manual = manual.unwrap_or_default();
        let (input, source, token) = if let Some((input, source)) = &self.explicit_input {
            (input.clone(), *source, None)
        } else if context_ready {
            (
                self.launch_context.text.clone().unwrap_or_default(),
                if self.launch_context.status == CaptureStatus::CandidateText {
                    InputSource::Candidate
                } else {
                    InputSource::Selection
                },
                self.launch_context.replace_token.clone(),
            )
        } else {
            (manual, InputSource::Manual, None)
        };
        if input.trim().is_empty() {
            self.transient_status = Some("Enter text before sending.".into());
            cx.notify();
            return;
        }
        let request = ActionRequest {
            input,
            source,
            parameters: self.parameters(&descriptor, cx),
            context_token: token,
        };
        if let Some(token) = request.context_token.clone() {
            self.result_tokens.insert(descriptor.qualified_id(), token);
        } else {
            self.result_tokens.remove(&descriptor.qualified_id());
        }
        self.transient_status = None;
        let actions = self.actions.clone();
        let action = descriptor.qualified_id();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            if let Err(error) = actions.invoke(action, request).await {
                let _ = view.update(cx, |view, cx| {
                    view.transient_status = Some(error.to_string().into());
                    cx.notify();
                });
            }
        }));
        cx.notify();
    }

    fn selected_text_snapshot(&self) -> Option<&TextActionSnapshot> {
        self.text_snapshots.get(&self.selected_action)
    }

    fn copy_result(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.selected_text_snapshot() else {
            return;
        };
        let text = snapshot.output.clone();
        if text.is_empty() {
            return;
        }
        let context = self.context.clone();
        self.side_effect_task = Some(cx.spawn(async move |view, cx| {
            let result = context.copy(text).await;
            let _ = view.update(cx, |view, cx| {
                view.transient_status = Some(match result {
                    Ok(()) => "Result copied to the clipboard.".into(),
                    Err(error) => error.to_string().into(),
                });
                cx.notify();
            });
        }));
    }

    fn load_clipboard(&mut self, cx: &mut Context<Self>) {
        let context = self.context.clone();
        self.side_effect_task = Some(cx.spawn(async move |view, cx| {
            let result = context.read_clipboard().await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(text) if !text.trim().is_empty() => {
                        view.explicit_input = Some((text, InputSource::Clipboard));
                        view.transient_status =
                            Some("Clipboard text loaded explicitly for this request.".into());
                    }
                    Ok(_) => view.transient_status = Some("Clipboard text is empty.".into()),
                    Err(error) => view.transient_status = Some(error.to_string().into()),
                }
                cx.notify();
            });
        }));
    }

    fn toggle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some(invocation) = self
            .selected_text_snapshot()
            .and_then(|snapshot| snapshot.invocation_id.clone())
        else {
            return;
        };
        let favorites = self.favorites.clone();
        self.side_effect_task = Some(cx.spawn(async move |view, cx| {
            let result = favorites.toggle(invocation).await;
            let _ = view.update(cx, |view, cx| {
                view.transient_status = Some(match result {
                    Ok(true) => "Saved to favorites.".into(),
                    Ok(false) => "Removed from favorites.".into(),
                    Err(error) => format!("Could not update favorite: {error}").into(),
                });
                cx.notify();
            });
        }));
    }

    fn replace_result(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.selected_text_snapshot() else {
            return;
        };
        let Some(token) = self.result_tokens.get(&self.selected_action).cloned() else {
            self.transient_status = Some(
                "This source has no verified replacement target. Copy the result instead.".into(),
            );
            cx.notify();
            return;
        };
        let text = snapshot.output.clone();
        let context = self.context.clone();
        self.side_effect_task = Some(cx.spawn(async move |view, cx| {
            let result = context.replace(token, text).await;
            let _ = view.update(cx, |view, cx| {
                view.transient_status = Some(match result {
                    Ok(_) => {
                        "Original selection replaced. The result remains on the clipboard.".into()
                    }
                    Err(error) => error.to_string().into(),
                });
                cx.notify();
            });
        }));
    }
}

impl Render for QuickShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings_controller = self.controller.clone();
        let hide_controller = self.controller.clone();
        let selected = self.selected_action.clone();
        let action_choices = self.descriptors.iter().map(|descriptor| {
            let action = descriptor.qualified_id();
            let checked = action == selected;
            Radio::new(format!("action-{action}"))
                .label(descriptor.display_name().to_owned())
                .checked(checked)
                .on_change(cx.listener(move |this, checked, _, cx| {
                    if *checked {
                        this.selected_action = action.clone();
                        this.transient_status = None;
                        cx.notify();
                    }
                }))
        });
        let source_text = self
            .explicit_input
            .as_ref()
            .map(|(text, _)| text.clone())
            .or_else(|| self.launch_context.text.clone())
            .unwrap_or_default();
        let context_ready = self.explicit_input.is_some()
            || self.launch_context.status == CaptureStatus::VerifiedSelection
            || (self.launch_context.status == CaptureStatus::CandidateText
                && self.candidate_confirmed);
        let selected_descriptor = self.selected_descriptor().cloned();
        let text_snapshot = self.selected_text_snapshot().cloned();
        let selected_is_chat = self.selected_action == self.chat_action;
        let can_stop = if selected_is_chat {
            self.chat_snapshot.can_stop
        } else {
            text_snapshot
                .as_ref()
                .is_some_and(TextActionSnapshot::can_stop)
        };
        let status = self.transient_status.clone().unwrap_or_else(|| {
            if selected_is_chat {
                self.chat_snapshot.status_text.clone().into()
            } else {
                text_snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.status_text.clone().into())
                    .unwrap_or_else(|| "Ready".into())
            }
        });
        let output = text_snapshot
            .as_ref()
            .map(|snapshot| snapshot.output.clone())
            .unwrap_or_default();
        let invocation = text_snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.invocation_id.as_ref());
        let is_favorite = invocation.is_some_and(|id| self.favorites.contains(id));
        let allow_replace = selected_descriptor.as_ref().is_some_and(|descriptor| {
            matches!(descriptor.kind(), ActionKind::Declarative(definition) if definition.output.allow_replace)
        });
        let allow_clipboard = selected_descriptor.as_ref().is_some_and(|descriptor| {
            matches!(descriptor.kind(), ActionKind::Declarative(definition) if definition.allowed_sources.contains(&lexwisp_core::ActionInputSource::Clipboard))
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(div().text_lg().font_semibold().child("LexWisp Quick Shell"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.launch_context.detail.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                Button::new("quick-settings")
                                    .ghost()
                                    .label("Settings")
                                    .on_click(move |_, _, cx| {
                                        let _ = settings_controller.update(cx, |controller, cx| {
                                            let _ = controller.show_control_center(cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("quick-hide")
                                    .ghost()
                                    .label("Hide")
                                    .on_click(move |_, window, cx| {
                                        let _ = hide_controller.update(cx, |controller, cx| {
                                            controller.hide_quick_shell(window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(action_choices),
            )
            .when(!source_text.is_empty(), |this| {
                this.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if self.explicit_input.is_some() {
                                    "Clipboard text · explicitly loaded"
                                } else if self.launch_context.status == CaptureStatus::CandidateText {
                                    "Candidate text · confirmation required"
                                } else {
                                    "Verified selection"
                                }),
                        )
                        .child(TextView::markdown("source-preview", source_text).selectable(true))
                        .when(
                            self.explicit_input.is_none()
                                && self.launch_context.status == CaptureStatus::CandidateText
                                && !self.candidate_confirmed,
                            |this| {
                                this.child(
                                    Button::new("confirm-candidate")
                                        .label("Use this candidate text")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.candidate_confirmed = true;
                                            this.transient_status = Some(
                                                "Candidate confirmed for this request.".into(),
                                            );
                                            cx.notify();
                                        })),
                                )
                            },
                        ),
                )
            })
            .when_some(selected_descriptor.clone(), |this, descriptor| {
                let ActionKind::Declarative(definition) = descriptor.kind() else {
                    return this;
                };
                let action = descriptor.qualified_id();
                this.children(definition.parameters.iter().map(|parameter| {
                    let key = (action.clone(), parameter.key.clone());
                    let label = div().text_sm().child(parameter.label.clone());
                    match parameter.kind {
                        ParameterKind::Text | ParameterKind::Number => div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label)
                            .children(self.parameter_inputs.get(&key).map(|state| {
                                Input::new(state).aria_label(parameter.label.clone())
                            }))
                            .into_any_element(),
                        ParameterKind::Enum => div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(label)
                            .child(div().flex().gap_3().children(parameter.choices.iter().map(
                                |choice| {
                                    let action = action.clone();
                                    let parameter_key = parameter.key.clone();
                                    let choice_value = choice.clone();
                                    Radio::new(format!("param-{action}-{parameter_key}-{choice}"))
                                        .label(choice.clone())
                                        .checked(
                                            self.parameter_values.get(&(
                                                action.clone(),
                                                parameter_key.clone(),
                                            )) == Some(choice),
                                        )
                                        .on_change(cx.listener(
                                            move |this, checked: &bool, _, cx| {
                                                if *checked {
                                                    this.parameter_values.insert(
                                                        (action.clone(), parameter_key.clone()),
                                                        choice_value.clone(),
                                                    );
                                                    cx.notify();
                                                }
                                            },
                                        ))
                                },
                            )))
                            .into_any_element(),
                        ParameterKind::Boolean => {
                            let checked = self
                                .parameter_values
                                .get(&key)
                                .is_some_and(|value| value == "true");
                            let action = action.clone();
                            let parameter_key = parameter.key.clone();
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .child(label)
                                .child(
                                    Switch::new(format!("param-{action}-{parameter_key}"))
                                        .checked(checked)
                                        .on_change(cx.listener(
                                            move |this, checked: &bool, _, cx| {
                                                this.parameter_values.insert(
                                                    (action.clone(), parameter_key.clone()),
                                                    checked.to_string(),
                                                );
                                                cx.notify();
                                            },
                                        )),
                                )
                                .into_any_element()
                        }
                    }
                }))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .when(selected_is_chat, |this| {
                        this.child(
                            div()
                                .size_full()
                                .overflow_y_scrollbar()
                                .p_3()
                                .children(
                                    self.chat_snapshot
                                        .messages
                                        .clone()
                                        .into_iter()
                                        .map(render_message),
                                ),
                        )
                    })
                    .when(!selected_is_chat, |this| {
                        this.child(
                            div()
                                .size_full()
                                .overflow_y_scrollbar()
                                .p_3()
                                .when(output.is_empty(), |this| {
                                    this.child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("The result will appear here as it streams."),
                                    )
                                })
                                .when(!output.is_empty(), |this| {
                                    this.child(
                                        TextView::markdown("action-result", output.clone())
                                            .selectable(true),
                                    )
                                }),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(!context_ready, |this| {
                        this.child(
                            Textarea::new(&self.composer)
                                .h(px(88.))
                                .aria_label("Text input"),
                        )
                    })
                    .when(allow_clipboard, |this| {
                        this.child(
                            Button::new("use-clipboard")
                                .label("Use clipboard text")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.load_clipboard(cx);
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(
                                        if text_snapshot
                                            .as_ref()
                                            .is_some_and(|snapshot| snapshot.storage == StorageState::Unsaved)
                                        {
                                            cx.theme().danger
                                        } else {
                                            cx.theme().muted_foreground
                                        },
                                    )
                                    .child(status),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .when(!selected_is_chat, |this| {
                                        this.child(
                                            Button::new("action-copy")
                                                .label("Copy")
                                                .disabled(output.is_empty())
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.copy_result(cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("action-favorite")
                                                .label(if is_favorite { "Unfavorite" } else { "Favorite" })
                                                .disabled(invocation.is_none())
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.toggle_favorite(cx);
                                                })),
                                        )
                                        .when(allow_replace, |this| {
                                            this.child(
                                                Button::new("action-replace")
                                                    .label("Replace original")
                                                    .disabled(
                                                        output.is_empty()
                                                            || !self.result_tokens.contains_key(&self.selected_action),
                                                    )
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.replace_result(cx);
                                                    })),
                                            )
                                        })
                                    })
                                    .child(
                                        Button::new("action-stop")
                                            .label("Stop")
                                            .disabled(!can_stop)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let result = if this.selected_action == this.chat_action {
                                                    this.chat.stop().map_err(|error| error.to_string())
                                                } else {
                                                    this.text_actions
                                                        .iter()
                                                        .find(|action| action.action() == this.selected_action)
                                                        .ok_or_else(|| "action is unavailable".to_string())
                                                        .and_then(|action| action.stop().map_err(|error| error.to_string()))
                                                };
                                                if let Err(error) = result {
                                                    this.transient_status = Some(error.into());
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("action-send")
                                            .primary()
                                            .label("Send")
                                            .disabled(can_stop)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                let manual = this.composer.read(cx).value().to_string();
                                                this.begin_selected(Some(manual), cx);
                                            })),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Enter sends · Shift+Enter adds a line · candidate text is never sent without confirmation"),
                    ),
            )
    }
}

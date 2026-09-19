use std::sync::Arc;

use gpui_kit::component::{
    ActiveTheme, Disableable, StyledExt,
    bubble::{Bubble, BubbleVariant},
    button::{Button, ButtonVariants},
    clipboard::Clipboard,
    input::{InputEvent, Textarea, TextareaState},
    message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader},
    message_scroller::{MessageScroller, MessageScrollerState},
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, WeakEntity, Window, div, px,
};
use lexwisp_core::{
    ActionRequest, ActionUiPort, ChatMessageSnapshot, ChatMessageStatus, ChatSnapshot, ChatUiPort,
    QualifiedActionId,
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
            let result = actions.invoke(action, ActionRequest { input }).await;
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

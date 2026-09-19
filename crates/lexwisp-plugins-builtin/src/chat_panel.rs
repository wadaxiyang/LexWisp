use std::{rc::Rc, sync::Arc};

use gpui_kit::base::{StyledExt as _, VirtualListScrollHandle, v_virtual_list};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Selectable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    message_scroller::{MessageScroller, MessageScrollerState},
    radio::Radio,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, WeakEntity, Window, div, px, size,
};
use lexwisp_core::{ChatModelPreference, ChatSnapshot, ChatUiPort, ConversationId, ProviderUiPort};
use lexwisp_ui::SurfaceController;

use crate::view::render_message;

pub struct ChatPanel {
    controller: WeakEntity<SurfaceController>,
    chat: Arc<dyn ChatUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    snapshot: ChatSnapshot,
    composer: Entity<TextareaState>,
    title: Entity<InputState>,
    messages: Entity<MessageScrollerState>,
    conversations_scroll: VirtualListScrollHandle,
    transient_status: Option<SharedString>,
    request_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _listener_task: Task<()>,
}

impl ChatPanel {
    pub fn new(
        controller: WeakEntity<SurfaceController>,
        chat: Arc<dyn ChatUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = chat.snapshot();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Message this conversation…")
                .submit_on_enter(true)
        });
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Conversation title")
                .default_value(snapshot.title.clone())
        });
        let submit = cx.subscribe_in(
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
                    this.begin_send(input, window, cx);
                }
            },
        );
        let messages = cx.new(|cx| MessageScrollerState::new(snapshot.messages.len(), cx));
        let receiver = chat.subscribe(16);
        let listener_task = cx.spawn(async move |view, cx| {
            while let Ok(snapshot) = receiver.recv().await {
                if view
                    .update_in(cx, |view, window, cx| {
                        view.apply_snapshot(snapshot, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            controller,
            chat,
            providers,
            snapshot,
            composer,
            title,
            messages,
            conversations_scroll: VirtualListScrollHandle::new(),
            transient_status: None,
            request_task: None,
            _subscriptions: vec![submit],
            _listener_task: listener_task,
        }
    }

    fn apply_snapshot(
        &mut self,
        snapshot: ChatSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed_conversation = self.snapshot.conversation_id != snapshot.conversation_id;
        let changed_title = self.snapshot.title != snapshot.title;
        let old_count = self.snapshot.messages.len();
        let new_count = snapshot.messages.len();
        self.snapshot = snapshot;
        if changed_conversation || changed_title {
            self.title.update(cx, |title, cx| {
                title.set_value(self.snapshot.title.clone(), window, cx)
            });
        }
        if changed_conversation {
            self.messages
                .update(cx, |messages, cx| messages.reset(new_count, cx));
        } else if new_count > old_count {
            self.messages.update(cx, |messages, cx| {
                let _ = messages.append(new_count - old_count, cx);
            });
        } else if new_count < old_count {
            self.messages
                .update(cx, |messages, cx| messages.reset(new_count, cx));
        } else if new_count > 0 {
            self.messages.update(cx, |messages, cx| {
                let _ = messages.remeasure_items(new_count - 1..new_count, cx);
            });
        }
        cx.notify();
    }

    fn begin_send(&mut self, input: String, window: &mut Window, cx: &mut Context<Self>) {
        if !self.snapshot.can_send || input.trim().is_empty() {
            return;
        }
        self.transient_status = None;
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        let chat = self.chat.clone();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            let result = chat.send(input).await;
            let _ = view.update(cx, |view, cx| {
                view.request_task = None;
                if let Err(error) = result {
                    view.transient_status = Some(error.to_string().into());
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn begin_regenerate(&mut self, cx: &mut Context<Self>) {
        if !self.snapshot.can_retry {
            return;
        }
        self.transient_status = None;
        let chat = self.chat.clone();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            let result = chat.retry().await;
            let _ = view.update(cx, |view, cx| {
                view.request_task = None;
                if let Err(error) = result {
                    view.transient_status = Some(error.to_string().into());
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn switch_conversation(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.switch_conversation(&id) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn create_conversation(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.create_conversation() {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn save_title(&mut self, cx: &mut Context<Self>) {
        let title = self.title.read(cx).value().to_string();
        if let Err(error) = self.chat.rename_conversation(title) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn confirm_delete(&self, window: &mut Window, cx: &mut Context<Self>) {
        let chat = self.chat.clone();
        let id = self.snapshot.conversation_id.clone();
        let title = self.snapshot.title.clone();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let chat = chat.clone();
            let id = id.clone();
            dialog
                .title(format!("Delete “{title}”?"))
                .description("Its saved messages and attempts will be removed. An active reply is stopped first.")
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Delete")
                        .ok_variant(ButtonVariant::Danger)
                        .show_cancel(true)
                        .on_ok(move |_, _, _| chat.delete_conversation(&id).is_ok()),
                )
        });
    }

    fn set_model(&mut self, preference: ChatModelPreference, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.set_model_preference(preference) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn render_conversation_row(&mut self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(conversation) = self.snapshot.conversations.get(ix).cloned() else {
            return div().into_any_element();
        };
        let id = conversation.id().clone();
        let selected = id == self.snapshot.conversation_id;
        let label = if conversation.is_generating() {
            format!("{} · generating", conversation.title())
        } else {
            conversation.title().to_owned()
        };
        div()
            .w_full()
            .px_2()
            .py_1()
            .child(
                Button::new(format!("chat-conversation-{id}"))
                    .ghost()
                    .small()
                    .w_full()
                    .justify_start()
                    .selected(selected)
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.switch_conversation(id.clone(), cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for ChatPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let conversation_sizes = Rc::new(vec![
            size(px(1.), window.rem_size() * 2.5);
            self.snapshot.conversations.len()
        ]);
        let close_controller = self.controller.clone();
        let provider = self.providers.snapshot();
        let model_name = provider.model_id.clone();
        let status = self
            .transient_status
            .clone()
            .unwrap_or_else(|| self.snapshot.status_text.clone().into());
        let messages = self.snapshot.messages.clone();
        let message_scroller = self.messages.clone();
        let active_preference = self.snapshot.model_preference.clone();

        div()
            .size_full()
            .flex()
            .items_stretch()
            .child(
                div()
                    .w_64()
                    .min_w_48()
                    .max_w_80()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(cx.theme().sidebar)
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().font_semibold().child("Conversations"))
                            .child(
                                Button::new("new-conversation")
                                    .label("New chat")
                                    .small()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.create_conversation(cx)
                                    })),
                            ),
                    )
                    .child(
                        div().flex_1().min_h_0().child(
                            v_virtual_list(
                                cx.entity(),
                                "conversation-list",
                                conversation_sizes,
                                |this, range, _, cx| {
                                    range
                                        .map(|ix| this.render_conversation_row(ix, cx))
                                        .collect::<Vec<_>>()
                                },
                            )
                            .track_scroll(&self.conversations_scroll),
                        ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().flex_1().min_w_0().child(Input::new(&self.title)))
                            .child(
                                Button::new("save-conversation-title")
                                    .label("Rename")
                                    .on_click(cx.listener(|this, _, _, cx| this.save_title(cx))),
                            )
                            .child(
                                Button::new("delete-conversation")
                                    .danger()
                                    .outline()
                                    .label("Delete")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.confirm_delete(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("close-chat-panel")
                                    .ghost()
                                    .label("Close")
                                    .on_click(move |_, window, cx| {
                                        let _ = close_controller.update(cx, |controller, cx| {
                                            controller.close_chat_panel(window, cx)
                                        });
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_3()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .child(div().text_sm().child("Model"))
                            .child(
                                Radio::new("chat-model-fast")
                                    .label("Fast")
                                    .checked(matches!(active_preference, ChatModelPreference::Fast))
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        if *checked {
                                            this.set_model(ChatModelPreference::Fast, cx)
                                        }
                                    })),
                            )
                            .child(
                                Radio::new("chat-model-smart")
                                    .label("Smart")
                                    .checked(matches!(active_preference, ChatModelPreference::Smart))
                                    .on_change(cx.listener(|this, checked, _, cx| {
                                        if *checked {
                                            this.set_model(ChatModelPreference::Smart, cx)
                                        }
                                    })),
                            )
                            .when(provider.configured && !model_name.is_empty(), |this| {
                                let checked = matches!(
                                    &active_preference,
                                    ChatModelPreference::Model(model) if model == &model_name
                                );
                                this.child(
                                    Radio::new("chat-model-concrete")
                                        .label(model_name.clone())
                                        .checked(checked)
                                        .on_change(cx.listener(move |this, checked, _, cx| {
                                            if *checked {
                                                this.set_model(
                                                    ChatModelPreference::Model(model_name.clone()),
                                                    cx,
                                                )
                                            }
                                        })),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(
                                MessageScroller::new(
                                    "chat-panel-transcript",
                                    message_scroller,
                                    move |ix, _, _| {
                                        messages
                                            .get(ix)
                                            .map(|message| render_message(message.as_ref().clone()))
                                            .unwrap_or_else(|| div().into_any_element())
                                    },
                                )
                                .with_jump_button_label("Jump to latest"),
                            ),
                    )
                    .when_some(self.snapshot.context_notice.clone(), |this, notice| {
                        this.child(
                            div()
                                .px_4()
                                .py_2()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(notice),
                        )
                    })
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(Textarea::new(&self.composer).h_24().aria_label("Chat message"))
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
                                                Button::new("regenerate-last")
                                                    .label("Regenerate")
                                                    .disabled(!self.snapshot.can_retry)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.begin_regenerate(cx)
                                                    })),
                                            )
                                            .child(
                                                Button::new("stop-chat-panel")
                                                    .label("Stop")
                                                    .disabled(!self.snapshot.can_stop)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        if let Err(error) = this.chat.stop() {
                                                            this.transient_status =
                                                                Some(error.to_string().into());
                                                            cx.notify();
                                                        }
                                                    })),
                                            )
                                            .child(
                                                Button::new("send-chat-panel")
                                                    .primary()
                                                    .label("Send")
                                                    .disabled(!self.snapshot.can_send)
                                                    .on_click(cx.listener(|this, _, window, cx| {
                                                        let input = this
                                                            .composer
                                                            .read(cx)
                                                            .value()
                                                            .to_string();
                                                        this.begin_send(input, window, cx)
                                                    })),
                                            ),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child("Enter sends · Shift+Enter adds a line · closing this panel keeps active replies running"),
                            ),
                    ),
            )
    }
}

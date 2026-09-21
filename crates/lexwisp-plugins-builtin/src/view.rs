use std::{path::PathBuf, rc::Rc, sync::Arc};

use gpui_kit::base::{VirtualListScrollHandle, v_virtual_list};
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Selectable, Sizable, StyledExt, WindowExt,
    attachment::{
        Attachment, AttachmentActions, AttachmentContent, AttachmentDescription, AttachmentGroup,
        AttachmentMedia, AttachmentTitle,
    },
    bubble::{Bubble, BubbleVariant},
    button::{Button, ButtonVariant, ButtonVariants},
    clipboard::Clipboard,
    dialog::DialogButtonProps,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    message::{Message, MessageAlignment, MessageContent, MessageFooter, MessageHeader},
    message_scroller::{MessageScroller, MessageScrollerState},
    popover::Popover,
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AppContext, Context, Entity, ExternalPaths, InteractiveElement, IntoElement, ParentElement,
    PathPromptOptions, Render, SharedString, Styled, Subscription, Task, WeakEntity, Window, div,
    rems, size,
};
use lexwisp_core::{
    ChatAttachment, ChatDraft, ChatMessageSnapshot, ChatMessageStatus, ChatModelPreference,
    ChatSnapshot, ChatUiPort, ContextSnapshot, ConversationId, ProviderUiPort, ShellPresentation,
};
use lexwisp_ui::{ShellSession, SurfaceController};

const MAX_ATTACHMENTS: usize = 8;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 512 * 1024;

struct DraftAttachment {
    id: u64,
    path: PathBuf,
    value: ChatAttachment,
}

impl DraftAttachment {
    fn name(&self) -> &str {
        self.value.name()
    }

    fn description(&self) -> &'static str {
        match self.value.content() {
            lexwisp_core::ChatAttachmentContent::Image { .. } => "Image",
            lexwisp_core::ChatAttachmentContent::Text { .. } => "Text file",
        }
    }

    fn is_image(&self) -> bool {
        matches!(
            self.value.content(),
            lexwisp_core::ChatAttachmentContent::Image { .. }
        )
    }
}

pub struct ChatExperience {
    controller: WeakEntity<SurfaceController>,
    session: Entity<ShellSession>,
    chat: Arc<dyn ChatUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    snapshot: ChatSnapshot,
    composer: Entity<TextareaState>,
    rename_input: Entity<InputState>,
    messages: Entity<MessageScrollerState>,
    conversations_scroll: VirtualListScrollHandle,
    context: Option<ContextSnapshot>,
    attachments: Vec<DraftAttachment>,
    next_attachment_id: u64,
    transient_status: Option<SharedString>,
    request_task: Option<Task<()>>,
    attachment_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
    _snapshot_task: Task<()>,
}

impl ChatExperience {
    pub fn new(
        controller: WeakEntity<SurfaceController>,
        session: Entity<ShellSession>,
        chat: Arc<dyn ChatUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let snapshot = chat.snapshot();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask LexWisp…")
                .submit_on_enter(true)
        });
        let rename_input =
            cx.new(|cx| InputState::new(window, cx).default_value(snapshot.title.clone()));
        let submit = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    shift: false,
                    secondary: false,
                } => {
                    let input = state.read(cx).value().to_string();
                    this.begin_send(input, window, cx);
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        );
        let session_observer = cx.observe(&session, |this, session, cx| {
            let launch_context = session.read(cx).launch_context().clone();
            this.context = launch_context.is_verified().then_some(launch_context);
            cx.notify();
        });
        let messages = cx.new(|cx| MessageScrollerState::new(snapshot.messages.len(), cx));
        let receiver = chat.subscribe(16);
        let snapshot_task = cx.spawn(async move |view, cx| {
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
        let initial_context = session.read(cx).launch_context().clone();
        composer.update(cx, |composer, cx| composer.focus(window, cx));
        Self {
            controller,
            session,
            chat,
            providers,
            snapshot,
            composer,
            rename_input,
            messages,
            conversations_scroll: VirtualListScrollHandle::new(),
            context: initial_context.is_verified().then_some(initial_context),
            attachments: Vec::new(),
            next_attachment_id: 0,
            transient_status: None,
            request_task: None,
            attachment_task: None,
            _subscriptions: vec![submit, session_observer],
            _snapshot_task: snapshot_task,
        }
    }

    fn apply_snapshot(
        &mut self,
        snapshot: ChatSnapshot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if snapshot.generation <= self.snapshot.generation {
            return;
        }
        let changed_conversation = self.snapshot.conversation_id != snapshot.conversation_id;
        let changed_title = self.snapshot.title != snapshot.title;
        let old_count = self.snapshot.messages.len();
        let new_count = snapshot.messages.len();
        self.snapshot = snapshot;
        if changed_conversation || changed_title {
            self.rename_input.update(cx, |title, cx| {
                title.set_value(self.snapshot.title.clone(), window, cx)
            });
        }
        self.messages.update(cx, |messages, cx| {
            if changed_conversation || new_count < old_count {
                messages.reset(new_count, cx);
            } else if new_count > old_count {
                let _ = messages.append(new_count - old_count, cx);
            } else if new_count > 0 {
                let _ = messages.remeasure_items(new_count - 1..new_count, cx);
            }
        });
        cx.notify();
    }

    fn begin_send(&mut self, input: String, window: &mut Window, cx: &mut Context<Self>) {
        let has_context = self.context.is_some();
        if !self.snapshot.can_send
            || (input.trim().is_empty() && self.attachments.is_empty() && !has_context)
        {
            return;
        }
        let presentation = self.session.read(cx).presentation();
        if presentation == ShellPresentation::Compact {
            let _ = self.controller.update(cx, |controller, cx| {
                controller.set_main_shell_presentation(ShellPresentation::Expanded, cx)
            });
        }

        let mut attachments = self
            .attachments
            .drain(..)
            .map(|attachment| attachment.value)
            .collect::<Vec<_>>();
        if let Some(context) = self.context.take()
            && let Some(text) = context.text
        {
            attachments.push(ChatAttachment::text(
                format!("selection-{}", context.captured_at_ms),
                "Selected text",
                text,
            ));
        }
        let draft = ChatDraft::new(input, attachments);
        self.transient_status = None;
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        let chat = self.chat.clone();
        self.request_task = Some(cx.spawn(async move |view, cx| {
            let result = chat.send_draft(draft).await;
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

    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.stop() {
            self.transient_status = Some(error.to_string().into());
        }
        cx.notify();
    }

    fn switch_conversation(&mut self, id: ConversationId, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.switch_conversation(&id) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn create_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.create_conversation() {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.composer
            .update(cx, |composer, cx| composer.set_value("", window, cx));
        self.attachments.clear();
        self.context = None;
        self.composer.update(cx, |composer, cx| {
            composer.focus(window, cx);
        });
        cx.notify();
    }

    fn open_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_input.update(cx, |title, cx| {
            title.set_value(self.snapshot.title.clone(), window, cx)
        });
        let chat = self.chat.clone();
        let title = self.rename_input.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let title_for_save = title.clone();
            let chat_for_save = chat.clone();
            dialog
                .title("Rename conversation")
                .child(Input::new(&title).aria_label("Conversation title"))
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Rename")
                        .show_cancel(true)
                        .on_ok(move |_, _, cx| {
                            let next = title_for_save.read(cx).value().to_string();
                            chat_for_save.rename_conversation(next).is_ok()
                        }),
                )
        });
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
                .description(
                    "Its saved messages and attempts will be removed. An active reply is stopped first.",
                )
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

    fn choose_attachments(&mut self, cx: &mut Context<Self>) {
        if self.attachment_task.is_some() {
            return;
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images or text files".into()),
        });
        self.attachment_task = Some(cx.spawn(async move |view, cx| match receiver.await {
            Ok(Ok(Some(paths))) => {
                let _ = view.update(cx, |view, cx| view.load_attachments(paths, cx));
            }
            Ok(Ok(None)) => {
                let _ = view.update(cx, |view, cx| {
                    view.attachment_task = None;
                    cx.notify();
                });
            }
            Ok(Err(error)) => {
                let _ = view.update(cx, |view, cx| {
                    view.attachment_task = None;
                    view.transient_status =
                        Some(format!("Couldn’t open the file picker: {error}").into());
                    cx.notify();
                });
            }
            Err(_) => {
                let _ = view.update(cx, |view, cx| {
                    view.attachment_task = None;
                    view.transient_status = Some("The file picker closed unexpectedly.".into());
                    cx.notify();
                });
            }
        }));
    }

    fn load_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let remaining = MAX_ATTACHMENTS.saturating_sub(self.attachments.len());
        if remaining == 0 {
            self.attachment_task = None;
            self.transient_status = Some("Remove an attachment before adding another.".into());
            cx.notify();
            return;
        }
        let paths = paths.into_iter().take(remaining).collect::<Vec<_>>();
        let first_id = self.next_attachment_id;
        self.next_attachment_id = self
            .next_attachment_id
            .saturating_add(paths.len().try_into().unwrap_or(u64::MAX));
        let load = cx.background_spawn(async move { load_attachment_files(paths, first_id) });
        self.attachment_task = Some(cx.spawn(async move |view, cx| {
            let loaded = load.await;
            let _ = view.update(cx, |view, cx| {
                view.attachment_task = None;
                for item in loaded {
                    match item {
                        Ok(attachment) => view.attachments.push(attachment),
                        Err(error) => view.transient_status = Some(error.into()),
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn remove_attachment(&mut self, id: u64, cx: &mut Context<Self>) {
        self.attachments.retain(|attachment| attachment.id != id);
        cx.notify();
    }

    fn remove_context(&mut self, cx: &mut Context<Self>) {
        self.context = None;
        cx.notify();
    }

    fn set_presentation(&mut self, presentation: ShellPresentation, cx: &mut Context<Self>) {
        if let Err(error) = self.controller.update(cx, |controller, cx| {
            controller.set_main_shell_presentation(presentation, cx)
        }) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, cx| {
            controller.hide_main_shell_from_view(window, cx);
        });
    }

    fn render_conversation_row(
        &mut self,
        ix: usize,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
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
        Button::new(format!("conversation-{id}"))
            .ghost()
            .small()
            .w_full()
            .justify_start()
            .selected(selected)
            .label(label)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.switch_conversation(id.clone(), cx);
            }))
            .into_any_element()
    }

    fn render_model_selector(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let active = self.snapshot.model_preference.clone();
        let provider = self.providers.snapshot();
        let view = cx.weak_entity();
        Popover::new("chat-model-selector")
            .trigger(
                Button::new("chat-model-trigger")
                    .ghost()
                    .small()
                    .label(active.display_name().to_owned())
                    .tooltip("Choose model"),
            )
            .content(move |popover, window, cx| {
                let popup = cx.weak_entity();
                let option = |id: &'static str,
                              label: String,
                              preference: ChatModelPreference,
                              selected: bool| {
                    let view = view.clone();
                    let popup = popup.clone();
                    Button::new(id)
                        .ghost()
                        .small()
                        .w_full()
                        .justify_start()
                        .selected(selected)
                        .label(label)
                        .on_click(move |_, window, cx| {
                            let _ =
                                view.update(cx, |view, cx| view.set_model(preference.clone(), cx));
                            let _ = popup.update(cx, |popup, cx| popup.dismiss(window, cx));
                        })
                };
                div()
                    .w_48()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(option(
                        "model-fast",
                        "Fast".into(),
                        ChatModelPreference::Fast,
                        matches!(active, ChatModelPreference::Fast),
                    ))
                    .child(option(
                        "model-smart",
                        "Smart".into(),
                        ChatModelPreference::Smart,
                        matches!(active, ChatModelPreference::Smart),
                    ))
                    .when(
                        provider.configured && !provider.model_id.is_empty(),
                        |this| {
                            let preference = ChatModelPreference::Model(provider.model_id.clone());
                            let selected = active == preference;
                            this.child(option(
                                "model-configured",
                                provider.model_id.clone(),
                                preference,
                                selected,
                            ))
                        },
                    )
                    .map(|content| {
                        let _ = popover;
                        let _ = window;
                        content
                    })
            })
            .into_any_element()
    }

    fn render_context_chip(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        let context = self.context.as_ref()?;
        let preview = context
            .text
            .as_deref()
            .unwrap_or_default()
            .split_whitespace()
            .take(8)
            .collect::<Vec<_>>()
            .join(" ");
        Some(
            div()
                .max_w_full()
                .flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(cx.theme().radius_full())
                .bg(cx.theme().muted)
                .text_xs()
                .child(div().truncate().child(if preview.is_empty() {
                    "Selected text".to_owned()
                } else {
                    format!("Selected text · {preview}")
                }))
                .child(
                    Button::new("remove-selection-context")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("Remove selected text")
                        .on_click(cx.listener(|this, _, _, cx| this.remove_context(cx))),
                )
                .into_any_element(),
        )
    }

    fn render_attachments(&self, cx: &mut Context<Self>) -> Option<gpui_kit::AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        Some(
            AttachmentGroup::new("draft-attachments")
                .children(self.attachments.iter().map(|attachment| {
                    let id = attachment.id;
                    let media = if attachment.is_image() {
                        AttachmentMedia::new().src(attachment.path.clone())
                    } else {
                        AttachmentMedia::new().child("TXT")
                    };
                    Attachment::new()
                        .id(format!("draft-attachment-{id}"))
                        .small()
                        .media(media)
                        .content(
                            AttachmentContent::new()
                                .title(AttachmentTitle::new(attachment.name().to_owned()))
                                .description(AttachmentDescription::new(attachment.description())),
                        )
                        .actions(
                            AttachmentActions::new().child(
                                Button::new(format!("remove-attachment-{id}"))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip(format!("Remove {}", attachment.name()))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_attachment(id, cx)
                                    })),
                            ),
                        )
                }))
                .into_any_element(),
        )
    }

    fn render_composer(&mut self, compact: bool, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let input = self.composer.read(cx).value().to_string();
        let send_enabled = self.snapshot.can_send
            && (!input.trim().is_empty() || !self.attachments.is_empty() || self.context.is_some());
        let status = self
            .transient_status
            .clone()
            .unwrap_or_else(|| self.snapshot.status_text.clone().into());
        let attaching = self.attachment_task.is_some();

        div()
            .id("chat-composer-drop-zone")
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(cx.theme().radius_2xl())
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.load_attachments(paths.paths().to_vec(), cx);
            }))
            .children(self.render_context_chip(cx))
            .children(self.render_attachments(cx))
            .child(
                Textarea::new(&self.composer)
                    .h(if compact { rems(4.) } else { rems(5.5) })
                    .aria_label("Message composer"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("attach-files")
                            .ghost()
                            .small()
                            .icon(IconName::File)
                            .tooltip("Attach images or text files")
                            .disabled(attaching || self.attachments.len() >= MAX_ATTACHMENTS)
                            .on_click(cx.listener(|this, _, _, cx| this.choose_attachments(cx))),
                    )
                    .child(self.render_model_selector(cx))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .truncate()
                            .text_color(if self.snapshot.has_unsaved_result {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(status),
                    )
                    .when(self.snapshot.can_retry && !self.snapshot.can_stop, |this| {
                        this.child(
                            Button::new("regenerate-last")
                                .ghost()
                                .small()
                                .icon(IconName::RotateCw)
                                .tooltip("Regenerate last answer")
                                .on_click(cx.listener(|this, _, _, cx| this.begin_regenerate(cx))),
                        )
                    })
                    .when(self.snapshot.can_stop, |this| {
                        this.child(
                            Button::new("stop-response")
                                .outline()
                                .small()
                                .icon(IconName::Pause)
                                .tooltip("Stop generating")
                                .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                        )
                    })
                    .child(
                        Button::new("send-message")
                            .primary()
                            .small()
                            .icon(IconName::ArrowUp)
                            .tooltip("Send message")
                            .disabled(!send_enabled)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let input = this.composer.read(cx).value().to_string();
                                this.begin_send(input, window, cx);
                            })),
                    ),
            )
            .into_any_element()
    }

    fn render_header(
        &mut self,
        presentation: ShellPresentation,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().font_semibold().child("LexWisp"))
            .when(presentation != ShellPresentation::Compact, |this| {
                this.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.snapshot.title.clone()),
                )
            })
            .when(presentation == ShellPresentation::Compact, |this| {
                this.child(div().flex_1())
            })
            .when(presentation == ShellPresentation::Expanded, |this| {
                this.child(
                    Button::new("open-workspace")
                        .ghost()
                        .small()
                        .icon(IconName::PanelLeftOpen)
                        .tooltip("Open workspace")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_presentation(ShellPresentation::Workspace, cx)
                        })),
                )
            })
            .child(
                Button::new("hide-main-shell")
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Hide LexWisp")
                    .on_click(cx.listener(|this, _, window, cx| this.hide(window, cx))),
            )
            .into_any_element()
    }

    fn render_transcript(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.snapshot.messages.is_empty() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .max_w(rems(30.))
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .text_center()
                        .child(div().text_lg().font_semibold().child("How can I help?"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(if self.providers.snapshot().configured {
                                    "Start a conversation or attach an image, Markdown, text, or code file."
                                } else {
                                    "Configure a provider in Settings, then start your first conversation."
                                }),
                        ),
                )
                .into_any_element();
        }
        let messages = self.snapshot.messages.clone();
        MessageScroller::new(
            "main-shell-transcript",
            self.messages.clone(),
            move |ix, _, _| {
                messages
                    .get(ix)
                    .map(|message| render_message(message.as_ref().clone()))
                    .unwrap_or_else(|| div().into_any_element())
            },
        )
        .with_jump_button_label("Jump to latest")
        .into_any_element()
    }

    fn render_compact(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_header(ShellPresentation::Compact, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .pb_3()
                    .pt_2()
                    .child(self.render_composer(true, cx)),
            )
            .into_any_element()
    }

    fn render_expanded(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(self.render_header(ShellPresentation::Expanded, cx))
            .child(div().flex_1().min_h_0().child(self.render_transcript(cx)))
            .children(self.snapshot.context_notice.clone().map(|notice| {
                div()
                    .px_4()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(notice)
            }))
            .child(div().px_3().pb_3().child(self.render_composer(false, cx)))
            .into_any_element()
    }

    fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let conversation_sizes = Rc::new(vec![
            size(gpui_kit::px(1.), window.rem_size() * 2.5);
            self.snapshot.conversations.len()
        ]);
        div()
            .w_64()
            .min_w_56()
            .max_w_72()
            .h_full()
            .flex()
            .flex_col()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().font_semibold().child("LexWisp"))
                    .child(
                        Button::new("collapse-workspace")
                            .ghost()
                            .small()
                            .icon(IconName::PanelLeftClose)
                            .tooltip("Collapse workspace")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_presentation(ShellPresentation::Expanded, cx)
                            })),
                    ),
            )
            .child(
                div().px_2().pb_2().child(
                    Button::new("new-conversation")
                        .w_full()
                        .justify_start()
                        .icon(IconName::Plus)
                        .label("New chat")
                        .on_click(
                            cx.listener(|this, _, window, cx| this.create_conversation(window, cx)),
                        ),
                ),
            )
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Conversations"),
            )
            .child(
                div().flex_1().min_h_0().child(
                    v_virtual_list(
                        cx.entity(),
                        "workspace-conversation-list",
                        conversation_sizes,
                        |this, range, _, cx| {
                            range
                                .map(|ix| this.render_conversation_row(ix, cx))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.conversations_scroll),
                ),
            )
            .child(
                div()
                    .px_2()
                    .py_2()
                    .flex()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("rename-conversation")
                            .ghost()
                            .small()
                            .icon(IconName::FileText)
                            .tooltip("Rename conversation")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_rename_dialog(window, cx)
                            })),
                    )
                    .child(
                        Button::new("delete-conversation")
                            .ghost()
                            .small()
                            .icon(IconName::Delete)
                            .tooltip("Delete conversation")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.confirm_delete(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("workspace-settings")
                            .ghost()
                            .small()
                            .icon(IconName::Settings)
                            .tooltip("Settings")
                            .on_click({
                                let controller = self.controller.clone();
                                move |_, _, cx| {
                                    let _ = controller.update(cx, |controller, cx| {
                                        let _ = controller.show_control_center(cx);
                                    });
                                }
                            }),
                    ),
            )
            .map(|sidebar| {
                let _ = window;
                sidebar.into_any_element()
            })
    }

    fn render_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        div()
            .size_full()
            .flex()
            .items_stretch()
            .child(self.render_sidebar(window, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(self.render_header(ShellPresentation::Workspace, cx))
                    .child(div().flex_1().min_h_0().child(self.render_transcript(cx)))
                    .children(self.snapshot.context_notice.clone().map(|notice| {
                        div()
                            .px_6()
                            .py_1()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(notice)
                    }))
                    .child(
                        div().w_full().flex().justify_center().px_6().pb_4().child(
                            div()
                                .w_full()
                                .max_w(rems(52.))
                                .child(self.render_composer(false, cx)),
                        ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ChatExperience {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.session.read(cx).presentation() {
            ShellPresentation::Compact => self.render_compact(cx),
            ShellPresentation::Expanded => self.render_expanded(cx),
            ShellPresentation::Workspace => self.render_workspace(window, cx),
        }
    }
}

pub(crate) fn render_message(message: ChatMessageSnapshot) -> gpui_kit::AnyElement {
    let id = message.id.as_str().to_owned();
    let mut content = if message.content.is_empty() {
        if message.attachments.is_empty() {
            "Waiting for the provider…".to_owned()
        } else {
            String::new()
        }
    } else {
        message.content.clone()
    };
    if !message.attachments.is_empty() {
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str("Attachments: ");
        content.push_str(
            &message
                .attachments
                .iter()
                .map(|attachment| format!("`{}`", attachment.name()))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
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
    let code_blocks = fenced_code_blocks(&message.content);
    let footer = if message.is_user {
        MessageFooter::new().child(status)
    } else {
        MessageFooter::new()
            .child(status)
            .child(
                Clipboard::new(format!("copy-message-{id}"))
                    .value(content)
                    .tooltip("Copy answer"),
            )
            .children(code_blocks.into_iter().enumerate().map(|(ix, code)| {
                Clipboard::new(format!("copy-code-{id}-{ix}"))
                    .value(code)
                    .tooltip(format!("Copy code block {}", ix + 1))
            }))
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
        .into_any_element()
}

fn load_attachment_files(
    paths: Vec<PathBuf>,
    first_id: u64,
) -> Vec<Result<DraftAttachment, String>> {
    paths
        .into_iter()
        .enumerate()
        .map(|(ix, path)| {
            let id = first_id.saturating_add(ix.try_into().unwrap_or(u64::MAX));
            load_attachment_file(id, path)
        })
        .collect()
}

fn load_attachment_file(id: u64, path: PathBuf) -> Result<DraftAttachment, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "The selected file has no usable name.".to_owned())?
        .to_owned();
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("Couldn’t inspect “{name}”: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("“{name}” is not a file."));
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let image_type = image_media_type(&extension);
    let limit = if image_type.is_some() {
        MAX_IMAGE_BYTES
    } else if is_text_extension(&extension) {
        MAX_TEXT_BYTES
    } else {
        return Err(format!(
            "“{name}” isn’t supported yet. Attach an image, Markdown, text, or code file."
        ));
    };
    if metadata.len() > limit {
        return Err(format!(
            "“{name}” is too large. Images are limited to 8 MiB and text files to 512 KiB."
        ));
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("Couldn’t read “{name}”: {error}"))?;
    let value = if let Some(media_type) = image_type {
        ChatAttachment::image(id.to_string(), name, media_type, Arc::<[u8]>::from(bytes))
    } else {
        let text =
            String::from_utf8(bytes).map_err(|_| format!("“{name}” is not valid UTF-8 text."))?;
        ChatAttachment::text(id.to_string(), name, text)
    };
    Ok(DraftAttachment { id, path, value })
}

fn image_media_type(extension: &str) -> Option<&'static str> {
    match extension {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

fn is_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        "txt"
            | "md"
            | "markdown"
            | "rs"
            | "js"
            | "jsx"
            | "ts"
            | "tsx"
            | "json"
            | "toml"
            | "yaml"
            | "yml"
            | "py"
            | "go"
            | "java"
            | "c"
            | "cc"
            | "cpp"
            | "h"
            | "hpp"
            | "css"
            | "html"
            | "xml"
            | "csv"
            | "log"
    )
}

fn fenced_code_blocks(markdown: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current = None::<String>;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            if let Some(code) = current.take() {
                blocks.push(code.trim_end_matches('\n').to_owned());
            } else {
                current = Some(String::new());
            }
        } else if let Some(code) = current.as_mut() {
            code.push_str(line);
            code.push('\n');
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::{fenced_code_blocks, image_media_type, is_text_extension};

    #[test]
    fn fenced_code_blocks_are_independently_copyable() {
        assert_eq!(
            fenced_code_blocks("Before\n```rust\nlet answer = 42;\n```\nAfter"),
            vec!["let answer = 42;"]
        );
        assert!(fenced_code_blocks("```unterminated").is_empty());
    }

    #[test]
    fn attachment_types_are_explicitly_bounded() {
        assert_eq!(image_media_type("png"), Some("image/png"));
        assert!(is_text_extension("rs"));
        assert!(!is_text_extension("pdf"));
    }
}

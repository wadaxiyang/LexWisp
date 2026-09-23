use std::{path::PathBuf, sync::Arc};

use gpui_kit::assets::IconName as AssetIconName;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, StyledExt, WindowExt,
    attachment::{
        Attachment, AttachmentActions, AttachmentContent, AttachmentDescription, AttachmentGroup,
        AttachmentMedia, AttachmentTitle,
    },
    button::{Button, ButtonVariant, ButtonVariants},
    clipboard::Clipboard,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    message_scroller::{MessageScroller, MessageScrollerState},
    scroll::ScrollableElement,
    text::{TextView, TextViewStyle},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    App, AppContext, Context, Entity, ExternalPaths, Focusable, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, PathPromptOptions, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, WeakEntity, Window, WindowControlArea, actions, div, px, rems,
};
use lexwisp_core::{
    ChatAttachment, ChatConversationSummary, ChatDraft, ChatMessageSnapshot, ChatMessageStatus,
    ChatModelPreference, ChatSnapshot, ChatUiPort, ConversationId,
};

use crate::{SettingsView, SurfaceController, SurfaceServices, ui_metrics};

actions!(
    lexwisp_main_window,
    [OpenSwitcher, NewChat, OpenHistory, Dismiss, TogglePin]
);
const KEY_CONTEXT: &str = "LexWispMainWindow";
const MAX_ATTACHMENTS: usize = 8;
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 512 * 1024;
const HISTORY_PAGE_SIZE: usize = 100;

pub fn register_shortcuts(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenSwitcher, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-n", NewChat, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-h", OpenHistory, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-p", TogglePin, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Page {
    Chat,
    History,
    Settings,
    About,
}

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
    chat: Arc<dyn ChatUiPort>,
    snapshot: ChatSnapshot,
    composer: Entity<TextareaState>,
    rename_input: Entity<InputState>,
    search: Entity<InputState>,
    messages: Entity<MessageScrollerState>,
    settings_view: Entity<SettingsView>,
    page: Page,
    switcher_open: bool,
    model_open: bool,
    favorites_only: bool,
    history_limit: usize,
    pinned: bool,
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
        services: SurfaceServices,
        pinned: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let SurfaceServices {
            chat,
            providers,
            settings,
            history,
        } = services;
        let snapshot = chat.snapshot();
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask LexWisp…")
                .submit_on_enter(true)
        });
        let rename_input =
            cx.new(|cx| InputState::new(window, cx).default_value(snapshot.title.clone()));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search conversations…"));
        let messages = cx.new(|cx| MessageScrollerState::new(snapshot.messages.len(), cx));
        let settings_view =
            cx.new(|cx| SettingsView::new(settings, providers.clone(), history, window, cx));
        let submit = cx.subscribe_in(
            &composer,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter {
                    shift: false,
                    secondary: false,
                } => {
                    this.begin_send(state.read(cx).value().to_string(), window, cx);
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        );
        let search_submit = cx.subscribe_in(
            &search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.history_limit = HISTORY_PAGE_SIZE;
                    cx.notify();
                }
                InputEvent::PressEnter { shift: false, .. } => {
                    if let Some(item) = this.filtered_conversations(cx).first() {
                        this.switch_conversation(item.id().clone(), window, cx);
                    }
                }
                _ => {}
            },
        );
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
        composer.update(cx, |composer, cx| composer.focus(window, cx));
        Self {
            controller,
            chat,
            snapshot,
            composer,
            rename_input,
            search,
            messages,
            settings_view,
            page: Page::Chat,
            switcher_open: false,
            model_open: false,
            favorites_only: false,
            history_limit: HISTORY_PAGE_SIZE,
            pinned,
            attachments: Vec::new(),
            next_attachment_id: 0,
            transient_status: None,
            request_task: None,
            attachment_task: None,
            _subscriptions: vec![submit, search_submit],
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
        if !self.snapshot.can_send || (input.trim().is_empty() && self.attachments.is_empty()) {
            return;
        }
        let attachments = self
            .attachments
            .drain(..)
            .map(|attachment| attachment.value)
            .collect::<Vec<_>>();
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

    fn switch_conversation(
        &mut self,
        id: ConversationId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.chat.switch_conversation(&id) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
            return;
        }
        self.page = Page::Chat;
        self.switcher_open = false;
        self.focus_composer(window, cx);
        cx.notify();
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
        self.page = Page::Chat;
        self.switcher_open = false;
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

    fn hide(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.controller.update(cx, |controller, cx| {
            controller.hide_main_shell_from_view(window, cx);
        });
    }

    pub fn show_chat(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Chat;
        self.switcher_open = false;
        self.model_open = false;
        cx.notify();
    }

    pub fn show_settings(&mut self, cx: &mut Context<Self>) {
        self.page = Page::Settings;
        self.switcher_open = false;
        self.model_open = false;
        cx.notify();
    }

    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.page = Page::About;
        self.switcher_open = false;
        self.model_open = false;
        cx.notify();
    }

    pub fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == Page::Chat && !self.switcher_open {
            self.composer
                .update(cx, |composer, cx| composer.focus(window, cx));
        }
    }

    fn filtered_conversations(&self, cx: &Context<Self>) -> Vec<ChatConversationSummary> {
        self.chat
            .search_conversations(&self.search.read(cx).value(), self.favorites_only)
    }

    fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Chat;
        self.switcher_open = true;
        self.model_open = false;
        self.favorites_only = false;
        self.search
            .update(cx, |search, cx| search.clean(window, cx));
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
        cx.notify();
    }

    fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher_open = false;
        self.model_open = false;
        self.page = Page::History;
        self.favorites_only = false;
        self.history_limit = HISTORY_PAGE_SIZE;
        self.search
            .update(cx, |search, cx| search.clean(window, cx));
        self.search
            .update(cx, |search, cx| search.focus(window, cx));
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_open {
            self.model_open = false;
            self.focus_composer(window, cx);
        } else if self.switcher_open {
            self.switcher_open = false;
            self.focus_composer(window, cx);
        } else if self.page != Page::Chat {
            self.page = Page::Chat;
            self.focus_composer(window, cx);
        } else {
            self.hide(window, cx);
        }
        cx.notify();
    }

    fn toggle_pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = !self.pinned;
        match self
            .controller
            .update(cx, |controller, cx| controller.set_pinned(next, window, cx))
        {
            Ok(Ok(())) => self.pinned = next,
            Ok(Err(error)) => self.transient_status = Some(error.to_string().into()),
            Err(error) => self.transient_status = Some(error.to_string().into()),
        }
        cx.notify();
    }

    fn toggle_favorite(&mut self, id: ConversationId, favorite: bool, cx: &mut Context<Self>) {
        if let Err(error) = self.chat.set_conversation_favorite(&id, favorite) {
            self.transient_status = Some(error.to_string().into());
            cx.notify();
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_chat = self.page == Page::Chat;
        let title: SharedString = match self.page {
            Page::Chat => self.snapshot.title.clone().into(),
            Page::History => "Conversations".into(),
            Page::Settings => "Settings".into(),
            Page::About => "About LexWisp".into(),
        };
        h_flex()
            .h(px(ui_metrics::HEADER_HEIGHT))
            .flex_none()
            .w_full()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().title_bar_border)
            .bg(cx.theme().title_bar)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .pl_3()
                    .child(
                        Button::new("history-or-back")
                            .ghost()
                            .small()
                            .icon(if is_chat {
                                Icon::new(AssetIconName::BookOpen)
                            } else {
                                Icon::new(AssetIconName::ArrowLeft)
                            })
                            .accessibility_label(if is_chat {
                                "Open conversations"
                            } else {
                                "Back to chat"
                            })
                            .tooltip(if is_chat {
                                "Open conversations · Ctrl+K"
                            } else {
                                "Back to chat"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.page == Page::Chat {
                                    this.open_switcher(window, cx);
                                } else {
                                    this.page = Page::Chat;
                                    this.focus_composer(window, cx);
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_medium()
                            .child(title),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_8()
                            .h_full()
                            .window_control_area(WindowControlArea::Drag),
                    ),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("new-chat")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .accessibility_label("New chat")
                            .tooltip("New chat · Ctrl+N")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.create_conversation(window, cx)
                            })),
                    )
                    .child(
                        Button::new("settings")
                            .ghost()
                            .small()
                            .icon(Icon::new(AssetIconName::Settings))
                            .accessibility_label("Settings")
                            .tooltip("Settings")
                            .selected(self.page == Page::Settings)
                            .on_click(cx.listener(|this, _, _, cx| this.show_settings(cx))),
                    )
                    .child(
                        Button::new("pin-main-window")
                            .ghost()
                            .small()
                            .icon(Icon::new(AssetIconName::Pin))
                            .selected(self.pinned)
                            .accessibility_label(if self.pinned {
                                "Unpin window"
                            } else {
                                "Pin window"
                            })
                            .tooltip(if self.pinned {
                                "Unpin window · Ctrl+P"
                            } else {
                                "Pin window · Ctrl+P"
                            })
                            .on_click(
                                cx.listener(|this, _, window, cx| this.toggle_pin(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("close-main-window")
                            .aria_label("Hide window")
                            .tooltip(|window, cx| {
                                Tooltip::new("Hide window · Esc").build(window, cx)
                            })
                            .w(px(ui_metrics::CAPTION_WIDTH))
                            .h_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .window_control_area(WindowControlArea::Close)
                            .hover(|style| {
                                style
                                    .bg(cx.theme().danger)
                                    .text_color(cx.theme().danger_foreground)
                            })
                            .active(|style| {
                                style
                                    .bg(cx.theme().danger_active)
                                    .text_color(cx.theme().danger_foreground)
                            })
                            .child(Icon::new(IconName::Close).small()),
                    ),
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
                                    .accessibility_label(format!("Remove {}", attachment.name()))
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

    fn render_model_options(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .p_2()
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Model profile"),
            )
            .children(
                [ChatModelPreference::Fast, ChatModelPreference::Smart]
                    .into_iter()
                    .map(|preference| {
                        let selected = self.snapshot.model_preference == preference;
                        Button::new(format!("model-{}", preference.display_name()))
                            .ghost()
                            .small()
                            .label(preference.display_name())
                            .selected(selected)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_model(preference.clone(), cx);
                                this.model_open = false;
                                this.focus_composer(window, cx);
                                cx.notify();
                            }))
                    }),
            )
    }

    fn render_composer(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_send = self.snapshot.can_send
            && (!self.composer.read(cx).value().trim().is_empty() || !self.attachments.is_empty());
        let focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        div()
            .id("chat-composer-drop-zone")
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(px(ui_metrics::RADIUS_COMPOSER))
            .border_1()
            .border_color(if focused {
                cx.theme().ring
            } else {
                cx.theme().input
            })
            .bg(cx.theme().group_box)
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.load_attachments(paths.paths().to_vec(), cx);
            }))
            .children(self.render_attachments(cx))
            .child(
                Textarea::new(&self.composer)
                    .h_16()
                    .appearance(false)
                    .bordered(false)
                    .aria_label("Message"),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                Button::new("attach-files")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Plus)
                                    .accessibility_label("Attach files")
                                    .tooltip("Attach files…")
                                    .disabled(
                                        self.attachment_task.is_some()
                                            || self.attachments.len() >= MAX_ATTACHMENTS,
                                    )
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.choose_attachments(cx)),
                                    ),
                            )
                            .child(
                                Button::new("choose-model")
                                    .ghost()
                                    .small()
                                    .label(self.snapshot.model_preference.display_name())
                                    .tooltip("Choose model profile")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.model_open = !this.model_open;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when(self.snapshot.can_retry && !self.snapshot.can_stop, |row| {
                                row.child(
                                    Button::new("retry-answer")
                                        .ghost()
                                        .small()
                                        .label("Retry")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.begin_regenerate(cx)),
                                        ),
                                )
                            })
                            .when(self.snapshot.can_stop, |row| {
                                row.child(
                                    Button::new("stop-response")
                                        .ghost()
                                        .small()
                                        .label("Stop")
                                        .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                                )
                            })
                            .child(
                                Button::new("send-message")
                                    .primary()
                                    .small()
                                    .icon(IconName::ArrowUp)
                                    .accessibility_label("Send message")
                                    .tooltip("Send · Enter")
                                    .disabled(!can_send)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        let input = this.composer.read(cx).value().to_string();
                                        this.begin_send(input, window, cx);
                                    })),
                            ),
                    ),
            )
            .when(self.model_open, |this| {
                this.child(
                    div()
                        .absolute()
                        .bottom_12()
                        .left_3()
                        .w_48()
                        .child(self.render_model_options(cx)),
                )
            })
    }

    fn render_transcript(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        if self.snapshot.messages.is_empty() {
            return v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(cx.theme().muted_foreground)
                        .child("LexWisp"),
                )
                .child(div().text_lg().font_medium().child("What can I help with?"))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Ask below to start a conversation."),
                )
                .into_any_element();
        }
        let messages = self.snapshot.messages.clone();
        MessageScroller::new(
            "main-window-transcript",
            self.messages.clone(),
            move |index, _, cx| {
                messages
                    .get(index)
                    .map(|message| render_message(message, cx))
                    .unwrap_or_else(|| div().into_any_element())
            },
        )
        .flex_1()
        .min_h_0()
        .w_full()
        .with_jump_button_label("Jump to latest")
        .into_any_element()
    }

    fn render_conversation_row(
        &self,
        item: &ChatConversationSummary,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = item.id().clone();
        let selected = self.snapshot.conversation_id == id;
        let favorite = item.favorite();
        let title = item.title().to_owned();
        h_flex()
            .w_full()
            .min_h(px(ui_metrics::HISTORY_ROW_HEIGHT))
            .gap_2()
            .child(
                Button::new(format!("conversation-{id}"))
                    .ghost()
                    .small()
                    .flex_1()
                    .min_w_0()
                    .selected(selected)
                    .accessibility_label(title.clone())
                    .tooltip(title.clone())
                    .child(div().w_full().truncate().text_left().child(title))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.switch_conversation(id.clone(), window, cx)
                    })),
            )
            .when(self.page == Page::History, |row| {
                let id = item.id().clone();
                row.child(
                    Button::new(format!("favorite-{id}"))
                        .ghost()
                        .xsmall()
                        .flex_none()
                        .icon(IconName::Star)
                        .selected(favorite)
                        .accessibility_label(if favorite {
                            "Remove favorite"
                        } else {
                            "Add favorite"
                        })
                        .tooltip(if favorite {
                            "Remove favorite"
                        } else {
                            "Add favorite"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_favorite(id.clone(), !favorite, cx)
                        })),
                )
            })
            .into_any_element()
    }

    fn render_switcher(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .absolute()
            .top(px(ui_metrics::HEADER_HEIGHT))
            .left_3()
            .w(px(ui_metrics::SWITCHER_WIDTH))
            .max_h(px(320.0))
            .rounded(cx.theme().radius_lg)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_lg()
            .bg(cx.theme().popover)
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(Input::new(&self.search).aria_label("Search conversations")),
            )
            .child(
                Button::new("switcher-new-chat")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .label("New chat")
                    .on_click(
                        cx.listener(|this, _, window, cx| this.create_conversation(window, cx)),
                    ),
            )
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Recent"),
                    )
                    .children(
                        self.filtered_conversations(cx)
                            .iter()
                            .take(5)
                            .map(|item| self.render_conversation_row(item, cx)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .p_2()
                    .child(
                        Button::new("view-all-conversations")
                            .ghost()
                            .small()
                            .label("View all conversations")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.open_history(window, cx)),
                            ),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("rename-conversation")
                            .ghost()
                            .small()
                            .label("Rename…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_rename_dialog(window, cx)
                            })),
                    )
                    .child(
                        Button::new("delete-conversation")
                            .ghost()
                            .small()
                            .label("Delete…")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.confirm_delete(window, cx)),
                            ),
                    ),
            )
    }

    fn render_about(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .flex_1()
            .w_full()
            .gap_3()
            .p_5()
            .bg(cx.theme().background)
            .child(div().text_lg().font_semibold().child("LexWisp"))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
            )
            .child(
                div()
                    .text_sm()
                    .child("A floating AI chat client for Windows."),
            )
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.filtered_conversations(cx);
        let has_more = items.len() > self.history_limit;
        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .child(Input::new(&self.search).aria_label("Search history"))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("recent-tab")
                            .ghost()
                            .small()
                            .label("Recent")
                            .selected(!self.favorites_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.favorites_only = false;
                                this.history_limit = HISTORY_PAGE_SIZE;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("favorites-tab")
                            .ghost()
                            .small()
                            .label("Favorites")
                            .selected(self.favorites_only)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.favorites_only = true;
                                this.history_limit = HISTORY_PAGE_SIZE;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_1()
                    .when(items.is_empty(), |list| {
                        list.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No matching conversations"),
                        )
                    })
                    .children(
                        items
                            .iter()
                            .take(self.history_limit)
                            .map(|item| self.render_conversation_row(item, cx)),
                    )
                    .when(has_more, |list| {
                        list.child(
                            Button::new("history-load-more")
                                .ghost()
                                .small()
                                .label("Load more conversations")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.history_limit =
                                        this.history_limit.saturating_add(HISTORY_PAGE_SIZE);
                                    cx.notify();
                                })),
                        )
                    }),
            )
    }
}

impl Render for ChatExperience {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.transient_status.clone().or_else(|| {
            if self.snapshot.has_unsaved_result {
                Some("Unsaved result".into())
            } else if matches!(
                self.snapshot.status_text.as_str(),
                "Ready" | "Restored" | "Answer complete"
            ) {
                None
            } else {
                Some(self.snapshot.status_text.clone().into())
            }
        });
        v_flex()
            .relative()
            .size_full()
            .min_h_0()
            .key_context(KEY_CONTEXT)
            .on_action(
                cx.listener(|this, _: &OpenSwitcher, window, cx| this.open_switcher(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenHistory, window, cx| this.open_history(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NewChat, window, cx| this.create_conversation(window, cx)),
            )
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &TogglePin, window, cx| this.toggle_pin(window, cx)))
            .bg(cx.theme().transparent)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(match self.page {
                Page::History => self.render_history(cx).into_any_element(),
                Page::Settings => self.settings_view.clone().into_any_element(),
                Page::About => self.render_about(cx).into_any_element(),
                Page::Chat => v_flex()
                    .flex_1()
                    .min_h_0()
                    .bg(cx.theme().background)
                    .child(self.render_transcript(cx))
                    .when_some(status, |this, status| {
                        this.child(
                            div()
                                .px_5()
                                .pb_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    })
                    .child(div().px_4().pb_4().child(self.render_composer(window, cx)))
                    .into_any_element(),
            })
            .when(self.switcher_open, |this| {
                this.child(self.render_switcher(cx))
            })
    }
}

fn render_message(message: &ChatMessageSnapshot, cx: &mut App) -> gpui_kit::AnyElement {
    let id = message.id.as_str();
    let mut content = message.content.clone();
    if !message.attachments.is_empty() {
        if !content.is_empty() {
            content.push_str("\n\n");
        }
        content.push_str("Attachments: ");
        content.push_str(
            &message
                .attachments
                .iter()
                .map(|attachment| format!("\x60{}\x60", attachment.name()))
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    let mut code_surface = div()
        .bg(cx.theme().group_box)
        .border_1()
        .border_color(cx.theme().border)
        .rounded(px(ui_metrics::RADIUS_CONTROL));
    let code_message_id = id.to_owned();
    let body = TextView::markdown(format!("message-body-{id}"), content.clone())
        .selectable(true)
        .line_height(rems(1.45))
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.75))
                .heading_font_size(|level, _| {
                    px(match level {
                        1 => ui_metrics::FONT_PAGE_TITLE,
                        2 => ui_metrics::FONT_SECTION + 1.0,
                        _ => ui_metrics::FONT_BODY,
                    })
                })
                .code_block(code_surface.style().clone()),
        )
        .code_block_actions(move |code, _, _| {
            Clipboard::new(format!("copy-code-{code_message_id}-{:?}", code.span))
                .value(code.code())
                .tooltip("Copy code block")
        });
    let status = match message.status {
        ChatMessageStatus::Generating => Some("Generating"),
        ChatMessageStatus::CancelledPartial => Some("Stopped · partial"),
        ChatMessageStatus::FailedPartial => Some("Failed · partial kept"),
        ChatMessageStatus::Submitted | ChatMessageStatus::Completed => None,
    };
    if message.is_user {
        h_flex()
            .w_full()
            .justify_end()
            .px_5()
            .py_3()
            .child(
                v_flex()
                    .max_w(px(520.0))
                    .ml_12()
                    .gap_1()
                    .p_3()
                    .rounded(px(ui_metrics::RADIUS_PANEL))
                    .bg(cx.theme().accent)
                    .child(body),
            )
            .into_any_element()
    } else {
        v_flex()
            .w_full()
            .gap_2()
            .px_5()
            .py_3()
            .child(
                div()
                    .text_xs()
                    .font_medium()
                    .text_color(cx.theme().muted_foreground)
                    .child("LexWisp"),
            )
            .child(body)
            .child(
                h_flex()
                    .gap_2()
                    .when_some(status, |row, status| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    })
                    .child(
                        Clipboard::new(format!("copy-message-{id}"))
                            .value(content)
                            .tooltip("Copy answer"),
                    ),
            )
            .into_any_element()
    }
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

#[cfg(test)]
mod tests {
    use super::{image_media_type, is_text_extension};

    #[test]
    fn attachment_types_are_explicitly_bounded() {
        assert_eq!(image_media_type("png"), Some("image/png"));
        assert!(is_text_extension("rs"));
        assert!(!is_text_extension("pdf"));
    }
}

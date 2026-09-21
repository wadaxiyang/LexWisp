pub mod theme;
pub mod ui_metrics;

use std::{path::PathBuf, sync::Arc, time::Duration};

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Root, Selectable, Sizable, StyledExt,
    bubble::{Bubble, BubbleVariant},
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    message::{Message, MessageAlignment, MessageContent, MessageHeader},
    message_scroller::{MessageScroller, MessageScrollerState},
    scroll::ScrollableElement,
    text::TextView,
    v_flex,
};
use gpui_kit::{
    App, AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, KeyBinding,
    ParentElement, PathPromptOptions, Render, SharedString, Styled, Subscription, Task, Window,
    WindowBackgroundAppearance, WindowBounds, WindowControlArea, WindowDecorations, WindowKind,
    WindowOptions, actions, div, prelude::FluentBuilder, px, rems,
};

use gpui_kit::assets::IconName as AssetIconName;
use theme::ThemePreference;

actions!(
    lexwisp_main_window,
    [OpenSwitcher, NewChat, Dismiss, TogglePin, OpenHistory]
);

const KEY_CONTEXT: &str = "LexWispMainWindow";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Page {
    Chat,
    History,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebMode {
    Off,
    Auto,
    On,
}

impl WebMode {
    fn next(self) -> Self {
        match self {
            Self::Off => Self::Auto,
            Self::Auto => Self::On,
            Self::On => Self::Off,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Off => "Web off",
            Self::Auto => "Web auto",
            Self::On => "Web on",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DemoModel {
    Quick,
    Thoughtful,
}

impl DemoModel {
    fn label(self) -> &'static str {
        match self {
            Self::Quick => "Quick preview",
            Self::Thoughtful => "Thoughtful preview",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Speaker {
    User,
    Assistant,
}

#[derive(Clone)]
struct ChatMessage {
    id: u64,
    speaker: Speaker,
    body: SharedString,
}

impl ChatMessage {
    fn new(id: u64, speaker: Speaker, body: impl Into<SharedString>) -> Self {
        Self {
            id,
            speaker,
            body: body.into(),
        }
    }
}

struct Conversation {
    id: u64,
    title: SharedString,
    messages: Arc<Vec<ChatMessage>>,
    favorite: bool,
}

impl Conversation {
    fn new(id: u64, title: impl Into<SharedString>, messages: Vec<ChatMessage>) -> Self {
        Self {
            id,
            title: title.into(),
            messages: Arc::new(messages),
            favorite: false,
        }
    }

    fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty()
            || self.title.to_lowercase().contains(&query)
            || self
                .messages
                .iter()
                .any(|message| message.body.to_lowercase().contains(&query))
    }

    fn preview(&self) -> SharedString {
        self.messages
            .last()
            .map(|message| message.body.clone())
            .unwrap_or_else(|| "New conversation".into())
    }
}

struct LexWispMainWindow {
    composer: Entity<TextareaState>,
    search: Entity<InputState>,
    scroller: Entity<MessageScrollerState>,
    conversations: Vec<Conversation>,
    active_id: Option<u64>,
    next_id: u64,
    next_message_id: u64,
    page: Page,
    switcher_open: bool,
    model_open: bool,
    pinned: bool,
    favorites_only: bool,
    web_mode: WebMode,
    model: DemoModel,
    attachments: Vec<PathBuf>,
    status: Option<SharedString>,
    reply_generation: u64,
    replying: bool,
    reply_task: Option<Task<()>>,
    picker_task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl LexWispMainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>, preference: ThemePreference) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask LexWisp…")
                .submit_on_enter(true)
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search conversations…"));
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        composer.update(cx, |state, cx| state.focus(window, cx));

        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                _ => {}
            },
        );
        let search_subscription =
            cx.subscribe_in(&search, window, |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => {
                    if let Some(id) = this.filtered_conversations(cx).first().map(|item| item.id) {
                        this.select_conversation(id, window, cx);
                    }
                }
                _ => {}
            });
        let appearance_subscription = window.observe_window_appearance(move |window, cx| {
            if preference == ThemePreference::System {
                theme::sync_system_theme(window, cx);
            }
        });

        let conversations = vec![
            Conversation::new(
                1,
                "GPUI state ownership",
                vec![
                    ChatMessage::new(
                        1,
                        Speaker::User,
                        "How should a chat window keep its draft while switching conversations?",
                    ),
                    ChatMessage::new(
                        2,
                        Speaker::Assistant,
                        "Keep one retained Composer entity in the main window. Switching conversations changes the active transcript without recreating the input.",
                    ),
                ],
            ),
            Conversation::new(
                2,
                "A quieter chat surface",
                vec![
                    ChatMessage::new(
                        3,
                        Speaker::User,
                        "Make the interface feel calm and focused.",
                    ),
                    ChatMessage::new(
                        4,
                        Speaker::Assistant,
                        "Use one clear action, a compact header, subtle structural borders and an uninterrupted transcript. Keep navigation in the switcher instead of a permanent sidebar.",
                    ),
                ],
            ),
        ];

        Self {
            composer,
            search,
            scroller,
            conversations,
            active_id: None,
            next_id: 3,
            next_message_id: 5,
            page: Page::Chat,
            switcher_open: false,
            model_open: false,
            pinned: false,
            favorites_only: false,
            web_mode: WebMode::Off,
            model: DemoModel::Quick,
            attachments: Vec::new(),
            status: None,
            reply_generation: 0,
            replying: false,
            reply_task: None,
            picker_task: None,
            _subscriptions: vec![
                composer_subscription,
                search_subscription,
                appearance_subscription,
            ],
        }
    }

    fn active_conversation(&self) -> Option<&Conversation> {
        self.conversations
            .iter()
            .find(|item| Some(item.id) == self.active_id)
    }

    fn active_conversation_mut(&mut self) -> Option<&mut Conversation> {
        self.conversations
            .iter_mut()
            .find(|item| Some(item.id) == self.active_id)
    }

    fn title(&self) -> SharedString {
        self.active_conversation()
            .map(|item| item.title.clone())
            .unwrap_or_else(|| "New chat".into())
    }

    fn filtered_conversations(&self, cx: &Context<Self>) -> Vec<&Conversation> {
        let query = self.search.read(cx).value();
        self.conversations
            .iter()
            .rev()
            .filter(|item| (!self.favorites_only || item.favorite) && item.matches(&query))
            .collect()
    }

    fn focus_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }

    fn open_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Chat;
        self.switcher_open = true;
        self.model_open = false;
        self.favorites_only = false;
        self.search.update(cx, |state, cx| state.clean(window, cx));
        self.search.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn open_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switcher_open = false;
        self.page = Page::History;
        self.favorites_only = false;
        self.search.update(cx, |state, cx| state.clean(window, cx));
        self.search.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_open {
            self.model_open = false;
            self.focus_composer(window, cx);
        } else if self.switcher_open {
            self.switcher_open = false;
            self.focus_composer(window, cx);
        } else if self.page == Page::History {
            self.page = Page::Chat;
            self.focus_composer(window, cx);
        } else {
            // A lab binary has no OS-global summon shortcut. Minimize keeps the
            // single live window recoverable from the Windows taskbar.
            window.minimize_window();
        }
        cx.notify();
    }

    fn start_new_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_generation += 1;
        self.reply_task = None;
        self.replying = false;
        self.active_id = None;
        self.page = Page::Chat;
        self.switcher_open = false;
        self.attachments.clear();
        self.status = None;
        self.composer
            .update(cx, |state, cx| state.clean(window, cx));
        self.scroller.update(cx, |state, cx| state.reset(0, cx));
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn select_conversation(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(count) = self
            .conversations
            .iter()
            .find(|item| item.id == id)
            .map(|item| item.messages.len())
        {
            self.reply_generation += 1;
            self.reply_task = None;
            self.replying = false;
            self.active_id = Some(id);
            self.page = Page::Chat;
            self.switcher_open = false;
            self.scroller.update(cx, |state, cx| state.reset(count, cx));
            self.focus_composer(window, cx);
            cx.notify();
        }
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.replying {
            return;
        }
        let draft = self.composer.read(cx).value();
        let draft = draft.trim();
        if draft.is_empty() && self.attachments.is_empty() {
            return;
        }
        let draft = if draft.is_empty() {
            "Review the attached context"
        } else {
            draft
        };
        if self.active_id.is_none() {
            let id = self.next_id;
            self.next_id += 1;
            self.conversations
                .push(Conversation::new(id, conversation_title(draft), vec![]));
            self.active_id = Some(id);
        }
        let user_message_id = self.next_message_id;
        let reply_id = user_message_id + 1;
        self.next_message_id += 2;
        let mut submitted = draft.to_owned();
        if !self.attachments.is_empty() {
            let names = self
                .attachments
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ");
            submitted.push_str(&format!("\n\nAttached locally: {names}"));
        }
        if let Some(conversation) = self.active_conversation_mut() {
            let messages = Arc::make_mut(&mut conversation.messages);
            messages.push(ChatMessage::new(user_message_id, Speaker::User, submitted));
            messages.push(ChatMessage::new(reply_id, Speaker::Assistant, ""));
        }
        let count = self
            .active_conversation()
            .map_or(0, |item| item.messages.len());
        self.scroller.update(cx, |state, cx| state.reset(count, cx));
        self.composer
            .update(cx, |state, cx| state.clean(window, cx));
        self.attachments.clear();
        self.status = None;
        self.page = Page::Chat;
        self.switcher_open = false;
        self.replying = true;
        self.reply_generation += 1;
        let generation = self.reply_generation;
        let conversation_id = self.active_id;
        let response = if self.model == DemoModel::Quick {
            "This is a local UI preview. LexWispMainWindow keeps one conversation and one Composer, so you can try a follow-up, switch chats, search history, pin the window, or attach context. No provider request was sent."
        } else {
            "This is a local UI preview, not a provider response. The retained chat state drives one continuous Conversation layout. Navigation stays inside the main window, and the transcript remains available for follow-up questions."
        };
        let chunks = response
            .split_whitespace()
            .collect::<Vec<_>>()
            .chunks(5)
            .map(|chunk| chunk.join(" "))
            .collect::<Vec<_>>();
        let chunk_count = chunks.len();
        self.reply_task = Some(cx.spawn_in(window, async move |this, cx| {
            for (index, chunk) in chunks.into_iter().enumerate() {
                cx.background_executor()
                    .timer(Duration::from_millis(45))
                    .await;
                let completed = index == chunk_count.saturating_sub(1);
                if this
                    .update_in(cx, |state, _, cx| {
                        if state.reply_generation != generation
                            || state.active_id != conversation_id
                        {
                            return;
                        }
                        if let Some(conversation) = state.active_conversation_mut() {
                            let messages = Arc::make_mut(&mut conversation.messages);
                            if let Some(message) =
                                messages.iter_mut().find(|message| message.id == reply_id)
                            {
                                let mut body = message.body.to_string();
                                if !body.is_empty() {
                                    body.push(' ');
                                }
                                body.push_str(&chunk);
                                message.body = body.into();
                            }
                        }
                        state.scroller.update(cx, |scroller, cx| {
                            scroller.remeasure_items(count.saturating_sub(1)..count, cx);
                        });
                        if completed {
                            state.replying = false;
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    return;
                }
            }
        }));
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn stop_reply(&mut self, cx: &mut Context<Self>) {
        self.reply_generation += 1;
        self.reply_task = None;
        self.replying = false;
        self.status = Some("Preview stopped. Partial text remains in the conversation.".into());
        cx.notify();
    }

    fn attach(&mut self, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        self.picker_task = Some(cx.spawn(async move |this, cx| match receiver.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update(cx, |state, cx| {
                    state.attachments.extend(paths);
                    state.status =
                        Some("Files are attached in this UI preview; no upload occurs.".into());
                    cx.notify();
                });
            }
            Ok(Ok(None)) => {}
            _ => {
                let _ = this.update(cx, |state, cx| {
                    state.status = Some("Could not open the file picker. Try again.".into());
                    cx.notify();
                });
            }
        }));
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.title();
        let is_history = self.page == Page::History;
        h_flex()
            .h(px(ui_metrics::HEADER_HEIGHT))
            .flex_none()
            .w_full()
            .justify_between()
            .gap_2()
            .px_3()
            .border_b_1()
            .border_color(cx.theme().sidebar_border)
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        Button::new("history-or-back")
                            .ghost()
                            .small()
                            .icon(if is_history {
                                Icon::new(AssetIconName::ArrowLeft)
                            } else {
                                Icon::new(AssetIconName::BookOpen)
                            })
                            .accessibility_label(if is_history {
                                "Back to chat"
                            } else {
                                "Open conversations"
                            })
                            .tooltip(if is_history {
                                "Back to chat"
                            } else {
                                "Open conversations · Ctrl+K"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.page == Page::History {
                                    this.page = Page::Chat;
                                    this.focus_composer(window, cx);
                                    cx.notify();
                                } else {
                                    this.open_switcher(window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new("conversation-title")
                            .ghost()
                            .small()
                            .label(if is_history {
                                "Conversations".into()
                            } else {
                                title
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.page == Page::History {
                                    this.page = Page::Chat;
                                    this.focus_composer(window, cx);
                                    cx.notify();
                                } else {
                                    this.open_switcher(window, cx);
                                }
                            })),
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
                            .on_click(
                                cx.listener(|this, _, window, cx| this.start_new_chat(window, cx)),
                            ),
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pinned = !this.pinned;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("hide-main-window")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .accessibility_label("Hide window")
                            .tooltip("Hide window · Esc")
                            .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
                    ),
            )
    }

    fn render_attachments(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .gap_2()
            .flex_wrap()
            .children(self.attachments.iter().enumerate().map(|(index, path)| {
                let label = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                h_flex()
                    .gap_1()
                    .px_2()
                    .py_1()
                    .rounded(cx.theme().radius)
                    .bg(cx.theme().muted)
                    .text_xs()
                    .child(div().max_w_40().truncate().child(label))
                    .child(
                        Button::new(format!("remove-attachment-{index}"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .accessibility_label("Remove attachment")
                            .tooltip("Remove attachment")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if index < this.attachments.len() {
                                    this.attachments.remove(index);
                                    cx.notify();
                                }
                            })),
                    )
            }))
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
                    .child("UI preview models"),
            )
            .children(
                [DemoModel::Quick, DemoModel::Thoughtful]
                    .into_iter()
                    .map(|model| {
                        Button::new(format!("model-{:?}", model))
                            .ghost()
                            .small()
                            .label(model.label())
                            .selected(self.model == model)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.model = model;
                                this.model_open = false;
                                this.focus_composer(window, cx);
                                cx.notify();
                            }))
                    }),
            )
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_send =
            !self.composer.read(cx).value().trim().is_empty() || !self.attachments.is_empty();
        v_flex()
            .relative()
            .w_full()
            .gap_2()
            .p_3()
            .rounded(px(ui_metrics::RADIUS_COMPOSER))
            .border_1()
            .border_color(cx.theme().input)
            .bg(cx.theme().group_box)
            .when(!self.attachments.is_empty(), |this| {
                this.child(self.render_attachments(cx))
            })
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
                                    .on_click(cx.listener(|this, _, _, cx| this.attach(cx))),
                            )
                            .child(
                                Button::new("web-mode")
                                    .ghost()
                                    .small()
                                    .label(self.web_mode.label())
                                    .tooltip("Cycle web mode · UI preview, no search request")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.web_mode = this.web_mode.next();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("choose-model")
                                    .ghost()
                                    .small()
                                    .label(self.model.label())
                                    .tooltip("Choose preview model")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.model_open = !this.model_open;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when(self.replying, |this| {
                                this.child(
                                    Button::new("stop-preview")
                                        .ghost()
                                        .small()
                                        .label("Stop")
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.stop_reply(cx)),
                                        ),
                                )
                            })
                            .child(
                                Button::new("send-message")
                                    .primary()
                                    .small()
                                    .icon(IconName::ArrowUp)
                                    .accessibility_label("Send message")
                                    .tooltip("Send · Enter")
                                    .disabled(!can_send || self.replying)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    ),
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
                        .shadow_lg()
                        .child(self.render_model_options(cx)),
                )
            })
    }

    fn render_message(message: &ChatMessage) -> impl IntoElement {
        let (alignment, label, variant) = match message.speaker {
            Speaker::User => (MessageAlignment::End, "You", BubbleVariant::Tinted),
            Speaker::Assistant => (MessageAlignment::Start, "LexWisp", BubbleVariant::Ghost),
        };
        Message::new()
            .alignment(alignment)
            .header(MessageHeader::new().child(label))
            .content(
                MessageContent::new().bubble(
                    Bubble::new()
                        .alignment(alignment)
                        .with_variant(variant)
                        .child(TextView::markdown(
                            format!("message-{}", message.id),
                            message.body.clone(),
                        )),
                ),
            )
    }

    fn render_transcript(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let messages = self
            .active_conversation()
            .map(|item| item.messages.clone())
            .unwrap_or_default();
        if messages.is_empty() {
            return v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_medium()
                        .child("What would you like to explore?"),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("Ask below to start a local UI preview."),
                )
                .into_any_element();
        }
        MessageScroller::new(
            "main-window-transcript",
            self.scroller.clone(),
            move |index, _, _| {
                messages
                    .get(index)
                    .map(Self::render_message)
                    .map(IntoElement::into_any_element)
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
        item: &Conversation,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let id = item.id;
        let selected = self.active_id == Some(id);
        let preview = item.preview();
        let button = Button::new(format!("conversation-{id}"))
            .ghost()
            .small()
            .flex_1()
            .min_w_0()
            .selected(selected)
            .accessibility_label(item.title.clone())
            .tooltip(preview.clone())
            .on_click(
                cx.listener(move |this, _, window, cx| this.select_conversation(id, window, cx)),
            );
        let button = if self.page == Page::History {
            button.h(rems(4.0)).child(
                v_flex()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap_1()
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_left()
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .w_full()
                            .truncate()
                            .text_left()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(preview),
                    ),
            )
        } else {
            button.h(rems(2.5)).child(
                div()
                    .w_full()
                    .truncate()
                    .text_left()
                    .child(item.title.clone()),
            )
        };
        h_flex()
            .w_full()
            .gap_2()
            .child(button)
            .when(self.page == Page::History, |this| {
                this.child(
                    Button::new(format!("favorite-{id}"))
                        .ghost()
                        .xsmall()
                        .flex_none()
                        .icon(IconName::Star)
                        .selected(item.favorite)
                        .accessibility_label(if item.favorite {
                            "Remove favorite"
                        } else {
                            "Add favorite"
                        })
                        .tooltip(if item.favorite {
                            "Remove favorite"
                        } else {
                            "Add favorite"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(item) =
                                this.conversations.iter_mut().find(|item| item.id == id)
                            {
                                item.favorite = !item.favorite;
                                cx.notify();
                            }
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
            .right_3()
            .h_72()
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
                    .on_click(cx.listener(|this, _, window, cx| this.start_new_chat(window, cx))),
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
                            .into_iter()
                            .take(5)
                            .map(|item| self.render_conversation_row(item, cx)),
                    ),
            )
            .child(
                div()
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
                    ),
            )
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .when(self.filtered_conversations(cx).is_empty(), |this| {
                        this.child(
                            div()
                                .p_4()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child("No matching conversations"),
                        )
                    })
                    .children(
                        self.filtered_conversations(cx)
                            .into_iter()
                            .map(|item| self.render_conversation_row(item, cx)),
                    ),
            )
    }
}

impl Render for LexWispMainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .on_action(cx.listener(|this, _: &NewChat, window, cx| this.start_new_chat(window, cx)))
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .on_action(cx.listener(|this, _: &TogglePin, _, cx| {
                this.pinned = !this.pinned;
                cx.notify();
            }))
            .bg(cx.theme().transparent)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(if self.page == Page::History {
                self.render_history(cx).into_any_element()
            } else {
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .bg(cx.theme().background)
                    .child(self.render_transcript(cx))
                    .when_some(self.status.clone(), |this, status| {
                        this.child(
                            div()
                                .px_5()
                                .pb_2()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(status),
                        )
                    })
                    .child(div().px_4().pb_4().child(self.render_composer(cx)))
                    .into_any_element()
            })
            .when(self.switcher_open, |this| {
                this.child(self.render_switcher(cx))
            })
    }
}

fn conversation_title(text: &str) -> SharedString {
    let mut chars = text.chars();
    let prefix = chars.by_ref().take(36).collect::<String>();
    if chars.next().is_some() {
        format!("{prefix}…").into()
    } else {
        prefix.into()
    }
}

fn register_shortcuts(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenSwitcher, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-n", NewChat, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-h", OpenHistory, Some(KEY_CONTEXT)),
        KeyBinding::new("cmd-p", TogglePin, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

fn main() {
    let preference = if std::env::args().any(|argument| argument == "--dark") {
        ThemePreference::Dark
    } else if std::env::args().any(|argument| argument == "--light") {
        ThemePreference::Light
    } else {
        ThemePreference::System
    };
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_shortcuts(cx);
            let bounds = Bounds::centered(None, ui_metrics::main_window_size(), cx);
            cx.spawn(async move |cx| {
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    kind: WindowKind::Normal,
                    window_decorations: Some(WindowDecorations::Client),
                    window_background: WindowBackgroundAppearance::MicaBackdrop,
                    is_resizable: false,
                    is_minimizable: true,
                    window_min_size: Some(ui_metrics::main_window_size()),
                    ..WindowOptions::default()
                };
                cx.open_window(options, |window, cx| {
                    window.set_window_title("LexWispMainWindow");
                    theme::apply_theme(preference, window, cx);
                    let view = cx.new(|cx| LexWispMainWindow::new(window, cx, preference));
                    cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().transparent))
                })
                .expect("failed to open LexWisp main window UI lab");
            })
            .detach();
        });
}

#[cfg(test)]
mod tests {
    use super::{Conversation, WebMode, conversation_title};
    use gpui_kit::{AppContext, TestAppContext, component::Root, px, test::TestWindowExt};

    #[test]
    fn conversation_search_includes_title_and_message() {
        let item = Conversation::new(
            1,
            "Main window UI",
            vec![super::ChatMessage::new(
                1,
                super::Speaker::User,
                "Retained Composer",
            )],
        );
        assert!(item.matches("window"));
        assert!(item.matches("composer"));
        assert!(!item.matches("provider"));
    }

    #[test]
    fn web_mode_cycles_without_a_network_request() {
        assert_eq!(WebMode::Off.next(), WebMode::Auto);
        assert_eq!(WebMode::Auto.next(), WebMode::On);
        assert_eq!(WebMode::On.next(), WebMode::Off);
    }

    #[test]
    fn long_titles_truncate_on_character_boundaries() {
        let title = conversation_title(&"界面".repeat(30));
        assert_eq!(title.chars().count(), 37);
        assert!(title.ends_with('…'));
    }

    #[gpui_kit::test]
    fn main_window_starts_in_conversation_and_switcher_lists_chats(cx: &mut TestAppContext) {
        cx.update(gpui_kit::init);
        cx.update(super::register_shortcuts);
        let mut experience = None;
        let handle = cx.open_window(super::ui_metrics::main_window_size(), |window, cx| {
            let view = cx.new(|cx| {
                super::LexWispMainWindow::new(window, cx, super::ThemePreference::System)
            });
            experience = Some(view.clone());
            Root::new(view, window, cx)
        });
        let experience = experience.expect("test view should exist");

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("history-or-back").is_some());
            assert!(window.try_find("send-message").is_some());
            assert_eq!(window.bounds().size, super::ui_metrics::main_window_size());
            let composer = experience.read(cx).composer.clone();
            composer.update(cx, |state, cx| {
                state.set_value("Test main window", window, cx)
            });
            window.render_frame(cx);
            window.click("send-message", cx);
        })
        .expect("test window should exist");
        cx.update(|cx| {
            let view = experience.read(cx);
            assert_eq!(
                view.active_conversation().map(|item| item.messages.len()),
                Some(2)
            );
        });

        cx.update_window(handle.into(), |_, window, cx| {
            window.render_frame(cx);
            window.press("cmd-k", cx);
            let view = experience.read(cx);
            assert!(view.switcher_open);
            assert_eq!(view.conversations.len(), 3);
            assert!(view.search.read(cx).value().is_empty());
            let row_bounds = window.find("conversation-1").bounds();
            assert!(row_bounds.top() > px(super::ui_metrics::HEADER_HEIGHT));
            assert!(row_bounds.bottom() < window.bounds().bottom());
            window.press("escape", cx);
        })
        .expect("test window should exist");
        cx.update(|cx| assert!(!experience.read(cx).switcher_open));

        cx.update_window(handle.into(), |_, window, cx| {
            window.press("cmd-n", cx);
        })
        .expect("test window should exist");
        cx.update(|cx| {
            let view = experience.read(cx);
            assert_eq!(view.active_id, None);
        });
    }
}

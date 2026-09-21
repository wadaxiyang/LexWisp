pub mod theme;
pub mod ui_metrics;

use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Root, Sizable, StyledExt, TitleBar,
    bubble::{Bubble, BubbleVariant},
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    message::{Message, MessageAlignment, MessageContent, MessageHeader},
    scroll::ScrollableElement,
    sidebar::{Sidebar, SidebarCollapsible, SidebarGroup, SidebarMenu, SidebarMenuItem},
    v_flex,
};
use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, div, prelude::FluentBuilder, px,
};

use theme::ThemePreference;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Speaker {
    User,
    Assistant,
}

#[derive(Clone, Debug)]
struct ChatMessage {
    speaker: Speaker,
    body: SharedString,
}

impl ChatMessage {
    fn user(body: impl Into<SharedString>) -> Self {
        Self {
            speaker: Speaker::User,
            body: body.into(),
        }
    }

    fn assistant(body: impl Into<SharedString>) -> Self {
        Self {
            speaker: Speaker::Assistant,
            body: body.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct Conversation {
    id: SharedString,
    title: SharedString,
    messages: Vec<ChatMessage>,
}

impl Conversation {
    fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        messages: Vec<ChatMessage>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            messages,
        }
    }
}

pub struct ChartMainWindow {
    composer: Entity<TextareaState>,
    conversations: Vec<Conversation>,
    selected_conversation_id: Option<SharedString>,
    next_conversation_id: usize,
    sidebar_collapsed: bool,
    context_panel_open: bool,
    _subscriptions: Vec<Subscription>,
}

impl ChartMainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Describe what you want to build…")
                .submit_on_enter(true)
        });
        let composer_subscription = cx.subscribe_in(
            &composer,
            window,
            |this, _, event, window, cx| match event {
                InputEvent::Change => cx.notify(),
                InputEvent::PressEnter { shift: false, .. } => {
                    this.send_message(window, cx);
                }
                _ => {}
            },
        );
        let appearance_subscription = window.observe_window_appearance(|window, cx| {
            theme::sync_system_theme(window, cx);
        });

        let conversations = vec![
            Conversation::new(
                "conversation-ui-shell",
                "Refine the workspace shell",
                vec![
                    ChatMessage::user(
                        "Make the Chart main window feel like a focused native AI workbench.",
                    ),
                    ChatMessage::assistant(
                        "The shell now uses one continuous title-and-navigation surface, while the raised conversation workspace owns a compact local header. Window controls still use native Windows hit regions, so minimize, maximize, close, and Snap keep their platform behavior.",
                    ),
                ],
            ),
            Conversation::new(
                "conversation-theme",
                "Audit the LexWisp theme",
                vec![
                    ChatMessage::user("Check the visual hierarchy in light and dark mode."),
                    ChatMessage::assistant(
                        "The workspace uses semantic canvas, sidebar, group-box, border, and primary roles. Persistent regions remain flat; only the Composer receives a contained working surface.",
                    ),
                ],
            ),
            Conversation::new(
                "conversation-titlebar",
                "Replace native title chrome",
                vec![
                    ChatMessage::user("Should the title bar look like the default Windows frame?"),
                    ChatMessage::assistant(
                        "No. The app owns the visible chrome, while GPUI maps the caption buttons to Windows control areas. That gives LexWisp a continuous workbench surface without giving up familiar desktop behavior.",
                    ),
                ],
            ),
        ];

        Self {
            composer,
            selected_conversation_id: Some(conversations[0].id.clone()),
            conversations,
            next_conversation_id: 1,
            sidebar_collapsed: false,
            context_panel_open: false,
            _subscriptions: vec![composer_subscription, appearance_subscription],
        }
    }

    fn selected_conversation(&self) -> Option<&Conversation> {
        let selected_id = self.selected_conversation_id.as_ref()?;
        self.conversations
            .iter()
            .find(|conversation| &conversation.id == selected_id)
    }

    fn selected_conversation_mut(&mut self) -> Option<&mut Conversation> {
        let selected_id = self.selected_conversation_id.as_ref()?;
        self.conversations
            .iter_mut()
            .find(|conversation| &conversation.id == selected_id)
    }

    fn selected_title(&self) -> SharedString {
        self.selected_conversation()
            .map(|conversation| conversation.title.clone())
            .unwrap_or_else(|| "New conversation".into())
    }

    fn start_new_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_conversation_id = None;
        self.composer
            .update(cx, |composer, cx| composer.clean(window, cx));
        cx.notify();
    }

    fn send_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.composer.read(cx).value();
        let draft = draft.trim();
        if draft.is_empty() {
            return;
        }

        if self.selected_conversation_id.is_none() {
            let id: SharedString =
                format!("conversation-local-{}", self.next_conversation_id).into();
            self.next_conversation_id += 1;
            self.conversations.push(Conversation::new(
                id.clone(),
                conversation_title(draft),
                Vec::new(),
            ));
            self.selected_conversation_id = Some(id);
        }

        if let Some(conversation) = self.selected_conversation_mut() {
            conversation
                .messages
                .push(ChatMessage::user(draft.to_owned()));
            conversation.messages.push(ChatMessage::assistant(
                "This Chart main window uses deterministic local state only. Your message was added to the active conversation so the shell and Composer flow can be reviewed without a backend.",
            ));
        }

        self.composer
            .update(cx, |composer, cx| composer.clean(window, cx));
        cx.notify();
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar_icon = if self.sidebar_collapsed {
            IconName::PanelLeftOpen
        } else {
            IconName::PanelLeftClose
        };

        TitleBar::new()
            .h(px(ui_metrics::HEADER_HEIGHT))
            .pl_0()
            .border_b(px(0.0))
            .bg(cx.theme().sidebar)
            .child(
                h_flex()
                    .size_full()
                    .min_w_0()
                    .gap_2()
                    .px_2()
                    .text_color(cx.theme().sidebar_foreground)
                    .child(
                        Button::new("toggle-sidebar")
                            .ghost()
                            .small()
                            .icon(sidebar_icon)
                            .accessibility_label(if self.sidebar_collapsed {
                                "Expand sidebar"
                            } else {
                                "Collapse sidebar"
                            })
                            .tooltip(if self.sidebar_collapsed {
                                "Expand sidebar"
                            } else {
                                "Collapse sidebar"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_collapsed = !this.sidebar_collapsed;
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child("LexWisp"),
                    ),
            )
    }

    fn render_workspace_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let context_icon = if self.context_panel_open {
            IconName::PanelRightClose
        } else {
            IconName::PanelRightOpen
        };

        h_flex()
            .h(px(ui_metrics::WORK_AREA_HEADER_HEIGHT))
            .w_full()
            .flex_none()
            .justify_between()
            .gap_3()
            .px_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(self.selected_title()),
            )
            .child(
                Button::new("toggle-context-panel")
                    .ghost()
                    .small()
                    .icon(context_icon)
                    .accessibility_label(if self.context_panel_open {
                        "Hide workspace context"
                    } else {
                        "Show workspace context"
                    })
                    .tooltip(if self.context_panel_open {
                        "Hide workspace context"
                    } else {
                        "Show workspace context"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.context_panel_open = !this.context_panel_open;
                        cx.notify();
                    })),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected_id = self.selected_conversation_id.clone();
        let items = self.conversations.iter().map(|conversation| {
            let conversation_id = conversation.id.clone();
            let is_active = selected_id.as_ref() == Some(&conversation_id);
            SidebarMenuItem::new(conversation.title.clone())
                .icon(IconName::Bot)
                .active(is_active)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_conversation_id = Some(conversation_id.clone());
                    cx.notify();
                }))
        });

        Sidebar::new("workspace-sidebar")
            .w(px(ui_metrics::SIDEBAR_WIDTH))
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(self.sidebar_collapsed)
            .header(
                v_flex().w_full().gap_3().child(
                    Button::new("new-conversation")
                        .ghost()
                        .small()
                        .icon(IconName::Plus)
                        .accessibility_label("New conversation")
                        .tooltip("New conversation")
                        .when(!self.sidebar_collapsed, |button| {
                            button.label("New conversation")
                        })
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.start_new_conversation(window, cx);
                        })),
                ),
            )
            .child(SidebarGroup::new("Conversations").child(SidebarMenu::new().children(items)))
    }

    fn render_message(message: &ChatMessage) -> impl IntoElement {
        let (alignment, sender, variant) = match message.speaker {
            Speaker::User => (MessageAlignment::End, "You", BubbleVariant::Tinted),
            Speaker::Assistant => (MessageAlignment::Start, "LexWisp", BubbleVariant::Ghost),
        };

        Message::new()
            .alignment(alignment)
            .header(MessageHeader::new().child(sender))
            .content(
                MessageContent::new().bubble(
                    Bubble::new()
                        .alignment(alignment)
                        .with_variant(variant)
                        .child(message.body.clone()),
                ),
            )
    }

    fn render_transcript(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let messages = self
            .selected_conversation()
            .map(|conversation| conversation.messages.as_slice())
            .unwrap_or_default();

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(ui_metrics::TRANSCRIPT_MAX_WIDTH))
                    .mx_auto()
                    .gap_6()
                    .px_6()
                    .py_8()
                    .when(messages.is_empty(), |this| {
                        this.flex_1().items_center().justify_center().child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .text_center()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_semibold()
                                        .child("What are you working on?"),
                                )
                                .child(
                                    div()
                                        .max_w_96()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(
                                            "Describe a task below. This UI lab keeps the interaction local and deterministic.",
                                        ),
                                ),
                        )
                    })
                    .children(messages.iter().map(Self::render_message)),
            )
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let can_send = !self.composer.read(cx).value().trim().is_empty();

        v_flex()
            .w_full()
            .max_w(px(ui_metrics::COMPOSER_MAX_WIDTH))
            .mx_auto()
            .gap_3()
            .p_3()
            .rounded(px(ui_metrics::RADIUS_COMPOSER))
            .border_1()
            .border_color(cx.theme().input)
            .bg(cx.theme().group_box)
            .child(
                Textarea::new(&self.composer)
                    .h_20()
                    .appearance(false)
                    .bordered(false)
                    .aria_label("Message"),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Enter to send · Shift+Enter for a new line"),
                    )
                    .child(
                        Button::new("send-message")
                            .primary()
                            .small()
                            .icon(IconName::ArrowUp)
                            .accessibility_label("Send message")
                            .tooltip("Send message")
                            .disabled(!can_send)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.send_message(window, cx);
                            })),
                    ),
            )
    }

    fn render_context_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .h_full()
            .w(px(ui_metrics::CONTEXT_PANEL_WIDTH))
            .flex_none()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .text_color(cx.theme().group_box_foreground)
            .child(
                h_flex()
                    .h_10()
                    .px_4()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().text_sm().font_semibold().child("Workspace context"))
                    .child(
                        Button::new("close-context-panel")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .accessibility_label("Hide workspace context")
                            .tooltip("Hide workspace context")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.context_panel_open = false;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                v_flex()
                    .gap_6()
                    .p_4()
                    .child(context_detail("Current surface", self.selected_title(), cx))
                    .child(context_detail(
                        "Reference",
                        "ChatGPT-style integrated workspace chrome",
                        cx,
                    ))
                    .child(context_detail(
                        "Runtime",
                        "Deterministic local UI state",
                        cx,
                    )),
            )
    }
}

impl Render for ChartMainWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .min_h_0()
            .bg(cx.theme().transparent)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_sidebar(cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .min_h_0()
                            .flex_1()
                            .overflow_hidden()
                            .rounded_t(cx.theme().radius_lg)
                            .bg(cx.theme().group_box)
                            .child(self.render_workspace_header(cx))
                            .child(
                                h_flex()
                                    .items_stretch()
                                    .flex_1()
                                    .min_h_0()
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .min_h_0()
                                            .flex_1()
                                            .child(self.render_transcript(cx))
                                            .child(
                                                div()
                                                    .w_full()
                                                    .px_6()
                                                    .pb_6()
                                                    .child(self.render_composer(cx)),
                                            ),
                                    )
                                    .when(self.context_panel_open, |this| {
                                        this.child(self.render_context_panel(cx))
                                    }),
                            ),
                    ),
            )
    }
}

fn context_detail(
    label: &'static str,
    value: impl Into<SharedString>,
    cx: &Context<ChartMainWindow>,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(div().text_sm().child(value.into()))
}

fn conversation_title(message: &str) -> SharedString {
    const MAX_CHARS: usize = 34;
    let mut chars = message.chars();
    let title: String = chars.by_ref().take(MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{title}…").into()
    } else {
        title.into()
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);
        let bounds = Bounds::centered(None, ui_metrics::workspace_shell_size(), cx);

        cx.spawn(async move |cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(ui_metrics::workspace_min_size()),
                titlebar: Some(TitlebarOptions {
                    title: Some("Chart main window".into()),
                    ..TitleBar::title_bar_options()
                }),
                window_decorations: Some(WindowDecorations::Client),
                window_background: WindowBackgroundAppearance::MicaBackdrop,
                ..TitleBar::window_options()
            };
            cx.open_window(options, |window, cx| {
                theme::apply_theme(ThemePreference::System, window, cx);
                let view = cx.new(|cx| ChartMainWindow::new(window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().transparent))
            })
            .expect("failed to open LexWisp UI lab window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use super::conversation_title;

    #[test]
    fn conversation_title_keeps_short_text() {
        assert_eq!(conversation_title("Short task").as_ref(), "Short task");
    }

    #[test]
    fn conversation_title_truncates_by_character() {
        let title = conversation_title(&"标题".repeat(20));
        assert_eq!(title.chars().count(), 35);
        assert!(title.ends_with('…'));
    }
}

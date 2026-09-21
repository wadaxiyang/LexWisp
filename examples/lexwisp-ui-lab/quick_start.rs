pub mod theme;
pub mod ui_metrics;

use gpui_kit::component::{
    ActiveTheme, Disableable, Root, StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{
    AppContext, Context, IntoElement, ParentElement, Render, Styled, Subscription, TitlebarOptions,
    Window, WindowOptions, div,
};

use theme::ThemePreference;

pub struct QuickStart {
    ready: bool,
    _appearance: Subscription,
}

impl QuickStart {
    fn new(window: &mut Window) -> Self {
        let appearance = window.observe_window_appearance(|window, cx| {
            theme::sync_system_theme(window, cx);
        });
        Self {
            ready: false,
            _appearance: appearance,
        }
    }
}

impl Render for QuickStart {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .p_6()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .v_flex()
                    .w_full()
                    .max_w_96()
                    .gap_4()
                    .p_6()
                    .rounded(cx.theme().radius_lg)
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(div().text_lg().font_semibold().child("LexWisp UI Lab"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(if self.ready {
                                "The local design system is active."
                            } else {
                                "A standalone GPUI-Kit workspace for LexWisp UI examples."
                            }),
                    )
                    .child(
                        Button::new("start-designing")
                            .primary()
                            .label(if self.ready {
                                "Ready"
                            } else {
                                "Start designing"
                            })
                            .disabled(self.ready)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.ready = true;
                                cx.notify();
                            })),
                    ),
            )
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(gpui_kit::assets::Assets);

    app.run(move |cx| {
        gpui_kit::init(cx);

        cx.spawn(async move |cx| {
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("LexWisp UI Lab · Quick Start".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            cx.open_window(options, |window, cx| {
                theme::apply_theme(ThemePreference::System, window, cx);
                let view = cx.new(|_| QuickStart::new(window));
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open LexWisp UI lab window");
        })
        .detach();
    });
}

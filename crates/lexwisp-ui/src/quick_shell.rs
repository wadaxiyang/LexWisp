use gpui_kit::component::{
    ActiveTheme, Icon, IconName,
    button::{Button, ButtonVariants},
};
use gpui_kit::{Context, IntoElement, ParentElement, Render, Styled, WeakEntity, Window, div};

use crate::SurfaceController;

pub struct QuickShell {
    controller: WeakEntity<SurfaceController>,
}

impl QuickShell {
    pub fn new(controller: WeakEntity<SurfaceController>) -> Self {
        Self { controller }
    }
}

impl Render for QuickShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings_controller = self.controller.clone();
        let hide_controller = self.controller.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_6()
            .gap_5()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(Icon::new(IconName::Sun).text_color(cx.theme().primary))
                    .child(div().text_2xl().child("LexWisp"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("STAGE 1 · SYSTEM SHELL"),
                    ),
            )
            .child(div().text_lg().child("Native shell is ready."))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Use the global shortcut or tray icon to reopen this window. AI actions are introduced in later stages."),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        Button::new("settings")
                            .primary()
                            .label("Settings…")
                            .on_click(move |_, _, cx| {
                                let _ = settings_controller.update(cx, |controller, cx| {
                                    let _ = controller.show_control_center(cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("hide")
                            .label("Hide")
                            .on_click(move |_, window, cx| {
                                let _ = hide_controller.update(cx, |controller, cx| {
                                    controller.hide_quick_shell(window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("quit")
                            .label("Exit LexWisp")
                            .on_click(|_, _, cx| cx.quit()),
                    ),
            )
    }
}

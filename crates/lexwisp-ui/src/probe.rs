use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Theme, ThemeMode, WindowExt,
    button::{Button, ButtonVariants},
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
};
use gpui_kit::{
    AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, px,
};

pub(crate) struct Probe {
    label: Entity<InputState>,
    input: Entity<TextareaState>,
    result: Entity<TextareaState>,
    status: SharedString,
    submissions: usize,
    _input_events: Subscription,
}

impl Probe {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let label = cx.new(|cx| {
            InputState::new(window, cx).default_value("本地输入验证 / Local input check")
        });
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(4)
                .submit_on_enter(true)
                .placeholder("输入中文或 English，然后按 Enter。Shift+Enter 换行。")
        });
        let result = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(5)
                .default_value("这里显示你提交的原文。可以拖选文字并按 Ctrl+C 复制。\nThis is a local preview. No network request is made.")
        });
        let input_events = cx.subscribe_in(&input, window, |this, _, event, window, cx| {
            if matches!(
                event,
                InputEvent::PressEnter {
                    shift: false,
                    secondary: false
                }
            ) {
                this.submit(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            label,
            input,
            result,
            status: "就绪 · 仅在内存中预览，不保存、不联网".into(),
            submissions: 0,
            _input_events: input_events,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.input.read(cx).value();
        if value.trim().is_empty() {
            self.status = "请先输入文字。".into();
        } else if value.len() > 128 * 1024 {
            self.status = "输入超过 128 KiB，请缩短后再提交。".into();
        } else {
            self.result
                .update(cx, |result, cx| result.set_value(value, window, cx));
            self.submissions += 1;
            self.status = format!("已预览 {} 次 · 本地原文，没有 AI 生成", self.submissions).into();
        }
        cx.notify();
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_6()
            .gap_4()
            .child(
                div().flex().items_center().gap_3()
                    .child(Icon::new(IconName::Sun).text_color(cx.theme().primary))
                    .child(div().text_2xl().child("LexWisp"))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("STAGE 0 · 原生窗口验证")),
            )
            .child(div().text_sm().text_color(cx.theme().muted_foreground)
                .child("验证输入、文字选择与窗口生命周期。AI、热键和托盘将在后续阶段接入。"))
            .child(Input::new(&self.label).aria_label("验证标题"))
            .child(Textarea::new(&self.input).aria_label("输入文本").h(px(120.)))
            .child(
                div().flex().items_center().gap_2()
                    .child(Button::new("preview").primary().label("预览原文").on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Enter 提交 · Shift+Enter 换行")),
            )
            .child(div().text_sm().child("结果 · 可选择、可复制、只读"))
            .child(Textarea::new(&self.result).readonly(true).aria_label("只读结果").h(px(150.)))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.status.clone()))
            .child(
                div().flex().gap_2()
                    .child(Button::new("theme").label("切换主题").on_click(|_, window, cx| {
                        let mode = if cx.theme().is_dark() { ThemeMode::Light } else { ThemeMode::Dark };
                        Theme::change(mode, Some(window), cx);
                    }))
                    .child(Button::new("overlay").label("关于验证").on_click(|_, window, cx| {
                        window.open_dialog(cx, |dialog, _, _| dialog.title("Stage 0 验证")
                            .child("输入内容仅用于本地预览。按 Escape 或关闭按钮返回，检查焦点恢复。"));
                    }))
                    .child(Button::new("recreate").label("重建窗口").on_click(|_, window, cx| {
                        // Create first: a failed replacement must leave a usable window.
                        match crate::open_probe_window(cx) {
                            Ok(_) => window.remove_window(),
                            Err(error) => {
                                let message: SharedString = format!("窗口创建失败：{error}").into();
                                window.open_dialog(cx, move |dialog, _, _| dialog.title("无法重建窗口").child(message.clone()));
                            }
                        }
                    }))
                    .child(Button::new("quit").label("退出").on_click(|_, _, cx| cx.quit())),
            )
    }
}

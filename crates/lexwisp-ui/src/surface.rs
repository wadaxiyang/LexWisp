use gpui_kit::component::{ActiveTheme, Root};
use gpui_kit::{
    AnyView, App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions, div, px, size,
};

use crate::probe::Probe;

fn build_window_options(cx: &App) -> WindowOptions {
    // Window bounds are a platform geometry boundary, so GPUI requires resolved
    // pixel dimensions here. Product content below the window root stays on the
    // rem-based layout scale.
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(760.), px(680.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("LexWisp · Stage 0".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(620.), px(600.))),
        app_id: Some("org.lexwisp.LexWisp".into()),
        ..Default::default()
    }
}

/// Window-level content and overlay composition; no plugin or request state.
struct LexWispWindowRoot {
    content: AnyView,
}

impl Render for LexWispWindowRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(self.content.clone())
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}

fn open_surface_window(
    cx: &mut App,
    create_content: impl FnOnce(&mut Window, &mut App) -> AnyView + 'static,
) -> anyhow::Result<WindowHandle<Root>> {
    cx.open_window(build_window_options(cx), |window, cx| {
        let content = create_content(window, cx);
        let shell = cx.new(|_| LexWispWindowRoot { content });
        cx.new(|cx| Root::new(shell, window, cx))
    })
}

pub fn open_probe_window(cx: &mut App) -> anyhow::Result<WindowHandle<Root>> {
    open_surface_window(cx, |window, cx| cx.new(|cx| Probe::new(window, cx)).into())
}

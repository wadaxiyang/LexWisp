use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{
    AnyView, App, AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled,
    Subscription, Task, TitlebarOptions, Window, WindowBounds, WindowHandle, WindowId,
    WindowOptions, div, px, size,
};
use lexwisp_core::{SettingsUiPort, SurfaceKind, ThemePreference};

use crate::{control_center::ControlCenter, quick_shell::QuickShell};

pub trait SurfaceWindowPlatform {
    fn hide(&self, window: &Window) -> Result<(), String>;
    fn show(&self, window: &Window) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceState {
    Visible,
    HiddenWarm,
}

struct WindowEntry {
    handle: WindowHandle<Root>,
    state: SurfaceState,
    generation: u64,
    warm_expiry: Option<Task<()>>,
}

#[derive(Default)]
pub struct WindowRegistry {
    entries: HashMap<SurfaceKind, WindowEntry>,
    next_generation: u64,
}

pub struct SurfaceController {
    platform: Rc<dyn SurfaceWindowPlatform>,
    settings: Arc<dyn SettingsUiPort>,
    registry: WindowRegistry,
}

pub type SurfaceFactory = SurfaceController;

impl SurfaceController {
    pub fn new(platform: Rc<dyn SurfaceWindowPlatform>, settings: Arc<dyn SettingsUiPort>) -> Self {
        Self {
            platform,
            settings,
            registry: WindowRegistry::default(),
        }
    }

    pub fn show_quick_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(SurfaceKind::QuickShell, cx)
    }

    pub fn toggle_quick_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if self
            .registry
            .entries
            .get(&SurfaceKind::QuickShell)
            .is_some_and(|entry| entry.state == SurfaceState::Visible)
        {
            if let Some(handle) = self
                .registry
                .entries
                .get(&SurfaceKind::QuickShell)
                .map(|entry| entry.handle)
            {
                let platform = self.platform.clone();
                handle.update(cx, move |_, window, _| {
                    platform.hide(window).map_err(anyhow::Error::msg)
                })??;
                self.begin_warm_retention(cx);
            }
            Ok(())
        } else {
            self.show_quick_shell(cx)
        }
    }

    pub fn show_control_center(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(SurfaceKind::ControlCenter, cx)
    }

    pub fn hide_quick_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.platform.hide(window).is_err() {
            window.remove_window();
            self.registry.entries.remove(&SurfaceKind::QuickShell);
            return;
        }
        self.begin_warm_retention(cx);
    }

    fn begin_warm_retention(&mut self, cx: &mut Context<Self>) {
        let Some(entry) = self.registry.entries.get_mut(&SurfaceKind::QuickShell) else {
            return;
        };
        entry.state = SurfaceState::HiddenWarm;
        entry.generation = entry.generation.saturating_add(1);
        let generation = entry.generation;
        let retention = self
            .settings
            .snapshot()
            .settings()
            .popup_retention_seconds();
        let timer = cx
            .background_executor()
            .timer(Duration::from_secs(retention));
        entry.warm_expiry = Some(cx.spawn(async move |controller, cx| {
            timer.await;
            let handle = controller
                .update(cx, |controller, _| {
                    controller.take_warm_quick_shell(generation)
                })
                .ok()
                .flatten();
            if let Some(handle) = handle {
                cx.update(|cx| {
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    });
                });
            }
        }));
    }

    pub fn window_closed(&mut self, id: WindowId) {
        self.registry
            .entries
            .retain(|_, entry| entry.handle.window_id() != id);
    }

    fn show(&mut self, kind: SurfaceKind, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if let Some(entry) = self.registry.entries.get_mut(&kind) {
            entry.warm_expiry = None;
            entry.state = SurfaceState::Visible;
            let platform = self.platform.clone();
            let preference = self.settings.snapshot().settings().theme();
            let shown = entry.handle.update(cx, |_, window, cx| {
                apply_theme(preference, window, cx);
                platform.show(window).map_err(anyhow::Error::msg)?;
                window.activate_window();
                Ok::<(), anyhow::Error>(())
            });
            if matches!(shown, Ok(Ok(()))) {
                return Ok(());
            }
            self.registry.entries.remove(&kind);
        }

        self.registry.next_generation = self.registry.next_generation.saturating_add(1);
        let generation = self.registry.next_generation;
        let controller = cx.weak_entity();
        let settings = self.settings.clone();
        let preference = settings.snapshot().settings().theme();
        let options = build_window_options(kind, cx);
        let handle = cx.open_window(options, move |window, cx| {
            apply_theme(preference, window, cx);
            if kind == SurfaceKind::QuickShell {
                let controller_for_close = controller.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    let _ = controller_for_close.update(cx, |controller, cx| {
                        controller.hide_quick_shell(window, cx);
                    });
                    false
                });
            }
            let content: AnyView = match kind {
                SurfaceKind::QuickShell => cx.new(|_| QuickShell::new(controller.clone())).into(),
                SurfaceKind::ControlCenter => {
                    cx.new(|_| ControlCenter::new(settings.clone())).into()
                }
            };
            let shell =
                cx.new(|_| LexWispWindowRoot::new(content, settings.clone(), preference, window));
            cx.new(|cx| Root::new(shell, window, cx))
        })?;
        self.registry.entries.insert(
            kind,
            WindowEntry {
                handle,
                state: SurfaceState::Visible,
                generation,
                warm_expiry: None,
            },
        );
        Ok(())
    }

    fn take_warm_quick_shell(&mut self, generation: u64) -> Option<WindowHandle<Root>> {
        let should_destroy = self
            .registry
            .entries
            .get(&SurfaceKind::QuickShell)
            .is_some_and(|entry| {
                entry.state == SurfaceState::HiddenWarm && entry.generation == generation
            });
        if !should_destroy {
            return None;
        }
        if let Some(mut entry) = self.registry.entries.remove(&SurfaceKind::QuickShell) {
            // This callback is running inside the retained warm-expiry task. Detach that now-ready
            // handle before dropping the entry so removal cannot cancel the task that is currently
            // unwinding its own future.
            if let Some(task) = entry.warm_expiry.take() {
                task.detach();
            }
            Some(entry.handle)
        } else {
            None
        }
    }
}

fn build_window_options(kind: SurfaceKind, cx: &App) -> WindowOptions {
    let (title, dimensions, minimum) = match kind {
        SurfaceKind::QuickShell => (
            "LexWisp · Quick Shell",
            size(px(560.), px(310.)),
            size(px(480.), px(280.)),
        ),
        SurfaceKind::ControlCenter => (
            "LexWisp · Settings",
            size(px(720.), px(680.)),
            size(px(620.), px(600.)),
        ),
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None, dimensions, cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        window_min_size: Some(minimum),
        app_id: Some("org.lexwisp.LexWisp".into()),
        ..Default::default()
    }
}

pub(crate) fn apply_theme(preference: ThemePreference, window: &mut Window, cx: &mut App) {
    let mode = match preference {
        ThemePreference::System => window.appearance().into(),
        ThemePreference::Light => ThemeMode::Light,
        ThemePreference::Dark => ThemeMode::Dark,
    };
    Theme::change(mode, Some(window), cx);
}

struct LexWispWindowRoot {
    content: AnyView,
    _appearance: Option<Subscription>,
}

impl LexWispWindowRoot {
    fn new(
        content: AnyView,
        settings: Arc<dyn SettingsUiPort>,
        _preference: ThemePreference,
        window: &mut Window,
    ) -> Self {
        let appearance = Some({
            window.observe_window_appearance(move |window, cx| {
                if settings.snapshot().settings().theme() == ThemePreference::System {
                    Theme::change(window.appearance(), Some(window), cx);
                }
            })
        });
        Self {
            content,
            _appearance: appearance,
        }
    }
}

impl Render for LexWispWindowRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::ActiveTheme as _;

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

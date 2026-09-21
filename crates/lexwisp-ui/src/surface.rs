use std::{rc::Rc, sync::Arc, time::Duration};

use gpui_kit::component::{ActiveTheme, Root};
use gpui_kit::{
    AnyView, App, AppContext, Bounds, Context, DisplayId, Entity, IntoElement, ParentElement,
    Render, Styled, Subscription, Task, Window, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowHandle, WindowId, WindowKind, WindowOptions, div,
};
use lexwisp_core::{
    ChatUiPort, HistoryUiPort, ProviderUiPort, SettingsUiPort, SurfaceKind, ThemePreference,
};

use crate::{ChatExperience, theme::apply_theme, ui_metrics};

pub trait SurfaceWindowPlatform {
    fn active_display_id(&self) -> Option<u64>;
    fn hide(&self, window: &Window) -> Result<(), String>;
    fn show(&self, window: &Window) -> Result<(), String>;
    fn set_pinned(&self, window: &Window, pinned: bool) -> Result<(), String>;
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SurfaceState {
    Visible,
    HiddenWarm,
}

struct WindowEntry {
    handle: WindowHandle<Root>,
    view: Entity<ChatExperience>,
    state: SurfaceState,
    generation: u64,
    warm_token: u64,
    warm_expiry: Option<Task<()>>,
}

pub struct SurfaceController {
    platform: Rc<dyn SurfaceWindowPlatform>,
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    chat: Arc<dyn ChatUiPort>,
    history: Arc<dyn HistoryUiPort>,
    entry: Option<WindowEntry>,
    next_generation: u64,
    next_warm_token: u64,
    pinned: bool,
}

#[derive(Clone)]
pub struct SurfaceServices {
    pub settings: Arc<dyn SettingsUiPort>,
    pub providers: Arc<dyn ProviderUiPort>,
    pub chat: Arc<dyn ChatUiPort>,
    pub history: Arc<dyn HistoryUiPort>,
}

impl SurfaceController {
    pub fn new(platform: Rc<dyn SurfaceWindowPlatform>, services: SurfaceServices) -> Self {
        Self {
            platform,
            settings: services.settings,
            providers: services.providers,
            chat: services.chat,
            history: services.history,
            entry: None,
            next_generation: 0,
            next_warm_token: 0,
            pinned: false,
        }
    }

    pub fn show_main_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(cx)?;
        if let Some(entry) = &self.entry {
            entry.view.update(cx, |view, cx| view.show_chat(cx));
            let view = entry.view.clone();
            let _ = entry.handle.update(cx, |_, window, cx| {
                view.update(cx, |view, cx| view.focus_composer(window, cx));
            });
        }
        Ok(())
    }

    pub fn show_settings(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(cx)?;
        if let Some(entry) = &self.entry {
            entry.view.update(cx, |view, cx| view.show_settings(cx));
        }
        Ok(())
    }

    pub fn toggle_main_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.state == SurfaceState::Visible)
        {
            self.hide_main_shell(cx)
        } else {
            self.show_main_shell(cx)
        }
    }

    pub fn hide_main_shell_from_view(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.hide_main_shell(cx);
    }

    pub fn set_pinned(
        &mut self,
        pinned: bool,
        window: &mut Window,
        _: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        self.platform
            .set_pinned(window, pinned)
            .map_err(anyhow::Error::msg)?;
        self.pinned = pinned;
        Ok(())
    }

    fn hide_main_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let Some(entry) = &self.entry else {
            return Ok(());
        };
        let handle = entry.handle;
        let platform = self.platform.clone();
        handle.update(cx, move |_, window, _| {
            platform.hide(window).map_err(anyhow::Error::msg)
        })??;
        self.begin_warm_retention(cx);
        Ok(())
    }

    fn hide_if_generation(&mut self, generation: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .entry
            .as_ref()
            .is_none_or(|entry| entry.generation != generation)
        {
            window.remove_window();
            return;
        }
        self.chat.set_surface_visible(SurfaceKind::MainShell, false);
        if self.platform.hide(window).is_err() {
            window.remove_window();
            self.entry = None;
            return;
        }
        self.begin_warm_retention(cx);
    }

    fn begin_warm_retention(&mut self, cx: &mut Context<Self>) {
        self.chat.set_surface_visible(SurfaceKind::MainShell, false);
        self.next_warm_token = self.next_warm_token.saturating_add(1);
        let token = self.next_warm_token;
        let Some(entry) = &mut self.entry else {
            return;
        };
        entry.state = SurfaceState::HiddenWarm;
        entry.warm_token = token;
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
                .update(cx, |controller, _| controller.take_warm_window(token))
                .ok()
                .flatten();
            if let Some(handle) = handle {
                cx.update(|cx| {
                    cx.defer(move |cx| {
                        let _ = handle.update(cx, |_, window, _| window.remove_window());
                    })
                });
            }
        }));
    }

    fn take_warm_window(&mut self, token: u64) -> Option<WindowHandle<Root>> {
        if self.entry.as_ref().is_none_or(|entry| {
            entry.state != SurfaceState::HiddenWarm || entry.warm_token != token
        }) {
            return None;
        }
        self.entry.take().map(|entry| entry.handle)
    }

    pub fn window_closed(&mut self, id: WindowId) {
        if self
            .entry
            .as_ref()
            .is_some_and(|entry| entry.handle.window_id() == id)
        {
            self.chat.set_surface_visible(SurfaceKind::MainShell, false);
            self.entry = None;
        }
    }

    fn show(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if let Some(entry) = &mut self.entry {
            entry.warm_expiry = None;
            entry.state = SurfaceState::Visible;
            let platform = self.platform.clone();
            let preference = self.settings.snapshot().settings().theme();
            let shown = entry.handle.update(cx, move |_, window, cx| {
                apply_theme(preference, window, cx);
                platform.show(window).map_err(anyhow::Error::msg)?;
                window.activate_window();
                Ok::<(), anyhow::Error>(())
            });
            if matches!(shown, Ok(Ok(()))) {
                self.chat.set_surface_visible(SurfaceKind::MainShell, true);
                return Ok(());
            }
            self.entry = None;
        }

        self.next_generation = self.next_generation.saturating_add(1);
        let generation = self.next_generation;
        let controller = cx.weak_entity();
        let settings = self.settings.clone();
        let providers = self.providers.clone();
        let chat = self.chat.clone();
        let history = self.history.clone();
        let preference = settings.snapshot().settings().theme();
        let pinned = self.pinned;
        let display_id = self.platform.active_display_id().map(DisplayId::new);
        let options = build_window_options(display_id, cx);
        let mut created_view = None;
        let handle = cx.open_window(options, |window, cx| {
            apply_theme(preference, window, cx);
            let controller_for_close = controller.clone();
            window.on_window_should_close(cx, move |window, cx| {
                let _ = controller_for_close.update(cx, |controller, cx| {
                    controller.hide_if_generation(generation, window, cx)
                });
                false
            });
            let view = cx.new(|cx| {
                ChatExperience::new(
                    controller.clone(),
                    SurfaceServices {
                        settings: settings.clone(),
                        providers: providers.clone(),
                        chat: chat.clone(),
                        history: history.clone(),
                    },
                    pinned,
                    window,
                    cx,
                )
            });
            created_view = Some(view.clone());
            let shell = cx.new(|_| LexWispWindowRoot::new(view.into(), settings.clone(), window));
            cx.new(|cx| Root::new(shell, window, cx).bg(cx.theme().transparent))
        })?;
        if pinned {
            let platform = self.platform.clone();
            handle.update(cx, move |_, window, _| {
                platform
                    .set_pinned(window, true)
                    .map_err(anyhow::Error::msg)
            })??;
        }
        self.entry = Some(WindowEntry {
            handle,
            view: created_view.expect("popup view is created with its window"),
            state: SurfaceState::Visible,
            generation,
            warm_token: 0,
            warm_expiry: None,
        });
        self.chat.set_surface_visible(SurfaceKind::MainShell, true);
        Ok(())
    }
}

fn build_window_options(display_id: Option<DisplayId>, cx: &App) -> WindowOptions {
    let dimensions = ui_metrics::main_window_size();
    let bounds = display_id
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display())
        .map(|display| Bounds::centered_at(display.visible_bounds().center(), dimensions))
        .unwrap_or_else(|| Bounds::centered(display_id, dimensions, cx));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: None,
        kind: WindowKind::Normal,
        window_decorations: Some(WindowDecorations::Client),
        window_background: WindowBackgroundAppearance::MicaBackdrop,
        is_resizable: false,
        is_minimizable: false,
        window_min_size: Some(dimensions),
        app_id: Some("org.lexwisp.LexWisp".into()),
        ..WindowOptions::default()
    }
}

struct LexWispWindowRoot {
    content: AnyView,
    _appearance: Subscription,
}

impl LexWispWindowRoot {
    fn new(content: AnyView, settings: Arc<dyn SettingsUiPort>, window: &mut Window) -> Self {
        let appearance = window.observe_window_appearance(move |window, cx| {
            if settings.snapshot().settings().theme() == ThemePreference::System {
                apply_theme(ThemePreference::System, window, cx);
            }
        });
        Self {
            content,
            _appearance: appearance,
        }
    }
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

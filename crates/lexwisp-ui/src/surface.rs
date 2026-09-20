use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{
    AnyView, App, AppContext, Bounds, Context, DisplayId, IntoElement, ParentElement, Pixels,
    Render, Size, Styled, Subscription, Task, TitlebarOptions, WeakEntity, Window, WindowBounds,
    WindowHandle, WindowId, WindowOptions, div, px, size,
};
use lexwisp_core::{
    ActionDescriptor, ChatUiPort, ContextSnapshot, HistoryUiPort, PluginManagementUiPort,
    ProviderUiPort, SettingsUiPort, SurfaceKind, TextActionUiPort, ThemePreference,
};

use crate::control_center::ControlCenter;

pub trait SurfaceWindowPlatform {
    fn active_display_id(&self) -> Option<u64>;
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
    warm_token: u64,
    warm_expiry: Option<Task<()>>,
}

#[derive(Default)]
pub struct WindowRegistry {
    entries: HashMap<SurfaceKind, WindowEntry>,
    next_generation: u64,
    next_warm_token: u64,
}

impl WindowRegistry {
    fn allocate_generation(&mut self) -> u64 {
        self.next_generation = self.next_generation.saturating_add(1);
        self.next_generation
    }

    fn allocate_warm_token(&mut self) -> u64 {
        self.next_warm_token = self.next_warm_token.saturating_add(1);
        self.next_warm_token
    }
}

pub struct SurfaceController {
    platform: Rc<dyn SurfaceWindowPlatform>,
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    chat: Arc<dyn ChatUiPort>,
    history: Arc<dyn HistoryUiPort>,
    plugin_management: Arc<dyn PluginManagementUiPort>,
    text_actions: Vec<Arc<dyn TextActionUiPort>>,
    launches: async_channel::Sender<ContextSnapshot>,
    action_descriptors: Vec<ActionDescriptor>,
    quick_shell_factory: QuickShellViewFactory,
    chat_panel_factory: ChatPanelViewFactory,
    registry: WindowRegistry,
}

pub struct SurfaceServices {
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    chat: Arc<dyn ChatUiPort>,
    history: Arc<dyn HistoryUiPort>,
    plugin_management: Arc<dyn PluginManagementUiPort>,
    text_actions: Vec<Arc<dyn TextActionUiPort>>,
    action_descriptors: Vec<ActionDescriptor>,
}

impl SurfaceServices {
    pub fn new(
        settings: Arc<dyn SettingsUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        chat: Arc<dyn ChatUiPort>,
        history: Arc<dyn HistoryUiPort>,
        plugin_management: Arc<dyn PluginManagementUiPort>,
        text_actions: Vec<Arc<dyn TextActionUiPort>>,
        action_descriptors: Vec<ActionDescriptor>,
    ) -> Self {
        Self {
            settings,
            providers,
            chat,
            history,
            plugin_management,
            text_actions,
            action_descriptors,
        }
    }
}

pub type QuickShellViewFactory =
    Rc<dyn Fn(WeakEntity<SurfaceController>, &mut Window, &mut App) -> AnyView>;
pub type ChatPanelViewFactory =
    Rc<dyn Fn(WeakEntity<SurfaceController>, &mut Window, &mut App) -> AnyView>;
pub type SurfaceFactory = SurfaceController;

impl SurfaceController {
    pub fn new(
        platform: Rc<dyn SurfaceWindowPlatform>,
        services: SurfaceServices,
        launches: async_channel::Sender<ContextSnapshot>,
        quick_shell_factory: QuickShellViewFactory,
        chat_panel_factory: ChatPanelViewFactory,
    ) -> Self {
        Self {
            platform,
            settings: services.settings,
            providers: services.providers,
            chat: services.chat,
            history: services.history,
            plugin_management: services.plugin_management,
            text_actions: services.text_actions,
            launches,
            action_descriptors: services.action_descriptors,
            quick_shell_factory,
            chat_panel_factory,
            registry: WindowRegistry::default(),
        }
    }

    pub fn show_quick_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(SurfaceKind::QuickShell, cx)
    }

    pub fn toggle_quick_shell(
        &mut self,
        snapshot: ContextSnapshot,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
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
            self.launches
                .try_send(snapshot)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            self.show_quick_shell(cx)
        }
    }

    pub fn show_control_center(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(SurfaceKind::ControlCenter, cx)
    }

    pub fn refresh_plugins(&mut self, cx: &mut Context<Self>) {
        if let Some(entry) = self.registry.entries.remove(&SurfaceKind::QuickShell) {
            self.chat
                .set_surface_visible(SurfaceKind::QuickShell, false);
            for action in self.all_text_actions() {
                action.set_surface_visible(false);
            }
            let handle = entry.handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    fn all_text_actions(&self) -> Vec<Arc<dyn TextActionUiPort>> {
        let mut actions = self.text_actions.clone();
        actions.extend(self.plugin_management.action_snapshot().controllers);
        actions
    }

    pub fn handoff_to_chat_panel(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        // Attach the destination observer before the popup is detached so an in-flight
        // invocation always has a visible projection throughout the handoff.
        self.show(SurfaceKind::ChatPanel, cx)?;
        let handle = self
            .registry
            .entries
            .get(&SurfaceKind::QuickShell)
            .filter(|entry| entry.state == SurfaceState::Visible)
            .map(|entry| entry.handle);
        if let Some(handle) = handle {
            let platform = self.platform.clone();
            handle.update(cx, move |_, window, _| {
                platform.hide(window).map_err(anyhow::Error::msg)
            })??;
            self.begin_warm_retention(cx);
        }
        Ok(())
    }

    pub fn close_chat_panel(&mut self, window: &mut Window, _: &mut Context<Self>) {
        self.chat.set_surface_visible(SurfaceKind::ChatPanel, false);
        self.registry.entries.remove(&SurfaceKind::ChatPanel);
        window.remove_window();
    }

    fn close_chat_panel_if_generation(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_generation(SurfaceKind::ChatPanel, generation) {
            window.remove_window();
            return;
        }
        self.close_chat_panel(window, cx);
    }

    pub fn hide_quick_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_current_quick_shell(window, cx);
    }

    fn hide_quick_shell_if_generation(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_generation(SurfaceKind::QuickShell, generation) {
            window.remove_window();
            return;
        }
        self.hide_current_quick_shell(window, cx);
    }

    fn hide_current_quick_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.chat
            .set_surface_visible(SurfaceKind::QuickShell, false);
        for action in self.all_text_actions() {
            action.set_surface_visible(false);
        }
        if self.platform.hide(window).is_err() {
            window.remove_window();
            self.registry.entries.remove(&SurfaceKind::QuickShell);
            return;
        }
        self.begin_warm_retention(cx);
    }

    fn is_current_generation(&self, kind: SurfaceKind, generation: u64) -> bool {
        self.registry
            .entries
            .get(&kind)
            .is_some_and(|entry| entry.generation == generation)
    }

    fn begin_warm_retention(&mut self, cx: &mut Context<Self>) {
        self.chat
            .set_surface_visible(SurfaceKind::QuickShell, false);
        for action in self.all_text_actions() {
            action.set_surface_visible(false);
        }
        let warm_token = self.registry.allocate_warm_token();
        let Some(entry) = self.registry.entries.get_mut(&SurfaceKind::QuickShell) else {
            return;
        };
        entry.state = SurfaceState::HiddenWarm;
        entry.warm_token = warm_token;
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
                    controller.take_warm_quick_shell(warm_token)
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
        let closed = self
            .registry
            .entries
            .iter()
            .filter_map(|(kind, entry)| (entry.handle.window_id() == id).then_some(*kind))
            .collect::<Vec<_>>();
        for kind in closed {
            if kind == SurfaceKind::QuickShell {
                self.chat
                    .set_surface_visible(SurfaceKind::QuickShell, false);
                for action in self.all_text_actions() {
                    action.set_surface_visible(false);
                }
            } else if kind == SurfaceKind::ChatPanel {
                self.chat.set_surface_visible(SurfaceKind::ChatPanel, false);
            }
        }
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
                if kind == SurfaceKind::QuickShell {
                    self.chat.set_surface_visible(SurfaceKind::QuickShell, true);
                    for action in self.all_text_actions() {
                        action.set_surface_visible(true);
                    }
                } else if kind == SurfaceKind::ChatPanel {
                    self.chat.set_surface_visible(SurfaceKind::ChatPanel, true);
                }
                return Ok(());
            }
            self.registry.entries.remove(&kind);
        }

        let generation = self.registry.allocate_generation();
        let controller = cx.weak_entity();
        let settings = self.settings.clone();
        let providers = self.providers.clone();
        let action_descriptors = self.action_descriptors.clone();
        let history = self.history.clone();
        let plugin_management = self.plugin_management.clone();
        let quick_shell_factory = self.quick_shell_factory.clone();
        let chat_panel_factory = self.chat_panel_factory.clone();
        let preference = settings.snapshot().settings().theme();
        let display_id = self.platform.active_display_id().map(DisplayId::new);
        let options = build_window_options(kind, display_id, cx);
        let handle = cx.open_window(options, move |window, cx| {
            apply_theme(preference, window, cx);
            if kind == SurfaceKind::QuickShell {
                let controller_for_close = controller.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    let _ = controller_for_close.update(cx, |controller, cx| {
                        controller.hide_quick_shell_if_generation(generation, window, cx);
                    });
                    false
                });
            } else if kind == SurfaceKind::ChatPanel {
                let controller_for_close = controller.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    let _ = controller_for_close.update(cx, |controller, cx| {
                        controller.close_chat_panel_if_generation(generation, window, cx);
                    });
                    false
                });
            }
            let content: AnyView = match kind {
                SurfaceKind::QuickShell => quick_shell_factory(controller.clone(), window, cx),
                SurfaceKind::ChatPanel => chat_panel_factory(controller.clone(), window, cx),
                SurfaceKind::ControlCenter => cx
                    .new(|cx| {
                        ControlCenter::new(
                            settings.clone(),
                            providers.clone(),
                            history.clone(),
                            plugin_management.clone(),
                            action_descriptors.clone(),
                            window,
                            cx,
                        )
                    })
                    .into(),
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
                warm_token: 0,
                warm_expiry: None,
            },
        );
        if kind == SurfaceKind::QuickShell {
            self.chat.set_surface_visible(SurfaceKind::QuickShell, true);
            for action in self.all_text_actions() {
                action.set_surface_visible(true);
            }
        } else if kind == SurfaceKind::ChatPanel {
            self.chat.set_surface_visible(SurfaceKind::ChatPanel, true);
        }
        Ok(())
    }

    fn take_warm_quick_shell(&mut self, warm_token: u64) -> Option<WindowHandle<Root>> {
        let should_destroy = self
            .registry
            .entries
            .get(&SurfaceKind::QuickShell)
            .is_some_and(|entry| {
                entry.state == SurfaceState::HiddenWarm && entry.warm_token == warm_token
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

fn build_window_options(
    kind: SurfaceKind,
    display_id: Option<DisplayId>,
    cx: &App,
) -> WindowOptions {
    let (title, dimensions, minimum) = match kind {
        SurfaceKind::QuickShell => (
            "LexWisp · Quick Shell",
            size(px(720.), px(640.)),
            size(px(560.), px(480.)),
        ),
        SurfaceKind::ChatPanel => (
            "LexWisp · Chat",
            size(px(1120.), px(760.)),
            size(px(760.), px(560.)),
        ),
        SurfaceKind::ControlCenter => (
            "LexWisp · Settings",
            size(px(720.), px(680.)),
            size(px(620.), px(600.)),
        ),
    };
    let bounds = display_id
        .and_then(|id| cx.find_display(id))
        .or_else(|| cx.primary_display())
        .map(|display| centered_in_work_area(display.visible_bounds(), dimensions))
        .unwrap_or_else(|| Bounds::centered(display_id, dimensions, cx));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        display_id,
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            ..Default::default()
        }),
        window_min_size: Some(minimum),
        app_id: Some("org.lexwisp.LexWisp".into()),
        ..Default::default()
    }
}

fn centered_in_work_area(work_area: Bounds<Pixels>, dimensions: Size<Pixels>) -> Bounds<Pixels> {
    Bounds::centered_at(work_area.center(), dimensions)
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

#[cfg(test)]
mod tests {
    use gpui_kit::{bounds, point};

    use super::*;

    #[test]
    fn placement_preserves_negative_monitor_coordinates() {
        let work_area = bounds(point(px(-1920.), px(40.)), size(px(1920.), px(1040.)));
        let placed = centered_in_work_area(work_area, size(px(720.), px(640.)));
        assert_eq!(placed.origin, point(px(-1320.), px(240.)));
        assert_eq!(placed.size, size(px(720.), px(640.)));
    }

    #[test]
    fn warm_retention_does_not_advance_window_generation() {
        let mut registry = WindowRegistry::default();
        let first_window = registry.allocate_generation();
        for _ in 0..100 {
            registry.allocate_warm_token();
        }
        assert_eq!(registry.allocate_generation(), first_window + 1);
        assert_eq!(registry.next_warm_token, 100);
    }
}

use std::{collections::HashMap, rc::Rc, sync::Arc, time::Duration};

use gpui_kit::component::{Root, Theme, ThemeMode};
use gpui_kit::{
    AnyView, App, AppContext, Bounds, Context, DisplayId, Entity, IntoElement, ParentElement,
    Pixels, Render, Size, Styled, Subscription, Task, TitlebarOptions, WeakEntity, Window,
    WindowBounds, WindowHandle, WindowId, WindowOptions, div, point, px, size,
};
use lexwisp_core::{
    ActionDescriptor, CaptureStatus, ChatUiPort, ContextSnapshot, HistoryUiPort,
    PluginManagementUiPort, ProviderUiPort, SettingsUiPort, ShellPresentation, SurfaceKind,
    ThemePreference,
};

use crate::control_center::ControlCenter;

const WINDOW_MARGIN: f32 = 24.;
const WINDOW_TRANSITION_FRAME: Duration = Duration::from_millis(16);

pub trait SurfaceWindowPlatform {
    fn active_display_id(&self) -> Option<u64>;
    fn hide(&self, window: &Window) -> Result<(), String>;
    fn show(&self, window: &Window) -> Result<(), String>;
    fn set_bounds(&self, window: &Window, bounds: Bounds<Pixels>) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurfaceState {
    Visible,
    HiddenWarm,
}

struct WindowEntry {
    handle: WindowHandle<Root>,
    session: Option<Entity<ShellSession>>,
    state: SurfaceState,
    generation: u64,
    warm_token: u64,
    warm_expiry: Option<Task<()>>,
    bounds_transition: Option<Task<()>>,
}

#[derive(Default)]
struct LaunchContextState {
    current_generation: u64,
    pending: Option<(u64, ContextSnapshot)>,
}

enum BeginLaunch {
    Stale,
    Current(Option<ContextSnapshot>),
}

enum ReceiveLaunch {
    Deferred,
    Current(ContextSnapshot),
}

impl LaunchContextState {
    fn begin(&mut self, generation: u64) -> BeginLaunch {
        if generation < self.current_generation
            || self
                .pending
                .as_ref()
                .is_some_and(|(pending, _)| *pending > generation)
        {
            return BeginLaunch::Stale;
        }
        self.current_generation = generation;
        let snapshot = self
            .pending
            .take()
            .and_then(|(pending, snapshot)| (pending == generation).then_some(snapshot));
        BeginLaunch::Current(snapshot)
    }

    fn receive(&mut self, generation: u64, snapshot: ContextSnapshot) -> ReceiveLaunch {
        if generation < self.current_generation {
            return ReceiveLaunch::Deferred;
        }
        if generation > self.current_generation {
            if self
                .pending
                .as_ref()
                .is_none_or(|(pending, _)| *pending <= generation)
            {
                self.pending = Some((generation, snapshot));
            }
            return ReceiveLaunch::Deferred;
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending > generation)
        {
            ReceiveLaunch::Deferred
        } else {
            ReceiveLaunch::Current(snapshot)
        }
    }
}

pub struct ShellSession {
    presentation: ShellPresentation,
    launch_context: ContextSnapshot,
}

impl ShellSession {
    fn new() -> Self {
        Self {
            presentation: ShellPresentation::Compact,
            launch_context: ContextSnapshot::empty(
                CaptureStatus::NoSelection,
                "No selection attached",
            ),
        }
    }

    pub const fn presentation(&self) -> ShellPresentation {
        self.presentation
    }

    pub const fn launch_context(&self) -> &ContextSnapshot {
        &self.launch_context
    }

    fn set_presentation(&mut self, presentation: ShellPresentation, cx: &mut Context<Self>) {
        if self.presentation != presentation {
            self.presentation = presentation;
            cx.notify();
        }
    }

    fn set_launch_context(&mut self, snapshot: ContextSnapshot, cx: &mut Context<Self>) {
        if self.launch_context != snapshot {
            self.launch_context = snapshot;
            cx.notify();
        }
    }
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
    launch_contexts: LaunchContextState,
    action_descriptors: Vec<ActionDescriptor>,
    shell_content_factory: ShellContentViewFactory,
    registry: WindowRegistry,
}

pub struct SurfaceServices {
    settings: Arc<dyn SettingsUiPort>,
    providers: Arc<dyn ProviderUiPort>,
    chat: Arc<dyn ChatUiPort>,
    history: Arc<dyn HistoryUiPort>,
    plugin_management: Arc<dyn PluginManagementUiPort>,
    action_descriptors: Vec<ActionDescriptor>,
}

impl SurfaceServices {
    pub fn new(
        settings: Arc<dyn SettingsUiPort>,
        providers: Arc<dyn ProviderUiPort>,
        chat: Arc<dyn ChatUiPort>,
        history: Arc<dyn HistoryUiPort>,
        plugin_management: Arc<dyn PluginManagementUiPort>,
        action_descriptors: Vec<ActionDescriptor>,
    ) -> Self {
        Self {
            settings,
            providers,
            chat,
            history,
            plugin_management,
            action_descriptors,
        }
    }
}

pub type ShellContentViewFactory = Rc<
    dyn Fn(WeakEntity<SurfaceController>, Entity<ShellSession>, &mut Window, &mut App) -> AnyView,
>;
pub type SurfaceFactory = SurfaceController;

impl SurfaceController {
    pub fn new(
        platform: Rc<dyn SurfaceWindowPlatform>,
        services: SurfaceServices,
        shell_content_factory: ShellContentViewFactory,
    ) -> Self {
        Self {
            platform,
            settings: services.settings,
            providers: services.providers,
            chat: services.chat,
            history: services.history,
            plugin_management: services.plugin_management,
            launch_contexts: LaunchContextState::default(),
            action_descriptors: services.action_descriptors,
            shell_content_factory,
            registry: WindowRegistry::default(),
        }
    }

    pub fn show_main_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.prepare_main_shell(ShellPresentation::Compact, None, cx);
        self.show(SurfaceKind::MainShell, cx)
    }

    pub fn toggle_main_shell(
        &mut self,
        launch_generation: u64,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let pending = match self.launch_contexts.begin(launch_generation) {
            BeginLaunch::Stale => return Ok(()),
            BeginLaunch::Current(snapshot) => snapshot,
        };
        if self
            .registry
            .entries
            .get(&SurfaceKind::MainShell)
            .is_some_and(|entry| entry.state == SurfaceState::Visible)
        {
            return self.hide_main_shell(cx);
        }

        let context = pending.unwrap_or_else(|| {
            ContextSnapshot::empty(
                CaptureStatus::NoSelection,
                "Checking the foreground selection…",
            )
        });
        self.prepare_main_shell(ShellPresentation::Compact, Some(context), cx);
        self.show(SurfaceKind::MainShell, cx)
    }

    pub fn apply_launch_context(
        &mut self,
        launch_generation: u64,
        snapshot: ContextSnapshot,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let ReceiveLaunch::Current(snapshot) =
            self.launch_contexts.receive(launch_generation, snapshot)
        else {
            return Ok(());
        };
        let Some(session) = self
            .registry
            .entries
            .get(&SurfaceKind::MainShell)
            .filter(|entry| entry.state == SurfaceState::Visible)
            .and_then(|entry| entry.session.clone())
        else {
            return Ok(());
        };
        session.update(cx, |session, cx| session.set_launch_context(snapshot, cx));
        Ok(())
    }

    pub fn set_main_shell_presentation(
        &mut self,
        presentation: ShellPresentation,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        if !self.registry.entries.contains_key(&SurfaceKind::MainShell) {
            self.show(SurfaceKind::MainShell, cx)?;
        }
        let Some((handle, session)) = self
            .registry
            .entries
            .get(&SurfaceKind::MainShell)
            .and_then(|entry| entry.session.clone().map(|session| (entry.handle, session)))
        else {
            return Err(anyhow::anyhow!("Main Shell session is unavailable"));
        };
        session.update(cx, |session, cx| session.set_presentation(presentation, cx));
        let (start, target) = handle.update(cx, move |_, window, cx| {
            let target = presentation_bounds(presentation, window, cx);
            window.set_window_title(match presentation {
                ShellPresentation::Compact => "LexWisp",
                ShellPresentation::Expanded => "LexWisp · Conversation",
                ShellPresentation::Workspace => "LexWisp · Workspace",
            });
            (window.bounds(), target)
        })?;

        let steps = if presentation == ShellPresentation::Workspace {
            12
        } else {
            10
        };
        let platform = self.platform.clone();
        let transition = cx.spawn(async move |_, cx| {
            for step in 1..=steps {
                cx.background_executor()
                    .timer(WINDOW_TRANSITION_FRAME)
                    .await;
                let progress = step as f32 / steps as f32;
                let bounds = interpolate_bounds(start, target, ease_out_cubic(progress));
                if !matches!(
                    handle.update(cx, |_, window, _| platform.set_bounds(window, bounds)),
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
        });
        if let Some(entry) = self.registry.entries.get_mut(&SurfaceKind::MainShell) {
            entry.bounds_transition = Some(transition);
        }
        Ok(())
    }

    pub fn show_control_center(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        self.show(SurfaceKind::ControlCenter, cx)
    }

    pub fn refresh_plugins(&mut self, _: &mut Context<Self>) {
        // Main Shell hosts only the compiled Chat experience. Installed text/script plugins
        // remain available to Host and Control Center without rebuilding this window.
    }

    pub fn hide_main_shell_from_view(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let _ = self.hide_main_shell(cx);
    }

    fn prepare_main_shell(
        &mut self,
        presentation: ShellPresentation,
        context: Option<ContextSnapshot>,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .registry
            .entries
            .get(&SurfaceKind::MainShell)
            .and_then(|entry| entry.session.clone())
        else {
            return;
        };
        session.update(cx, |session, cx| {
            session.set_presentation(presentation, cx);
            if let Some(context) = context {
                session.set_launch_context(context, cx);
            }
        });
    }

    fn hide_main_shell(&mut self, cx: &mut Context<Self>) -> anyhow::Result<()> {
        let Some(handle) = self
            .registry
            .entries
            .get_mut(&SurfaceKind::MainShell)
            .map(|entry| {
                entry.bounds_transition = None;
                entry.handle
            })
        else {
            return Ok(());
        };
        let platform = self.platform.clone();
        handle.update(cx, move |_, window, _| {
            platform.hide(window).map_err(anyhow::Error::msg)
        })??;
        self.begin_warm_retention(cx);
        Ok(())
    }

    fn hide_main_shell_if_generation(
        &mut self,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.is_current_generation(SurfaceKind::MainShell, generation) {
            window.remove_window();
            return;
        }
        if let Some(entry) = self.registry.entries.get_mut(&SurfaceKind::MainShell) {
            entry.bounds_transition = None;
        }
        self.chat.set_surface_visible(SurfaceKind::MainShell, false);
        if self.platform.hide(window).is_err() {
            window.remove_window();
            self.registry.entries.remove(&SurfaceKind::MainShell);
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
        self.chat.set_surface_visible(SurfaceKind::MainShell, false);
        let warm_token = self.registry.allocate_warm_token();
        let Some(entry) = self.registry.entries.get_mut(&SurfaceKind::MainShell) else {
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
                    controller.take_warm_main_shell(warm_token)
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
            if kind == SurfaceKind::MainShell {
                self.chat.set_surface_visible(SurfaceKind::MainShell, false);
            }
        }
        self.registry
            .entries
            .retain(|_, entry| entry.handle.window_id() != id);
    }

    fn show(&mut self, kind: SurfaceKind, cx: &mut Context<Self>) -> anyhow::Result<()> {
        if let Some(entry) = self.registry.entries.get_mut(&kind) {
            entry.warm_expiry = None;
            entry.bounds_transition = None;
            entry.state = SurfaceState::Visible;
            let platform = self.platform.clone();
            let preference = self.settings.snapshot().settings().theme();
            let presentation = entry
                .session
                .as_ref()
                .map(|session| session.read(cx).presentation());
            let shown = entry.handle.update(cx, move |_, window, cx| {
                apply_theme(preference, window, cx);
                if let Some(presentation) = presentation {
                    platform
                        .set_bounds(window, presentation_bounds(presentation, window, cx))
                        .map_err(anyhow::Error::msg)?;
                }
                platform.show(window).map_err(anyhow::Error::msg)?;
                window.activate_window();
                Ok::<(), anyhow::Error>(())
            });
            if matches!(shown, Ok(Ok(()))) {
                if kind == SurfaceKind::MainShell {
                    self.chat.set_surface_visible(SurfaceKind::MainShell, true);
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
        let shell_content_factory = self.shell_content_factory.clone();
        let preference = settings.snapshot().settings().theme();
        let display_id = self.platform.active_display_id().map(DisplayId::new);
        let options = build_window_options(kind, display_id, cx);
        let created_session =
            (kind == SurfaceKind::MainShell).then(|| cx.new(|_| ShellSession::new()));
        let session_for_window = created_session.clone();
        let handle = cx.open_window(options, move |window, cx| {
            apply_theme(preference, window, cx);
            if kind == SurfaceKind::MainShell {
                let controller_for_close = controller.clone();
                window.on_window_should_close(cx, move |window, cx| {
                    let _ = controller_for_close.update(cx, |controller, cx| {
                        controller.hide_main_shell_if_generation(generation, window, cx);
                    });
                    false
                });
            }
            let content: AnyView = match kind {
                SurfaceKind::MainShell => shell_content_factory(
                    controller.clone(),
                    session_for_window
                        .clone()
                        .expect("Main Shell always has a session"),
                    window,
                    cx,
                ),
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
                session: created_session,
                state: SurfaceState::Visible,
                generation,
                warm_token: 0,
                warm_expiry: None,
                bounds_transition: None,
            },
        );
        if kind == SurfaceKind::MainShell {
            self.chat.set_surface_visible(SurfaceKind::MainShell, true);
        }
        Ok(())
    }

    fn take_warm_main_shell(&mut self, warm_token: u64) -> Option<WindowHandle<Root>> {
        let should_destroy = self
            .registry
            .entries
            .get(&SurfaceKind::MainShell)
            .is_some_and(|entry| {
                entry.state == SurfaceState::HiddenWarm && entry.warm_token == warm_token
            });
        if !should_destroy {
            return None;
        }
        if let Some(mut entry) = self.registry.entries.remove(&SurfaceKind::MainShell) {
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
        SurfaceKind::MainShell => (
            "LexWisp",
            presentation_size(ShellPresentation::Compact),
            size(px(560.), px(160.)),
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

fn presentation_size(presentation: ShellPresentation) -> Size<Pixels> {
    match presentation {
        ShellPresentation::Compact => size(px(680.), px(190.)),
        ShellPresentation::Expanded => size(px(680.), px(640.)),
        ShellPresentation::Workspace => size(px(1180.), px(780.)),
    }
}

fn presentation_bounds(
    presentation: ShellPresentation,
    window: &Window,
    cx: &App,
) -> Bounds<Pixels> {
    let current = window.bounds();
    let work_area = window
        .display(cx)
        .map(|display| display.visible_bounds())
        .unwrap_or(current);
    let desired = presentation_size(presentation);
    let width = desired
        .width
        .as_f32()
        .min((work_area.size.width.as_f32() - WINDOW_MARGIN * 2.).max(560.));
    let height = desired
        .height
        .as_f32()
        .min((work_area.size.height.as_f32() - WINDOW_MARGIN * 2.).max(160.));
    let work_left = work_area.origin.x.as_f32();
    let work_top = work_area.origin.y.as_f32();
    let work_right = work_left + work_area.size.width.as_f32();
    let work_bottom = work_top + work_area.size.height.as_f32();
    let current_center_x = current.origin.x.as_f32() + current.size.width.as_f32() / 2.;
    let current_center_y = current.origin.y.as_f32() + current.size.height.as_f32() / 2.;
    let preferred_x = if presentation == ShellPresentation::Workspace {
        current.origin.x.as_f32() + current.size.width.as_f32() - width
    } else {
        current_center_x - width / 2.
    };
    let preferred_y = current_center_y - height / 2.;
    let x = preferred_x.clamp(work_left, (work_right - width).max(work_left));
    let y = preferred_y.clamp(work_top, (work_bottom - height).max(work_top));
    Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
}

fn ease_out_cubic(progress: f32) -> f32 {
    let remaining = 1. - progress.clamp(0., 1.);
    1. - remaining * remaining * remaining
}

fn interpolate_bounds(
    start: Bounds<Pixels>,
    target: Bounds<Pixels>,
    progress: f32,
) -> Bounds<Pixels> {
    let progress = progress.clamp(0., 1.);
    let interpolate = |start: Pixels, target: Pixels| {
        px(start.as_f32() + (target.as_f32() - start.as_f32()) * progress)
    };
    Bounds::new(
        point(
            interpolate(start.origin.x, target.origin.x),
            interpolate(start.origin.y, target.origin.y),
        ),
        size(
            interpolate(start.size.width, target.size.width),
            interpolate(start.size.height, target.size.height),
        ),
    )
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
        let placed = centered_in_work_area(work_area, size(px(680.), px(190.)));
        assert_eq!(placed.origin, point(px(-1300.), px(465.)));
        assert_eq!(placed.size, size(px(680.), px(190.)));
    }

    #[test]
    fn window_transition_reaches_the_exact_target() {
        let start = bounds(point(px(100.), px(200.)), size(px(680.), px(190.)));
        let target = bounds(point(px(-200.), px(40.)), size(px(1180.), px(780.)));
        assert_eq!(interpolate_bounds(start, target, 0.), start);
        assert_eq!(
            interpolate_bounds(start, target, ease_out_cubic(1.)),
            target
        );
        let midway = interpolate_bounds(start, target, ease_out_cubic(0.5));
        assert!(midway.origin.x < start.origin.x);
        assert!(midway.size.width > start.size.width);
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

    fn context(detail: &str) -> ContextSnapshot {
        ContextSnapshot::empty(CaptureStatus::NoSelection, detail)
    }

    #[test]
    fn launch_context_can_arrive_before_its_surface_intent() {
        let mut state = LaunchContextState::default();
        assert!(matches!(
            state.receive(2, context("second")),
            ReceiveLaunch::Deferred
        ));
        assert!(matches!(state.begin(1), BeginLaunch::Stale));
        let BeginLaunch::Current(Some(snapshot)) = state.begin(2) else {
            panic!("matching launch should receive its deferred context");
        };
        assert_eq!(snapshot.detail, "second");
    }

    #[test]
    fn old_launch_context_cannot_replace_the_current_launch() {
        let mut state = LaunchContextState::default();
        assert!(matches!(state.begin(2), BeginLaunch::Current(None)));
        assert!(matches!(
            state.receive(1, context("old")),
            ReceiveLaunch::Deferred
        ));
        let ReceiveLaunch::Current(snapshot) = state.receive(2, context("current")) else {
            panic!("current launch context should be applied");
        };
        assert_eq!(snapshot.detail, "current");
    }
}

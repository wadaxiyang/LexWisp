#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{cell::RefCell, process::ExitCode, rc::Rc, sync::Arc};

use gpui_kit::{
    AppContext, AsyncApp, Entity, Global, QuitMode, Subscription, Task, WeakEntity, Window,
};
use lexwisp_core::{ChatUiPort, HostUiCommand};
use lexwisp_host::{ChatController, Host};
use lexwisp_platform_windows::{
    SingleInstance, SingleInstanceGuard, WindowsAtomicFileWriter, WindowsShell,
    display_id_under_cursor, hide_native_window, set_native_window_pinned, show_native_window,
    show_startup_error,
};
use lexwisp_storage::ConfigStore;
use lexwisp_ui::{SurfaceController, SurfaceServices, SurfaceWindowPlatform, register_shortcuts};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

struct NativeWindowPlatform;

impl SurfaceWindowPlatform for NativeWindowPlatform {
    fn active_display_id(&self) -> Option<u64> {
        display_id_under_cursor()
    }

    fn hide(&self, window: &Window) -> Result<(), String> {
        hide_native_window(native_handle(window)?)
    }

    fn show(&self, window: &Window) -> Result<(), String> {
        show_native_window(native_handle(window)?)
    }

    fn set_pinned(&self, window: &Window, pinned: bool) -> Result<(), String> {
        set_native_window_pinned(native_handle(window)?, pinned)
    }
}

fn native_handle(window: &Window) -> Result<isize, String> {
    let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get()),
        _ => Err("GPUI did not provide a Win32 window handle".into()),
    }
}

fn handle_ui_command(
    command: HostUiCommand,
    surfaces: &WeakEntity<SurfaceController>,
    cx: &mut AsyncApp,
) -> bool {
    let result = match command {
        HostUiCommand::ToggleMainShell => surfaces
            .update(cx, |surfaces, cx| surfaces.toggle_main_shell(cx))
            .and_then(|result| result)
            .map_err(|error| ("toggle Main Shell", error)),
        HostUiCommand::ShowMainShell => surfaces
            .update(cx, |surfaces, cx| surfaces.show_main_shell(cx))
            .and_then(|result| result)
            .map_err(|error| ("open Main Shell", error)),
        HostUiCommand::ShowSettings => surfaces
            .update(cx, |surfaces, cx| surfaces.show_settings(cx))
            .and_then(|result| result)
            .map_err(|error| ("open Settings", error)),
        HostUiCommand::ShowAbout => surfaces
            .update(cx, |surfaces, cx| surfaces.show_about(cx))
            .and_then(|result| result)
            .map_err(|error| ("open About", error)),
        HostUiCommand::Quit => {
            cx.update(|cx| cx.quit());
            return true;
        }
    };
    if let Err((operation, error)) = result {
        show_startup_error(&format!("Could not {operation}.\n\n{error:#}"));
    }
    false
}

struct RuntimeOwners {
    host: Option<Host>,
    shell: Option<WindowsShell>,
    _instance: SingleInstanceGuard,
}

impl RuntimeOwners {
    fn shutdown(&mut self) {
        if let Some(host) = self.host.take() {
            host.shutdown();
        }
        if let Some(shell) = self.shell.take() {
            shell.shutdown();
        }
    }
}

struct ApplicationLifetime {
    _surfaces: Entity<SurfaceController>,
    _window_closed: Subscription,
    _quit: Subscription,
    _command_bridge: Task<()>,
    _surface_intent_bridge: Task<()>,
}

impl Global for ApplicationLifetime {}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            show_startup_error(&format!("LexWisp could not start.\n\n{error}"));
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let instance = match SingleInstanceGuard::acquire()? {
        SingleInstance::First(instance) => instance,
        SingleInstance::Existing => return Ok(()),
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate LexWisp.exe: {error}"))?;
    let writer = Arc::new(WindowsAtomicFileWriter);
    let config = ConfigStore::discover(&executable, writer).map_err(|error| error.to_string())?;
    let loaded = config.load().map_err(|error| error.to_string())?;
    let initial_settings = loaded.settings().clone();
    let (ui_sender, ui_receiver) = async_channel::bounded(32);
    let mut shell = WindowsShell::start(initial_settings.hotkey(), ui_sender.clone())
        .map_err(|error| error.to_string())?;
    let surface_intents = shell.take_surface_intents();
    let initial_hotkey_error = shell.initial_hotkey_error().map(str::to_owned);
    let (host, handles) = Host::build(
        initial_settings,
        config,
        shell.handle(),
        ui_sender,
        executable,
    )?;
    let owners = Rc::new(RefCell::new(Some(RuntimeOwners {
        host: Some(host),
        shell: Some(shell),
        _instance: instance,
    })));
    let owners_for_app = owners.clone();
    let settings = handles.settings();
    let providers = handles.providers();
    let history = handles.history();
    let chat_controller = ChatController::new(handles.chat_run_port(), handles.chat_history())
        .map_err(|error| error.to_string())?;
    let chat: Arc<dyn ChatUiPort> = chat_controller.clone();
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            register_shortcuts(cx);
            cx.set_quit_mode(QuitMode::Explicit);
            let surfaces = cx.new(|_| {
                SurfaceController::new(
                    Rc::new(NativeWindowPlatform),
                    SurfaceServices {
                        settings: settings.clone(),
                        providers: providers.clone(),
                        chat: chat.clone(),
                        history: history.clone(),
                    },
                )
            });
            let surface_for_commands = surfaces.downgrade();
            let command_bridge = cx.spawn(async move |cx| {
                while let Ok(command) = ui_receiver.recv().await {
                    if handle_ui_command(command, &surface_for_commands, cx) {
                        break;
                    }
                }
            });
            let surface_for_intents = surfaces.downgrade();
            let surface_intent_bridge = cx.spawn(async move |cx| {
                while let Ok(command) = surface_intents.recv().await {
                    if handle_ui_command(command, &surface_for_intents, cx) {
                        break;
                    }
                }
            });
            let surface_for_closed = surfaces.downgrade();
            let window_closed = cx.on_window_closed(move |cx, id| {
                let _ = surface_for_closed.update(cx, |surfaces, _| surfaces.window_closed(id));
            });
            let owners_for_quit = owners_for_app.clone();
            let quit = cx.on_app_quit(move |_| {
                if let Some(mut owners) = owners_for_quit.borrow_mut().take() {
                    owners.shutdown();
                }
                std::future::ready(())
            });
            cx.set_global(ApplicationLifetime {
                _surfaces: surfaces.clone(),
                _window_closed: window_closed,
                _quit: quit,
                _command_bridge: command_bridge,
                _surface_intent_bridge: surface_intent_bridge,
            });

            if let Some(error) = &initial_hotkey_error {
                show_startup_error(&format!(
                    "The configured global shortcut could not be registered. Open Settings from the notification-area menu to choose another shortcut.\n\n{error}"
                ));
            }
        });

    if let Some(mut owners) = owners.borrow_mut().take() {
        owners.shutdown();
    }
    Ok(())
}

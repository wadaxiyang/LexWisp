#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{cell::RefCell, process::ExitCode, rc::Rc, sync::Arc};

use gpui_kit::{AppContext, Entity, Global, QuitMode, Subscription, Task, Window};
use lexwisp_core::HostUiCommand;
use lexwisp_host::Host;
use lexwisp_platform_windows::{
    SingleInstance, SingleInstanceGuard, WindowsAtomicFileWriter, WindowsShell, hide_native_window,
    show_native_window, show_startup_error,
};
use lexwisp_storage::ConfigStore;
use lexwisp_ui::{SurfaceController, SurfaceWindowPlatform};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

struct NativeWindowPlatform;

impl SurfaceWindowPlatform for NativeWindowPlatform {
    fn hide(&self, window: &Window) -> Result<(), String> {
        hide_native_window(native_handle(window)?)
    }

    fn show(&self, window: &Window) -> Result<(), String> {
        show_native_window(native_handle(window)?)
    }
}

fn native_handle(window: &Window) -> Result<isize, String> {
    let handle = HasWindowHandle::window_handle(window).map_err(|error| error.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => Ok(handle.hwnd.get()),
        _ => Err("GPUI did not provide a Win32 window handle".into()),
    }
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
        SingleInstance::ExistingNotified => return Ok(()),
    };
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate LexWisp.exe: {error}"))?;
    let writer = Arc::new(WindowsAtomicFileWriter);
    let config = ConfigStore::discover(&executable, writer).map_err(|error| error.to_string())?;
    let loaded = config.load().map_err(|error| error.to_string())?;
    let initial_settings = loaded.settings().clone();
    let first_run = loaded.first_run();
    let (ui_sender, ui_receiver) = async_channel::bounded(32);
    let shell = WindowsShell::start(initial_settings.hotkey(), ui_sender.clone())
        .map_err(|error| error.to_string())?;
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

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_quit_mode(QuitMode::Explicit);
            let surfaces = cx.new(|_| {
                SurfaceController::new(Rc::new(NativeWindowPlatform), settings.clone())
            });
            let surface_for_commands = surfaces.downgrade();
            let command_bridge = cx.spawn(async move |cx| {
                while let Ok(command) = ui_receiver.recv().await {
                    match command {
                        HostUiCommand::ToggleQuickShell => {
                            let result = surface_for_commands
                                .update(cx, |surfaces, cx| surfaces.toggle_quick_shell(cx))
                                .and_then(|result| result);
                            if let Err(error) = result {
                                show_startup_error(&format!(
                                    "Could not toggle Quick Shell.\n\n{error:#}"
                                ));
                            }
                        }
                        HostUiCommand::ShowQuickShell => {
                            let result = surface_for_commands
                                .update(cx, |surfaces, cx| surfaces.show_quick_shell(cx))
                                .and_then(|result| result);
                            if let Err(error) = result {
                                show_startup_error(&format!(
                                    "Could not open Quick Shell.\n\n{error:#}"
                                ));
                            }
                        }
                        HostUiCommand::ShowControlCenter => {
                            let result = surface_for_commands
                                .update(cx, |surfaces, cx| surfaces.show_control_center(cx))
                                .and_then(|result| result);
                            if let Err(error) = result {
                                show_startup_error(&format!(
                                    "Could not open Control Center.\n\n{error:#}"
                                ));
                            }
                        }
                        HostUiCommand::Quit => {
                            cx.update(|cx| cx.quit());
                            break;
                        }
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
            });

            if (first_run || initial_hotkey_error.is_some())
                && let Err(error) =
                    surfaces.update(cx, |surfaces, cx| surfaces.show_control_center(cx))
            {
                show_startup_error(&format!("Could not open Control Center.\n\n{error:#}"));
                cx.quit();
                return;
            }
            if let Some(error) = &initial_hotkey_error {
                show_startup_error(&format!(
                    "The configured global shortcut could not be registered. Choose another shortcut in Settings.\n\n{error}"
                ));
            }
        });

    if let Some(mut owners) = owners.borrow_mut().take() {
        owners.shutdown();
    }
    Ok(())
}

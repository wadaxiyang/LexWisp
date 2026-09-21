use std::{
    ffi::OsStr,
    mem,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, mpsc},
    thread,
};

use async_channel::{Receiver, Sender, TrySendError};
use lexwisp_core::{GlobalHotkey, HostUiCommand};
use thiserror::Error;
use tokio::sync::oneshot;
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND, LPARAM, LRESULT, POINT, WPARAM},
    System::{
        LibraryLoader::GetModuleHandleW,
        Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegDeleteValueW, RegSetValueExW,
        },
    },
    UI::{
        Input::KeyboardAndMouse::{
            MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey,
            VK_SPACE,
        },
        Shell::{
            NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
        },
        WindowsAndMessaging::{
            AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
            DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetCursorPos, GetMessageW,
            GetWindowLongPtrW, HWND_MESSAGE, IDI_APPLICATION, LoadIconW, MF_SEPARATOR, MF_STRING,
            MSG, PostMessageW, PostQuitMessage, RegisterClassW, RegisterWindowMessageW,
            SetForegroundWindow, SetWindowLongPtrW, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
            TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP,
            WM_RBUTTONUP, WNDCLASSW,
        },
    },
};

use crate::WindowsContextHandle;

pub const MESSAGE_WINDOW_CLASS: &str = "LexWisp.MessageWindow.v1";
const WM_TRAY: u32 = WM_APP + 1;
const WM_COMMAND_QUEUE: u32 = WM_APP + 2;
pub const WM_LEXWISP_WAKE: u32 = WM_APP + 3;
const HOTKEY_PRIMARY: i32 = 1;
const HOTKEY_REPLACEMENT: i32 = 2;
const TRAY_ID: u32 = 1;
const MENU_MAIN_SHELL: usize = 100;
const MENU_SETTINGS: usize = 102;
const MENU_EXIT: usize = 103;
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "LexWisp";

#[derive(Debug, Error)]
pub enum PlatformError {
    #[error("Windows shell is unavailable: {0}")]
    Unavailable(String),
    #[error("global shortcut is unavailable: {0}")]
    Hotkey(String),
    #[error("startup registration failed: {0}")]
    Startup(String),
}

enum ThreadCommand {
    ReplaceHotkey {
        hotkey: GlobalHotkey,
        reply: oneshot::Sender<Result<(), PlatformError>>,
    },
    SetStartup {
        enabled: bool,
        executable: PathBuf,
        reply: oneshot::Sender<Result<(), PlatformError>>,
    },
    Shutdown {
        stopped: mpsc::SyncSender<()>,
    },
}

struct ThreadState {
    ui_commands: Sender<HostUiCommand>,
    surface_intents: LatestSurfaceIntent,
    context: WindowsContextHandle,
    commands: mpsc::Receiver<ThreadCommand>,
    launch_generation: u64,
    hotkey_id: Option<i32>,
    hotkey: Option<GlobalHotkey>,
    taskbar_created: u32,
    tray: NOTIFYICONDATAW,
}

struct LatestSurfaceIntent {
    sender: Sender<HostUiCommand>,
    stale_receiver: Receiver<HostUiCommand>,
}

impl LatestSurfaceIntent {
    fn send(&self, intent: HostUiCommand) -> bool {
        if self.sender.receiver_count() <= 1 {
            return false;
        }
        match self.sender.try_send(intent) {
            Ok(()) => true,
            Err(TrySendError::Closed(_)) => false,
            Err(TrySendError::Full(intent)) => {
                let _ = self.stale_receiver.try_recv();
                !matches!(self.sender.try_send(intent), Err(TrySendError::Closed(_)))
            }
        }
    }
}

struct ShellInner {
    window: isize,
    commands: mpsc::Sender<ThreadCommand>,
}

#[derive(Clone)]
pub struct WindowsShellHandle {
    inner: Arc<ShellInner>,
}

impl WindowsShellHandle {
    pub async fn replace_hotkey(&self, hotkey: GlobalHotkey) -> Result<(), PlatformError> {
        let (reply, response) = oneshot::channel();
        self.send(ThreadCommand::ReplaceHotkey { hotkey, reply })?;
        response.await.map_err(|_| {
            PlatformError::Unavailable("the Windows shell stopped before replying".into())
        })?
    }

    pub async fn set_startup(
        &self,
        enabled: bool,
        executable: PathBuf,
    ) -> Result<(), PlatformError> {
        let (reply, response) = oneshot::channel();
        self.send(ThreadCommand::SetStartup {
            enabled,
            executable,
            reply,
        })?;
        response.await.map_err(|_| {
            PlatformError::Unavailable("the Windows shell stopped before replying".into())
        })?
    }

    fn send(&self, command: ThreadCommand) -> Result<(), PlatformError> {
        self.inner
            .commands
            .send(command)
            .map_err(|_| PlatformError::Unavailable("the Windows shell has stopped".into()))?;
        // SAFETY: the message has no borrowed payload and the window belongs to the shell thread.
        if unsafe_post(self.inner.window, WM_COMMAND_QUEUE) == 0 {
            return Err(PlatformError::Unavailable(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        Ok(())
    }
}

pub struct WindowsShell {
    handle: WindowsShellHandle,
    thread: Option<thread::JoinHandle<()>>,
    initial_hotkey_error: Option<String>,
    surface_intents: Option<Receiver<HostUiCommand>>,
}

impl WindowsShell {
    pub fn start(
        hotkey: GlobalHotkey,
        ui_commands: Sender<HostUiCommand>,
        context: WindowsContextHandle,
    ) -> Result<Self, PlatformError> {
        let (commands_tx, commands_rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (surface_sender, surface_receiver) = async_channel::bounded(1);
        let stale_surface_receiver = surface_receiver.clone();
        let thread = thread::Builder::new()
            .name("lexwisp-windows-shell".into())
            .spawn(move || {
                shell_thread(
                    hotkey,
                    ui_commands,
                    surface_sender,
                    stale_surface_receiver,
                    context,
                    commands_rx,
                    ready_tx,
                )
            })
            .map_err(|error| PlatformError::Unavailable(error.to_string()))?;
        let ready = ready_rx.recv().map_err(|_| {
            PlatformError::Unavailable("the Windows shell exited during startup".into())
        })?;
        let (window, initial_hotkey_error) = match ready {
            Ok(value) => value,
            Err(error) => {
                let _ = thread.join();
                return Err(error);
            }
        };
        let handle = WindowsShellHandle {
            inner: Arc::new(ShellInner {
                window,
                commands: commands_tx,
            }),
        };
        Ok(Self {
            handle,
            thread: Some(thread),
            initial_hotkey_error,
            surface_intents: Some(surface_receiver),
        })
    }

    pub fn take_surface_intents(&mut self) -> Receiver<HostUiCommand> {
        self.surface_intents
            .take()
            .expect("surface intent receiver can only be taken once")
    }

    pub fn handle(&self) -> WindowsShellHandle {
        self.handle.clone()
    }

    pub fn initial_hotkey_error(&self) -> Option<&str> {
        self.initial_hotkey_error.as_deref()
    }

    pub fn shutdown(mut self) {
        let (stopped_tx, stopped_rx) = mpsc::sync_channel(1);
        let sent = self
            .handle
            .send(ThreadCommand::Shutdown {
                stopped: stopped_tx,
            })
            .is_ok();
        let stopped = sent
            && stopped_rx
                .recv_timeout(std::time::Duration::from_secs(2))
                .is_ok();
        if stopped && let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn hotkey_modifiers(hotkey: GlobalHotkey) -> u32 {
    let modifiers = match hotkey {
        GlobalHotkey::ControlAltSpace => MOD_CONTROL | MOD_ALT,
        GlobalHotkey::ControlShiftSpace => MOD_CONTROL | MOD_SHIFT,
        GlobalHotkey::AltShiftSpace => MOD_ALT | MOD_SHIFT,
    };
    modifiers | MOD_NOREPEAT
}

#[allow(unsafe_code)]
fn unsafe_post(window: isize, message: u32) -> i32 {
    // SAFETY: callers pass the owned message-window handle and messages carry no pointers.
    unsafe { PostMessageW(window as HWND, message, 0, 0) }
}

#[allow(unsafe_code)]
fn shell_thread(
    hotkey: GlobalHotkey,
    ui_commands: Sender<HostUiCommand>,
    surface_sender: Sender<HostUiCommand>,
    stale_surface_receiver: Receiver<HostUiCommand>,
    context: WindowsContextHandle,
    commands: mpsc::Receiver<ThreadCommand>,
    ready: mpsc::SyncSender<Result<(isize, Option<String>), PlatformError>>,
) {
    // SAFETY: this thread exclusively owns the window class, message window, tray icon, and
    // ThreadState pointer for their complete Win32 lifetime.
    unsafe {
        let class_name = wide(MESSAGE_WINDOW_CLASS);
        let instance = GetModuleHandleW(ptr::null());
        let class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance,
            hIcon: ptr::null_mut(),
            hCursor: ptr::null_mut(),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&class) == 0 {
            let _ = ready.send(Err(PlatformError::Unavailable(
                std::io::Error::last_os_error().to_string(),
            )));
            return;
        }
        let window = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            instance,
            ptr::null(),
        );
        if window.is_null() {
            let _ = ready.send(Err(PlatformError::Unavailable(
                std::io::Error::last_os_error().to_string(),
            )));
            return;
        }

        let mut tray = NOTIFYICONDATAW {
            cbSize: mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: window,
            uID: TRAY_ID,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: WM_TRAY,
            hIcon: LoadIconW(ptr::null_mut(), IDI_APPLICATION),
            ..Default::default()
        };
        let tip = wide("LexWisp");
        let tip_length = tip.len().min(tray.szTip.len());
        tray.szTip[..tip_length].copy_from_slice(&tip[..tip_length]);
        if Shell_NotifyIconW(NIM_ADD, &tray) == 0 {
            DestroyWindow(window);
            let _ = ready.send(Err(PlatformError::Unavailable(format!(
                "could not create the notification icon: {}",
                std::io::Error::last_os_error()
            ))));
            return;
        }

        let initial_hotkey_error = if RegisterHotKey(
            window,
            HOTKEY_PRIMARY,
            hotkey_modifiers(hotkey),
            VK_SPACE as u32,
        ) == 0
        {
            Some(std::io::Error::last_os_error().to_string())
        } else {
            None
        };
        let taskbar_created = RegisterWindowMessageW(wide("TaskbarCreated").as_ptr());
        let mut state = Box::new(ThreadState {
            ui_commands,
            surface_intents: LatestSurfaceIntent {
                sender: surface_sender,
                stale_receiver: stale_surface_receiver,
            },
            context,
            commands,
            launch_generation: 0,
            hotkey_id: initial_hotkey_error.is_none().then_some(HOTKEY_PRIMARY),
            hotkey: initial_hotkey_error.is_none().then_some(hotkey),
            taskbar_created,
            tray,
        });
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&mut *state as *mut ThreadState) as isize,
        );
        let _ = ready.send(Ok((window as isize, initial_hotkey_error)));

        let mut message = MSG::default();
        while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        if let Some(id) = state.hotkey_id {
            UnregisterHotKey(window, id);
        }
        Shell_NotifyIconW(NIM_DELETE, &state.tray);
        drop(state);
    }
}

#[allow(unsafe_code)]
unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: GWLP_USERDATA is set to the thread-owned state before messages are dispatched and
    // cleared before it is dropped. The window procedure runs only on that owning thread.
    let state = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) as *mut ThreadState };
    if !state.is_null() {
        // SAFETY: justified above; no alias escapes this callback.
        let state = unsafe { &mut *state };
        if message == state.taskbar_created {
            // SAFETY: the tray structure belongs to this thread and remains alive for the call.
            unsafe { Shell_NotifyIconW(NIM_ADD, &state.tray) };
            return 0;
        }
        match message {
            WM_HOTKEY => {
                let prepared = state.context.prepare_foreground_capture();
                state.launch_generation = state.launch_generation.saturating_add(1);
                let launch_generation = state.launch_generation;
                state
                    .surface_intents
                    .send(HostUiCommand::ToggleMainShell { launch_generation });
                state.context.capture_for_launch(
                    prepared,
                    launch_generation,
                    state.ui_commands.clone(),
                );
                return 0;
            }
            WM_LEXWISP_WAKE => {
                state.surface_intents.send(HostUiCommand::ShowMainShell);
                return 0;
            }
            WM_COMMAND_QUEUE => {
                while let Ok(command) = state.commands.try_recv() {
                    if handle_command(window, state, command) {
                        return 0;
                    }
                }
                return 0;
            }
            WM_TRAY => {
                match lparam as u32 {
                    WM_LBUTTONUP => {
                        state.surface_intents.send(HostUiCommand::ShowMainShell);
                    }
                    WM_RBUTTONUP => show_tray_menu(window, state),
                    _ => {}
                }
                return 0;
            }
            WM_DESTROY => {
                // SAFETY: this terminates only the owning shell thread's message loop.
                unsafe { PostQuitMessage(0) };
                return 0;
            }
            _ => {}
        }
    }
    // SAFETY: unhandled messages are forwarded with their original values.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

#[allow(unsafe_code)]
fn handle_command(window: HWND, state: &mut ThreadState, command: ThreadCommand) -> bool {
    match command {
        ThreadCommand::ReplaceHotkey { hotkey, reply } => {
            let result = replace_hotkey(window, state, hotkey);
            let _ = reply.send(result);
            false
        }
        ThreadCommand::SetStartup {
            enabled,
            executable,
            reply,
        } => {
            let _ = reply.send(set_startup(enabled, &executable));
            false
        }
        ThreadCommand::Shutdown { stopped } => {
            // SAFETY: this destroys the message window on its owning thread.
            unsafe { DestroyWindow(window) };
            let _ = stopped.send(());
            true
        }
    }
}

#[allow(unsafe_code)]
fn replace_hotkey(
    window: HWND,
    state: &mut ThreadState,
    hotkey: GlobalHotkey,
) -> Result<(), PlatformError> {
    if state.hotkey == Some(hotkey) {
        return Ok(());
    }
    let replacement_id = match state.hotkey_id {
        Some(HOTKEY_PRIMARY) => HOTKEY_REPLACEMENT,
        _ => HOTKEY_PRIMARY,
    };
    // SAFETY: registration is bound to the shell's live message window; the ID is unused.
    if unsafe {
        RegisterHotKey(
            window,
            replacement_id,
            hotkey_modifiers(hotkey),
            VK_SPACE as u32,
        )
    } == 0
    {
        return Err(PlatformError::Hotkey(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    if let Some(previous_id) = state.hotkey_id {
        // SAFETY: the previous ID is registered to this window and released only after success.
        unsafe { UnregisterHotKey(window, previous_id) };
    }
    state.hotkey_id = Some(replacement_id);
    state.hotkey = Some(hotkey);
    Ok(())
}

#[allow(unsafe_code)]
fn show_tray_menu(window: HWND, state: &ThreadState) {
    // SAFETY: the popup menu and strings live through TrackPopupMenu and are destroyed afterward.
    unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        let main_shell = wide("Open LexWisp");
        let settings = wide("Settings");
        let exit = wide("Exit LexWisp");
        AppendMenuW(menu, MF_STRING, MENU_MAIN_SHELL, main_shell.as_ptr());
        AppendMenuW(menu, MF_STRING, MENU_SETTINGS, settings.as_ptr());
        AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
        AppendMenuW(menu, MF_STRING, MENU_EXIT, exit.as_ptr());
        let mut point = POINT::default();
        GetCursorPos(&mut point);
        SetForegroundWindow(window);
        let selected = TrackPopupMenu(
            menu,
            TPM_LEFTALIGN | TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            window,
            ptr::null(),
        ) as usize;
        DestroyMenu(menu);
        match selected {
            MENU_MAIN_SHELL => {
                state.surface_intents.send(HostUiCommand::ShowMainShell);
            }
            MENU_SETTINGS => {
                let _ = state.ui_commands.try_send(HostUiCommand::ShowControlCenter);
            }
            MENU_EXIT => {
                let _ = state.ui_commands.try_send(HostUiCommand::Quit);
            }
            _ => {}
        }
    }
}

#[allow(unsafe_code)]
fn set_startup(enabled: bool, executable: &Path) -> Result<(), PlatformError> {
    let key_path = wide(RUN_KEY);
    let value_name = wide(RUN_VALUE);
    let mut key: HKEY = ptr::null_mut();
    // SAFETY: output key storage and NUL-terminated strings remain alive for the calls below.
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            key_path.as_ptr(),
            0,
            ptr::null_mut(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(PlatformError::Startup(
            std::io::Error::from_raw_os_error(status as i32).to_string(),
        ));
    }
    let result = if enabled {
        let command = wide(format!("\"{}\"", executable.display()));
        let bytes = command.len().saturating_mul(mem::size_of::<u16>());
        let byte_count = u32::try_from(bytes)
            .map_err(|_| PlatformError::Startup("the executable path is too long".into()))?;
        // SAFETY: the key is open for set-value and the UTF-16 buffer length includes its NUL.
        unsafe {
            RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                byte_count,
            )
        }
    } else {
        // SAFETY: the key is open for set-value and the value name is NUL-terminated.
        unsafe { RegDeleteValueW(key, value_name.as_ptr()) }
    };
    // SAFETY: the registry key is owned by this function and closed exactly once.
    unsafe { RegCloseKey(key) };
    if result != ERROR_SUCCESS && (enabled || result != ERROR_FILE_NOT_FOUND) {
        return Err(PlatformError::Startup(
            std::io::Error::from_raw_os_error(result as i32).to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ThreadHotkey(i32);

    impl Drop for ThreadHotkey {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: this ID was registered with a null HWND on the current test thread.
            unsafe { UnregisterHotKey(ptr::null_mut(), self.0) };
        }
    }

    #[test]
    fn full_surface_intent_queue_keeps_the_latest_user_intent() {
        let (sender, receiver) = async_channel::bounded(1);
        let intents = LatestSurfaceIntent {
            sender,
            stale_receiver: receiver.clone(),
        };
        assert!(intents.send(HostUiCommand::ShowMainShell));
        assert!(intents.send(HostUiCommand::SetMainShellPresentation(
            lexwisp_core::ShellPresentation::Workspace,
        )));
        assert_eq!(
            receiver.try_recv().expect("latest intent remains queued"),
            HostUiCommand::SetMainShellPresentation(lexwisp_core::ShellPresentation::Workspace,)
        );
    }

    #[test]
    fn surface_intent_port_detects_a_dropped_consumer() {
        let (sender, receiver) = async_channel::bounded(1);
        let intents = LatestSurfaceIntent {
            sender,
            stale_receiver: receiver.clone(),
        };
        drop(receiver);
        assert!(!intents.send(HostUiCommand::ShowMainShell));
    }

    #[test]
    #[ignore = "uses real global shortcuts and the Windows notification area"]
    #[allow(unsafe_code)]
    fn replacement_conflict_preserves_the_previous_hotkey() {
        const CONFLICT_ID: i32 = 91;
        const PROBE_ID: i32 = 92;
        // SAFETY: the null HWND binds the hotkey to this test thread; the guard unregisters it.
        let registered = unsafe {
            RegisterHotKey(
                ptr::null_mut(),
                CONFLICT_ID,
                MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT,
                VK_SPACE as u32,
            )
        };
        assert_ne!(registered, 0, "the conflict fixture hotkey must be free");
        let _conflict = ThreadHotkey(CONFLICT_ID);

        let (ui_commands, _receiver) = async_channel::bounded(4);
        let context = crate::WindowsContextService::start().expect("context worker should start");
        let shell =
            WindowsShell::start(GlobalHotkey::ControlAltSpace, ui_commands, context.handle())
                .expect("the Windows shell should start");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("the fixture runtime should start");
        let result = runtime.block_on(
            shell
                .handle()
                .replace_hotkey(GlobalHotkey::ControlShiftSpace),
        );
        assert!(matches!(result, Err(PlatformError::Hotkey(_))));

        // SAFETY: attempting a second registration is the Windows-level proof that the shell kept
        // the previous shortcut. A surprising success is immediately undone before failing.
        let previous_was_released = unsafe {
            RegisterHotKey(
                ptr::null_mut(),
                PROBE_ID,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                VK_SPACE as u32,
            )
        } != 0;
        if previous_was_released {
            // SAFETY: this branch owns PROBE_ID because its registration just succeeded.
            unsafe { UnregisterHotKey(ptr::null_mut(), PROBE_ID) };
        }
        shell.shutdown();
        context.shutdown();
        assert!(
            !previous_was_released,
            "the old shortcut must remain active"
        );
    }
}

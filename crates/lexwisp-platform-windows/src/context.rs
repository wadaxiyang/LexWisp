#![allow(unsafe_code)]

use std::{
    collections::HashMap,
    mem, ptr,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use lexwisp_core::{
    CaptureStatus, ContextError, ContextFuture, ContextSnapshot, ContextToken, ContextUiPort,
    ReplaceOutcome,
};
use windows::Win32::{
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
        UIA_TextPatternId,
    },
};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
            GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
            SetClipboardData,
        },
        Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock},
        Threading::GetCurrentProcessId,
    },
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            SendInput, VK_CONTROL, VK_MENU, VK_SHIFT,
        },
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
            IsWindow, SetForegroundWindow,
        },
    },
};

const CAPTURE_TIMEOUT: Duration = Duration::from_millis(800);
const COPY_WAIT: Duration = Duration::from_millis(300);
const TOKEN_LIFETIME: Duration = Duration::from_secs(60);
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_PALETTE: u32 = 9;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;

#[derive(Clone)]
struct ForegroundTarget {
    window: isize,
    process_id: u32,
    title: String,
}

struct TargetRecord {
    target: ForegroundTarget,
    element: IUIAutomationElement,
    selected_text: String,
    created: Instant,
}

enum WorkerCommand {
    Capture {
        target: ForegroundTarget,
        reply: mpsc::SyncSender<Result<ContextSnapshot, ContextError>>,
    },
    Replace {
        token: ContextToken,
        text: String,
        reply: mpsc::SyncSender<Result<ReplaceOutcome, ContextError>>,
    },
    Shutdown,
}

#[derive(Clone)]
pub struct WindowsContextHandle {
    sender: mpsc::Sender<WorkerCommand>,
    latest: Arc<RwLock<ContextSnapshot>>,
    capture_paused: Arc<AtomicBool>,
}

impl WindowsContextHandle {
    pub fn capture_foreground_blocking(&self) -> ContextSnapshot {
        if self.capture_paused.load(Ordering::Acquire) {
            return ContextSnapshot::empty(
                CaptureStatus::TimedOut,
                "UI Automation capture is paused after a blocked cross-process call. Restart LexWisp to retry; manual input remains available.",
            );
        }
        let target = foreground_target();
        let snapshot = match target {
            Some(target) => self.request_capture(target),
            None => ContextSnapshot::empty(
                CaptureStatus::Unsupported,
                "Windows did not report a foreground target.",
            ),
        };
        *self
            .latest
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = snapshot.clone();
        snapshot
    }

    fn request_capture(&self, target: ForegroundTarget) -> ContextSnapshot {
        let (reply, response) = mpsc::sync_channel(1);
        if self
            .sender
            .send(WorkerCommand::Capture { target, reply })
            .is_err()
        {
            return ContextSnapshot::empty(
                CaptureStatus::Unsupported,
                "The Windows context worker is unavailable.",
            );
        }
        match response.recv_timeout(CAPTURE_TIMEOUT) {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => ContextSnapshot::empty(CaptureStatus::Unsupported, error.to_string()),
            Err(_) => ContextSnapshot::empty(CaptureStatus::TimedOut, {
                self.capture_paused.store(true, Ordering::Release);
                "Selection capture exceeded 800 ms; further UI Automation capture is paused until restart."
            }),
        }
    }

    fn copy_blocking(&self, text: String) -> Result<(), ContextError> {
        write_unicode_clipboard(&text)
    }

    fn read_clipboard_blocking(&self) -> Result<String, ContextError> {
        read_unicode_clipboard()?
            .ok_or_else(|| ContextError::Failed("the clipboard does not contain text".into()))
    }

    fn replace_blocking(
        &self,
        token: ContextToken,
        text: String,
    ) -> Result<ReplaceOutcome, ContextError> {
        if self.capture_paused.load(Ordering::Acquire) {
            write_unicode_clipboard(&text)?;
            return Err(ContextError::TargetChanged);
        }
        let (reply, response) = mpsc::sync_channel(1);
        self.sender
            .send(WorkerCommand::Replace {
                token,
                text: text.clone(),
                reply,
            })
            .map_err(|_| ContextError::Failed("the Windows context worker stopped".into()))?;
        match response.recv_timeout(Duration::from_secs(2)) {
            Ok(result) => result,
            Err(_) => {
                self.capture_paused.store(true, Ordering::Release);
                write_unicode_clipboard(&text)?;
                Err(ContextError::TargetChanged)
            }
        }
    }
}

impl ContextUiPort for WindowsContextHandle {
    fn latest(&self) -> ContextSnapshot {
        self.latest
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn capture(&self) -> ContextFuture<'_, ContextSnapshot> {
        let this = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || this.capture_foreground_blocking())
                .await
                .map_err(|error| ContextError::Failed(error.to_string()))
        })
    }

    fn copy(&self, text: String) -> ContextFuture<'_, ()> {
        let this = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || this.copy_blocking(text))
                .await
                .map_err(|error| ContextError::Failed(error.to_string()))?
        })
    }

    fn read_clipboard(&self) -> ContextFuture<'_, String> {
        let this = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || this.read_clipboard_blocking())
                .await
                .map_err(|error| ContextError::Failed(error.to_string()))?
        })
    }

    fn replace(&self, token: ContextToken, text: String) -> ContextFuture<'_, ReplaceOutcome> {
        let this = self.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || this.replace_blocking(token, text))
                .await
                .map_err(|error| ContextError::Failed(error.to_string()))?
        })
    }
}

pub struct WindowsContextService {
    handle: WindowsContextHandle,
    worker: Option<thread::JoinHandle<()>>,
}

impl WindowsContextService {
    pub fn start() -> Result<Self, ContextError> {
        let (sender, receiver) = mpsc::channel();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("lexwisp-uia-mta".into())
            .spawn(move || worker_main(receiver, ready_sender))
            .map_err(|error| ContextError::Failed(error.to_string()))?;
        ready_receiver
            .recv()
            .map_err(|_| ContextError::Failed("UI Automation worker exited".into()))??;
        Ok(Self {
            handle: WindowsContextHandle {
                sender,
                latest: Arc::new(RwLock::new(ContextSnapshot::empty(
                    CaptureStatus::NoSelection,
                    "Use the global shortcut after selecting text, or type below.",
                ))),
                capture_paused: Arc::new(AtomicBool::new(false)),
            },
            worker: Some(worker),
        })
    }

    pub fn handle(&self) -> WindowsContextHandle {
        self.handle.clone()
    }

    pub fn shutdown(mut self) {
        let _ = self.handle.sender.send(WorkerCommand::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn worker_main(
    receiver: mpsc::Receiver<WorkerCommand>,
    ready: mpsc::SyncSender<Result<(), ContextError>>,
) {
    // SAFETY: COM is initialized and uninitialized on this dedicated MTA thread; every UIA
    // interface is created, retained, used, and released on this same thread.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        let _ = ready.send(Err(ContextError::Failed(format!(
            "could not initialize COM: {initialized:?}"
        ))));
        return;
    }
    // SAFETY: COM is initialized above and CUIAutomation is an in-process COM server.
    let automation: Result<IUIAutomation, _> =
        unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
    let automation = match automation {
        Ok(automation) => automation,
        Err(error) => {
            let _ = ready.send(Err(ContextError::Failed(error.to_string())));
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut targets = HashMap::<ContextToken, TargetRecord>::new();
    while let Ok(command) = receiver.recv() {
        targets.retain(|_, target| target.created.elapsed() <= TOKEN_LIFETIME);
        match command {
            WorkerCommand::Capture { target, reply } => {
                let _ = reply.send(capture(&automation, target, &mut targets));
            }
            WorkerCommand::Replace { token, text, reply } => {
                let result = replace(&automation, &mut targets, &token, &text);
                let _ = reply.send(result);
            }
            WorkerCommand::Shutdown => break,
        }
    }
    drop(targets);
    drop(automation);
    // SAFETY: balances the successful CoInitializeEx on this thread after all COM values drop.
    unsafe { CoUninitialize() };
}

fn capture(
    automation: &IUIAutomation,
    target: ForegroundTarget,
    targets: &mut HashMap<ContextToken, TargetRecord>,
) -> Result<ContextSnapshot, ContextError> {
    let captured_at_ms = now_ms();
    // SAFETY: UIA calls run on the owning COM MTA thread. The focused element is queried before
    // LexWisp shows its window, and is never exposed outside this worker.
    let element = unsafe { automation.GetFocusedElement() }
        .map_err(|error| ContextError::Failed(error.to_string()))?;
    // SAFETY: property access stays on the UIA thread.
    let process_id = unsafe { element.CurrentProcessId() }
        .map_err(|error| ContextError::Failed(error.to_string()))? as u32;
    if process_id != target.process_id {
        return Ok(snapshot_for(
            captured_at_ms,
            &target,
            CaptureStatus::NoSelection,
            None,
            None,
            "The focused control changed before selection capture completed.",
        ));
    }
    // SAFETY: property access stays on the UIA thread.
    if unsafe { element.CurrentIsPassword() }
        .map(bool::from)
        .unwrap_or(false)
    {
        return Ok(snapshot_for(
            captured_at_ms,
            &target,
            CaptureStatus::PermissionDenied,
            None,
            None,
            "Password and protected controls are never captured.",
        ));
    }

    match selected_text(&element) {
        Ok(text) if !text.trim().is_empty() => {
            let token = ContextToken::new();
            targets.insert(
                token.clone(),
                TargetRecord {
                    target: target.clone(),
                    element,
                    selected_text: text.clone(),
                    created: Instant::now(),
                },
            );
            Ok(snapshot_for(
                captured_at_ms,
                &target,
                CaptureStatus::VerifiedSelection,
                Some(text),
                Some(token),
                "Selection verified through Windows UI Automation.",
            ))
        }
        Ok(_) => Ok(snapshot_for(
            captured_at_ms,
            &target,
            CaptureStatus::NoSelection,
            None,
            None,
            "The focused control reports no selected text.",
        )),
        Err(_) => match copy_fallback(&target) {
            Ok(Some(text)) => Ok(snapshot_for(
                captured_at_ms,
                &target,
                CaptureStatus::CandidateText,
                Some(text),
                None,
                "Windows copy fallback produced text. Confirm it before sending; it is not a verified selection.",
            )),
            Ok(None) => Ok(snapshot_for(
                captured_at_ms,
                &target,
                CaptureStatus::NoSelection,
                None,
                None,
                "No selected text was available. Type or paste text manually.",
            )),
            Err(error) => Ok(snapshot_for(
                captured_at_ms,
                &target,
                CaptureStatus::Unsupported,
                None,
                None,
                error.to_string(),
            )),
        },
    }
}

fn selected_text(element: &IUIAutomationElement) -> Result<String, ContextError> {
    // SAFETY: UIA pattern and ranges remain on the worker's COM thread.
    let pattern: IUIAutomationTextPattern =
        unsafe { element.GetCurrentPatternAs(UIA_TextPatternId) }
            .map_err(|error| ContextError::Failed(error.to_string()))?;
    // SAFETY: same COM ownership as above.
    let ranges = unsafe { pattern.GetSelection() }
        .map_err(|error| ContextError::Failed(error.to_string()))?;
    let count =
        unsafe { ranges.Length() }.map_err(|error| ContextError::Failed(error.to_string()))?;
    let mut text = String::new();
    for index in 0..count {
        let range = unsafe { ranges.GetElement(index) }
            .map_err(|error| ContextError::Failed(error.to_string()))?;
        let part = unsafe { range.GetText(-1) }
            .map_err(|error| ContextError::Failed(error.to_string()))?;
        text.push_str(&part.to_string());
    }
    Ok(text)
}

fn replace(
    _automation: &IUIAutomation,
    targets: &mut HashMap<ContextToken, TargetRecord>,
    token: &ContextToken,
    text: &str,
) -> Result<ReplaceOutcome, ContextError> {
    let Some(target) = targets.get(token) else {
        write_unicode_clipboard(text)?;
        return Err(ContextError::Expired);
    };
    if target.created.elapsed() > TOKEN_LIFETIME {
        targets.remove(token);
        write_unicode_clipboard(text)?;
        return Err(ContextError::Expired);
    }
    // SAFETY: HWND validity and process identity are checked immediately before activation.
    let valid_window = unsafe { IsWindow(target.target.window as HWND) } != 0;
    let mut process_id = 0_u32;
    if valid_window {
        // SAFETY: process ID output is valid for the duration of the call.
        unsafe {
            GetWindowThreadProcessId(target.target.window as HWND, &mut process_id);
        }
    }
    let current = selected_text(&target.element).unwrap_or_default();
    if !valid_window || process_id != target.target.process_id || current != target.selected_text {
        write_unicode_clipboard(text)?;
        return Err(ContextError::TargetChanged);
    }
    write_unicode_clipboard(text)?;
    // SAFETY: activation targets the still-valid original HWND. Failure is handled without input.
    if unsafe { SetForegroundWindow(target.target.window as HWND) } == 0 {
        return Err(ContextError::PermissionDenied);
    }
    send_shortcut(b'V' as u16)?;
    targets.remove(token);
    Ok(ReplaceOutcome::ReplacedClipboardKept)
}

fn snapshot_for(
    captured_at_ms: u64,
    target: &ForegroundTarget,
    status: CaptureStatus,
    text: Option<String>,
    replace_token: Option<ContextToken>,
    detail: impl Into<String>,
) -> ContextSnapshot {
    ContextSnapshot {
        captured_at_ms,
        status,
        text,
        process_id: target.process_id,
        window_title: target.title.clone(),
        replace_token,
        detail: detail.into(),
    }
}

fn foreground_target() -> Option<ForegroundTarget> {
    // SAFETY: Win32 returns a borrowed handle and fills the process ID synchronously.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }
    let mut process_id = 0_u32;
    unsafe {
        GetWindowThreadProcessId(window, &mut process_id);
    }
    if process_id == 0 || process_id == unsafe { GetCurrentProcessId() } {
        return None;
    }
    // SAFETY: the UTF-16 buffer is sized from the current title length and remains valid.
    let title = unsafe {
        let length = GetWindowTextLengthW(window).max(0) as usize;
        let mut buffer = vec![0_u16; length.saturating_add(1)];
        let copied =
            GetWindowTextW(window, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
    };
    Some(ForegroundTarget {
        window: window as isize,
        process_id,
        title,
    })
}

struct ClipboardSnapshot(Vec<(u32, Vec<u8>)>);

fn copy_fallback(target: &ForegroundTarget) -> Result<Option<String>, ContextError> {
    if foreground_target().as_ref().map(|value| value.window) != Some(target.window) {
        return Err(ContextError::TargetChanged);
    }
    wait_for_hotkey_release();
    let snapshot = snapshot_clipboard()?;
    if foreground_target().as_ref().map(|value| value.window) != Some(target.window) {
        return Err(ContextError::TargetChanged);
    }
    let before = unsafe { GetClipboardSequenceNumber() };
    send_shortcut(b'C' as u16)?;
    let deadline = Instant::now() + COPY_WAIT;
    let changed = loop {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        if sequence != before {
            break Some(sequence);
        }
        if Instant::now() >= deadline {
            break None;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let Some(transaction_sequence) = changed else {
        return Ok(None);
    };
    let text = read_unicode_clipboard()?;
    if unsafe { GetClipboardSequenceNumber() } == transaction_sequence {
        restore_clipboard(snapshot)?;
    }
    Ok(text.filter(|value| !value.trim().is_empty()))
}

fn wait_for_hotkey_release() {
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        // SAFETY: GetAsyncKeyState has no pointer arguments and only samples key state.
        let down = unsafe {
            [VK_CONTROL, VK_MENU, VK_SHIFT]
                .into_iter()
                .any(|key| GetAsyncKeyState(key as i32) < 0)
        };
        if !down {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn send_shortcut(key: u16) -> Result<(), ContextError> {
    let key_input = |virtual_key, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [
        key_input(VK_CONTROL, 0),
        key_input(key, 0),
        key_input(key, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    // SAFETY: input structures are fully initialized and live through the synchronous call.
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            mem::size_of::<INPUT>() as i32,
        )
    };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(ContextError::PermissionDenied)
    }
}

fn snapshot_clipboard() -> Result<ClipboardSnapshot, ContextError> {
    with_open_clipboard(|| {
        let mut formats = Vec::new();
        let mut format = 0_u32;
        loop {
            // SAFETY: clipboard is open on this thread; zero starts enumeration.
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if matches!(
                format,
                CF_BITMAP | CF_METAFILEPICT | CF_PALETTE | CF_ENHMETAFILE
            ) {
                return Err(ContextError::Unsupported);
            }
            // SAFETY: clipboard is open; the returned handle remains owned by the clipboard.
            let handle = unsafe { GetClipboardData(format) } as HGLOBAL;
            if handle.is_null() {
                return Err(ContextError::Unsupported);
            }
            let size = unsafe { GlobalSize(handle) };
            if size == 0 {
                return Err(ContextError::Unsupported);
            }
            let source = unsafe { GlobalLock(handle) };
            if source.is_null() {
                return Err(ContextError::Unsupported);
            }
            // SAFETY: GlobalLock exposes `size` readable bytes until GlobalUnlock.
            let bytes = unsafe { std::slice::from_raw_parts(source.cast::<u8>(), size) }.to_vec();
            unsafe {
                GlobalUnlock(handle);
            }
            formats.push((format, bytes));
        }
        Ok(ClipboardSnapshot(formats))
    })
}

fn restore_clipboard(snapshot: ClipboardSnapshot) -> Result<(), ContextError> {
    with_open_clipboard(|| {
        if unsafe { EmptyClipboard() } == 0 {
            return Err(last_context_error("could not empty the clipboard"));
        }
        for (format, bytes) in snapshot.0 {
            set_clipboard_bytes(format, &bytes)?;
        }
        Ok(())
    })
}

fn write_unicode_clipboard(text: &str) -> Result<(), ContextError> {
    let mut utf16: Vec<u16> = text.encode_utf16().collect();
    utf16.push(0);
    // SAFETY: u16 has no padding and the byte slice lives through SetClipboardData.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            utf16.as_ptr().cast::<u8>(),
            utf16.len().saturating_mul(mem::size_of::<u16>()),
        )
    };
    with_open_clipboard(|| {
        if unsafe { EmptyClipboard() } == 0 {
            return Err(last_context_error("could not empty the clipboard"));
        }
        set_clipboard_bytes(CF_UNICODETEXT, bytes)
    })
}

fn read_unicode_clipboard() -> Result<Option<String>, ContextError> {
    with_open_clipboard(|| {
        if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT) } == 0 {
            return Ok(None);
        }
        let handle = unsafe { GetClipboardData(CF_UNICODETEXT) } as HGLOBAL;
        if handle.is_null() {
            return Ok(None);
        }
        let size = unsafe { GlobalSize(handle) } / mem::size_of::<u16>();
        let source = unsafe { GlobalLock(handle) };
        if source.is_null() {
            return Err(last_context_error("could not read clipboard text"));
        }
        let units = unsafe { std::slice::from_raw_parts(source.cast::<u16>(), size) };
        let end = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        let text = String::from_utf16_lossy(&units[..end]);
        unsafe {
            GlobalUnlock(handle);
        }
        Ok(Some(text))
    })
}

fn set_clipboard_bytes(format: u32, bytes: &[u8]) -> Result<(), ContextError> {
    // SAFETY: the movable allocation is copied while locked and ownership transfers only after a
    // successful SetClipboardData. Failure frees the still-owned allocation.
    unsafe {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len());
        if memory.is_null() {
            return Err(last_context_error("could not allocate clipboard memory"));
        }
        let destination = GlobalLock(memory);
        if destination.is_null() {
            GlobalFree(memory);
            return Err(last_context_error("could not lock clipboard memory"));
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), destination.cast::<u8>(), bytes.len());
        GlobalUnlock(memory);
        if SetClipboardData(format, memory).is_null() {
            GlobalFree(memory);
            return Err(last_context_error("could not set clipboard data"));
        }
    }
    Ok(())
}

fn with_open_clipboard<T>(
    operation: impl FnOnce() -> Result<T, ContextError>,
) -> Result<T, ContextError> {
    // SAFETY: clipboard is opened and closed on this worker thread without nested operations.
    if unsafe { OpenClipboard(ptr::null_mut()) } == 0 {
        return Err(last_context_error("could not open the clipboard"));
    }
    let result = operation();
    unsafe {
        CloseClipboard();
    }
    result
}

fn last_context_error(prefix: &str) -> ContextError {
    ContextError::Failed(format!("{prefix}: {}", std::io::Error::last_os_error()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

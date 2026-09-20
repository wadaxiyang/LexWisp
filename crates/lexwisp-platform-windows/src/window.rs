use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    UI::WindowsAndMessaging::{
        GetCursorPos, MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, SW_SHOWNORMAL,
        SetForegroundWindow, ShowWindow,
    },
};

#[allow(unsafe_code)]
pub fn display_id_under_cursor() -> Option<u64> {
    let mut cursor = POINT::default();
    // SAFETY: GetCursorPos initializes the caller-owned POINT. MonitorFromPoint returns an opaque
    // process-independent HMONITOR that GPUI's Windows DisplayId represents with the same value.
    if unsafe { GetCursorPos(&mut cursor) } == 0 {
        return None;
    }
    let monitor = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };
    (!monitor.is_null()).then_some(monitor as usize as u64)
}

#[allow(unsafe_code)]
pub fn show_startup_error(message: &str) {
    let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "LexWisp startup error"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: both NUL-terminated buffers live through this synchronous call; no HWND is retained.
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[allow(unsafe_code)]
pub fn hide_native_window(handle: isize) -> Result<(), String> {
    if handle == 0 {
        return Err("the native window handle is null".into());
    }
    // SAFETY: the handle comes from GPUI's live Win32 window and this call does not retain it.
    unsafe { ShowWindow(handle as _, SW_HIDE) };
    Ok(())
}

#[allow(unsafe_code)]
pub fn show_native_window(handle: isize) -> Result<(), String> {
    if handle == 0 {
        return Err("the native window handle is null".into());
    }
    // SAFETY: the handle comes from GPUI's live Win32 window and neither call retains it.
    unsafe {
        ShowWindow(handle as _, SW_SHOWNORMAL);
        SetForegroundWindow(handle as _);
    }
    Ok(())
}

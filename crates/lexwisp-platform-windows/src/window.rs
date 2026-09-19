use windows_sys::Win32::UI::WindowsAndMessaging::{
    MB_ICONERROR, MB_OK, MessageBoxW, SW_HIDE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
};

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

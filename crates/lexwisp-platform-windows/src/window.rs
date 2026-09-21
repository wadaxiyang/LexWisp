use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint},
    UI::WindowsAndMessaging::{
        AdjustWindowRectEx, GWL_EXSTYLE, GWL_STYLE, GetCursorPos, GetWindowLongW, MB_ICONERROR,
        MB_OK, MessageBoxW, SW_HIDE, SW_SHOWNORMAL, SWP_NOACTIVATE, SWP_NOZORDER,
        SetForegroundWindow, SetWindowPos, ShowWindow,
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

#[allow(unsafe_code)]
pub fn set_native_window_bounds(
    handle: isize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    scale_factor: f32,
) -> Result<(), String> {
    if handle == 0 {
        return Err("the native window handle is null".into());
    }
    if ![x, y, width, height, scale_factor]
        .into_iter()
        .all(f32::is_finite)
        || width <= 0.
        || height <= 0.
        || scale_factor <= 0.
    {
        return Err("the requested window bounds are invalid".into());
    }
    let window = handle as _;
    // SAFETY: the handle is the live GPUI-owned HWND. Style reads and bounds adjustment are
    // synchronous; SetWindowPos updates that same window and retains no Rust references.
    let (style, extended_style) = unsafe {
        (
            GetWindowLongW(window, GWL_STYLE) as u32,
            GetWindowLongW(window, GWL_EXSTYLE) as u32,
        )
    };
    let client_width = (width * scale_factor).round() as i32;
    let client_height = (height * scale_factor).round() as i32;
    let mut outer = RECT {
        left: 0,
        top: 0,
        right: client_width,
        bottom: client_height,
    };
    // SAFETY: `outer` is initialized and uniquely borrowed for the duration of the call.
    if unsafe { AdjustWindowRectEx(&mut outer, style, 0, extended_style) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let outer_x = (x * scale_factor).round() as i32 + outer.left;
    let outer_y = (y * scale_factor).round() as i32 + outer.top;
    let outer_width = outer.right - outer.left;
    let outer_height = outer.bottom - outer.top;
    // SAFETY: parameters are finite, range-checked above, and the HWND is live for this call.
    if unsafe {
        SetWindowPos(
            window,
            std::ptr::null_mut(),
            outer_x,
            outer_y,
            outer_width,
            outer_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

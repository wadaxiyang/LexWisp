//! Minimal startup-only Windows diagnostic; move into the platform crate in Stage 1.

pub fn show(message: &str) {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let message: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "LexWisp startup error"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: Both NUL-terminated buffers live through this synchronous call.
        // No HWND is owned, transferred, or accessed from another thread.
        #[allow(unsafe_code)]
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    #[cfg(not(target_os = "windows"))]
    eprintln!("{message}");
}

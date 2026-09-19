use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{FindWindowW, PostMessageW},
};

use crate::shell::{MESSAGE_WINDOW_CLASS, WM_LEXWISP_WAKE};

const MUTEX_NAME: &str = "Local\\LexWisp.SingleInstance.v1";

pub enum SingleInstance {
    First(SingleInstanceGuard),
    ExistingNotified,
}

pub struct SingleInstanceGuard {
    handle: HANDLE,
}

impl SingleInstanceGuard {
    #[allow(unsafe_code)]
    pub fn acquire() -> Result<SingleInstance, String> {
        let name = wide(MUTEX_NAME);
        // SAFETY: the optional security attributes are null and the name buffer is NUL-terminated.
        let handle = unsafe { CreateMutexW(ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            return Err(format!(
                "could not create the instance mutex: {}",
                std::io::Error::last_os_error()
            ));
        }
        // SAFETY: GetLastError has no preconditions and is read immediately after CreateMutexW.
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if already_exists {
            // SAFETY: this handle was returned by CreateMutexW and is no longer needed here.
            unsafe { CloseHandle(handle) };
            notify_existing_instance()?;
            Ok(SingleInstance::ExistingNotified)
        } else {
            Ok(SingleInstance::First(Self { handle }))
        }
    }
}

impl Drop for SingleInstanceGuard {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: this handle is exclusively owned by the guard and released exactly once.
        unsafe { CloseHandle(self.handle) };
    }
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[allow(unsafe_code)]
fn notify_existing_instance() -> Result<(), String> {
    let class = wide(MESSAGE_WINDOW_CLASS);
    for _ in 0..20 {
        // SAFETY: the class pointer is NUL-terminated; a null title matches any window title.
        let window = unsafe { FindWindowW(class.as_ptr(), ptr::null()) };
        if !window.is_null() {
            // SAFETY: the message carries no borrowed pointers and targets a live top-level window.
            if unsafe { PostMessageW(window, WM_LEXWISP_WAKE, 0, 0) } != 0 {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    Err("LexWisp is already running, but its command window did not respond".into())
}

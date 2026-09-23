use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};

use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
};

const MUTEX_NAME: &str = "Local\\LexWisp.SingleInstance.v1";

pub enum SingleInstance {
    First(SingleInstanceGuard),
    Existing,
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
            Ok(SingleInstance::Existing)
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

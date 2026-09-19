use std::{io, os::windows::ffi::OsStrExt, ptr, slice};

use lexwisp_core::{CredentialError, CredentialStore};
use windows_sys::Win32::{
    Foundation::{ERROR_NOT_FOUND, GetLastError},
    Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    },
};

#[derive(Default)]
pub struct WindowsCredentialStore;

impl CredentialStore for WindowsCredentialStore {
    #[allow(unsafe_code)]
    fn read(&self, reference: &str) -> Result<String, CredentialError> {
        let target = wide(reference);
        let mut raw = ptr::null_mut::<CREDENTIALW>();
        // SAFETY: `target` is NUL-terminated for the duration of the call and `raw` is an
        // initialized out pointer. Windows owns the returned allocation until CredFree below.
        if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
            // SAFETY: GetLastError is read immediately after the failed Win32 call.
            let code = unsafe { GetLastError() };
            return if code == ERROR_NOT_FOUND {
                Err(CredentialError::NotFound(reference.to_owned()))
            } else {
                Err(CredentialError::Platform(
                    io::Error::from_raw_os_error(code as i32).to_string(),
                ))
            };
        }
        if raw.is_null() {
            return Err(CredentialError::Platform(
                "Credential Manager returned a null credential".into(),
            ));
        }

        // SAFETY: CredReadW returned a valid CREDENTIALW whose blob remains alive until CredFree.
        let credential = unsafe { &*raw };
        // SAFETY: the blob pointer/size pair belongs to the credential and is valid for reads.
        let bytes = unsafe {
            slice::from_raw_parts(
                credential.CredentialBlob.cast_const(),
                credential.CredentialBlobSize as usize,
            )
        };
        let result =
            String::from_utf8(bytes.to_vec()).map_err(|_| CredentialError::InvalidEncoding);
        // SAFETY: `raw` is exactly the allocation returned by CredReadW and is freed once here.
        unsafe { CredFree(raw.cast()) };
        result
    }

    #[allow(unsafe_code)]
    fn write(&self, reference: &str, secret: &str) -> Result<(), CredentialError> {
        let mut target = wide(reference);
        let mut username = wide("LexWisp provider");
        let mut blob = secret.as_bytes().to_vec();
        if blob.len() > 2_560 {
            blob.fill(0);
            return Err(CredentialError::Platform(
                "credential exceeds the Windows generic credential limit".into(),
            ));
        }
        let credential = CREDENTIALW {
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: blob.as_mut_ptr(),
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            UserName: username.as_mut_ptr(),
            ..Default::default()
        };
        // SAFETY: all pointers in `credential` refer to live, writable buffers for the call.
        let written = unsafe { CredWriteW(&credential, 0) };
        blob.fill(0);
        if written == 0 {
            // SAFETY: GetLastError is read immediately after the failed Win32 call.
            let code = unsafe { GetLastError() };
            Err(CredentialError::Platform(
                io::Error::from_raw_os_error(code as i32).to_string(),
            ))
        } else {
            Ok(())
        }
    }

    #[allow(unsafe_code)]
    fn delete(&self, reference: &str) -> Result<(), CredentialError> {
        let target = wide(reference);
        // SAFETY: `target` is NUL-terminated and valid for the duration of the call.
        if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } != 0 {
            return Ok(());
        }
        // SAFETY: GetLastError is read immediately after the failed Win32 call.
        let code = unsafe { GetLastError() };
        if code == ERROR_NOT_FOUND {
            Ok(())
        } else {
            Err(CredentialError::Platform(
                io::Error::from_raw_os_error(code as i32).to_string(),
            ))
        }
    }
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

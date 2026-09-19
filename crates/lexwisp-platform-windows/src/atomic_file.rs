use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use lexwisp_core::{AtomicFileWriter, FileWriteError};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, GetLastError},
    Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW},
};

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub struct WindowsAtomicFileWriter;

impl AtomicFileWriter for WindowsAtomicFileWriter {
    fn write_atomic(&self, destination: &Path, contents: &[u8]) -> Result<(), FileWriteError> {
        let parent = destination.parent().ok_or_else(|| {
            FileWriteError::CreateDirectory("the destination has no parent directory".into())
        })?;
        fs::create_dir_all(parent)
            .map_err(|error| FileWriteError::CreateDirectory(error.to_string()))?;

        let temporary = temporary_path(destination);
        let write_result = (|| {
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| FileWriteError::WriteTemporary(error.to_string()))?;
            file.write_all(contents)
                .and_then(|()| file.sync_all())
                .map_err(|error| FileWriteError::WriteTemporary(error.to_string()))?;
            drop(file);
            replace(destination, &temporary)
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result
    }
}

fn temporary_path(destination: &Path) -> PathBuf {
    let suffix = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    destination.with_extension(format!("tmp-{}-{suffix}", std::process::id()))
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[allow(unsafe_code)]
fn replace(destination: &Path, temporary: &Path) -> Result<(), FileWriteError> {
    let destination = wide(destination);
    let temporary = wide(temporary);
    // SAFETY: all pointers refer to NUL-terminated buffers that remain alive for the calls.
    let replaced = unsafe {
        ReplaceFileW(
            destination.as_ptr(),
            temporary.as_ptr(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced != 0 {
        return Ok(());
    }
    // SAFETY: GetLastError has no preconditions and is read immediately after ReplaceFileW.
    let error = unsafe { GetLastError() };
    if error != ERROR_FILE_NOT_FOUND {
        return Err(FileWriteError::Replace(
            std::io::Error::from_raw_os_error(error as i32).to_string(),
        ));
    }
    // SAFETY: the source and destination buffers are valid and NUL-terminated for the call.
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(FileWriteError::Replace(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn replaces_an_existing_file_repeatedly() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the test clock must be after the Unix epoch")
            .as_nanos();
        let directory = std::env::temp_dir()
            .join("lexwisp-stage1-atomic-tests")
            .join(format!("{}-{nonce}", std::process::id()));
        let destination = directory.join("settings.toml");
        let writer = WindowsAtomicFileWriter;

        writer
            .write_atomic(&destination, b"generation = 1\n")
            .expect("the first write should succeed");
        writer
            .write_atomic(&destination, b"generation = 2\n")
            .expect("replacing an existing file should succeed");
        assert_eq!(
            fs::read_to_string(&destination).expect("the replacement should be readable"),
            "generation = 2\n"
        );

        fs::remove_dir_all(&directory)
            .expect("the explicit isolated test directory should be removable");
    }
}

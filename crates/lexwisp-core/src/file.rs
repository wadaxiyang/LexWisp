use std::path::Path;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum FileWriteError {
    #[error("could not create the settings directory: {0}")]
    CreateDirectory(String),
    #[error("could not write the temporary settings file: {0}")]
    WriteTemporary(String),
    #[error("could not replace the settings file atomically: {0}")]
    Replace(String),
}

pub trait AtomicFileWriter: Send + Sync {
    fn write_atomic(&self, destination: &Path, contents: &[u8]) -> Result<(), FileWriteError>;
}

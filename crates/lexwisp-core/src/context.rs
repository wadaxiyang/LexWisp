use std::{future::Future, pin::Pin};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ContextToken;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSource {
    Selection,
    Candidate,
    Manual,
    Clipboard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureStatus {
    VerifiedSelection,
    CandidateText,
    NoSelection,
    Unsupported,
    PermissionDenied,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextSnapshot {
    pub captured_at_ms: u64,
    pub status: CaptureStatus,
    pub text: Option<String>,
    pub process_id: u32,
    pub window_title: String,
    pub replace_token: Option<ContextToken>,
    pub detail: String,
}

impl ContextSnapshot {
    pub fn empty(status: CaptureStatus, detail: impl Into<String>) -> Self {
        Self {
            captured_at_ms: 0,
            status,
            text: None,
            process_id: 0,
            window_title: String::new(),
            replace_token: None,
            detail: detail.into(),
        }
    }

    pub const fn is_verified(&self) -> bool {
        matches!(self.status, CaptureStatus::VerifiedSelection)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaceOutcome {
    ReplacedClipboardKept,
    CopiedOnly,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum ContextError {
    #[error("the original selection is no longer valid; the result was copied instead")]
    TargetChanged,
    #[error("the selection expired; the result was copied instead")]
    Expired,
    #[error("the target does not support safe replacement; the result was copied instead")]
    Unsupported,
    #[error("Windows denied access to the target; the result was copied instead")]
    PermissionDenied,
    #[error("context operation failed: {0}")]
    Failed(String),
}

pub type ContextFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, ContextError>> + Send + 'a>>;

pub trait ContextUiPort: Send + Sync {
    fn latest(&self) -> ContextSnapshot;
    fn capture(&self) -> ContextFuture<'_, ContextSnapshot>;
    fn read_clipboard(&self) -> ContextFuture<'_, String>;
    fn copy(&self, text: String) -> ContextFuture<'_, ()>;
    fn replace(&self, token: ContextToken, text: String) -> ContextFuture<'_, ReplaceOutcome>;
}

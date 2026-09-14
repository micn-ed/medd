//! The one error type, serialisable to the frontend (architecture.md §2).
//!
//! Unused outside `document.rs` and its tests until increment 3 gives `commands.rs` something to
//! return it from.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::Serialize;

use crate::document::ContentHash;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MeddError {
    /// A filesystem operation failed for a reason not covered by a more specific variant.
    Io { path: PathBuf, message: String },
    /// The file's bytes are not valid UTF-8, so medd cannot open it as Markdown.
    NotUtf8 { path: PathBuf },
    /// A compare-and-swap write lost the race: the file on disk no longer matches
    /// `expected_hash`. Nothing was written (architecture.md §3).
    Conflict {
        current_content: String,
        hash: ContentHash,
    },
}

impl std::fmt::Display for MeddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MeddError::Io { path, message } => write!(f, "{}: {message}", path.display()),
            MeddError::NotUtf8 { path } => write!(f, "{}: not valid UTF-8", path.display()),
            MeddError::Conflict { .. } => write!(f, "write rejected: file changed on disk"),
        }
    }
}

impl std::error::Error for MeddError {}

//! The one error type, serialisable to the frontend (architecture.md §2).

use std::path::{Path, PathBuf};

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

impl MeddError {
    /// Wraps a `std::io::Error` with the path it happened to, so the message the frontend can
    /// show is meaningful rather than a bare OS error string. Shared by every module that talks
    /// to the filesystem, so the wrapping is consistent everywhere.
    pub fn io(path: impl AsRef<Path>, e: std::io::Error) -> Self {
        MeddError::Io {
            path: path.as_ref().to_path_buf(),
            message: e.to_string(),
        }
    }
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

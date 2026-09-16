//! The one error type, serialisable to the frontend (architecture.md §2).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::document::ContentHash;

#[derive(Debug, Clone, Serialize)]
// `rename_all_fields`, not `rename_all`. The distinction is the whole bug this attribute was
// changed to fix: `rename_all` camelCases the *variant* names, so `Conflict` went over the wire as
// `"conflict"` while every reader in the frontend compares against `'Conflict'` — and it left the
// struct-variant *fields* alone, so `current_content` arrived where `currentContent` was read.
// Both halves were wrong, in opposite directions, and nothing caught it because the frontend tests
// mock the payload they want rather than the one Rust sends. The wire format is pinned by
// `wire_format` below; `doc/doc.ts` points at it rather than restating it.
#[serde(tag = "kind", rename_all_fields = "camelCase")]
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
    /// A directory listing was requested outside the open workspace's root. Not a general
    /// sandboxing boundary — `document_read` still accepts any absolute path by design, for
    /// loose documents (D-15) — just a cheap rejection of enumerating your way to a path nothing
    /// legitimate has a reason to browse to.
    OutsideWorkspace { path: PathBuf },
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
            MeddError::OutsideWorkspace { path } => {
                write!(f, "{}: outside the open workspace", path.display())
            }
        }
    }
}

impl std::error::Error for MeddError {}

#[cfg(test)]
mod wire_format {
    use super::*;
    use crate::document::ContentHash;
    use std::path::PathBuf;

    // This is a *contract* test, and the only place the contract can be tested. The frontend's
    // own tests mock rejections, so they assert that the frontend agrees with itself -- which is
    // how `{kind: "conflict", current_content}` shipped against six green tests all mocking
    // `{kind: "Conflict", currentContent}`. Every CAS-rejected write therefore missed
    // `isConflictError` and fell through to a console line: increment 7's named blocker case,
    // "a rejected compare-and-swap write becoming a conflict", was dead in production.
    //
    // So assert the exact bytes. A string comparison is deliberate over a structural one: the
    // failure was a *name*, and a structural assertion built from the same enum cannot see a
    // renaming.

    #[test]
    fn conflict_is_what_doc_ts_reads() {
        let e = MeddError::Conflict {
            current_content: "theirs".to_string(),
            hash: ContentHash::of(b"theirs"),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(
            json.contains(r#""kind":"Conflict""#),
            "doc.ts's isConflictError compares against 'Conflict': {json}"
        );
        assert!(
            json.contains(r#""currentContent":"theirs""#),
            "doc.ts reads e.currentContent: {json}"
        );
    }

    #[test]
    fn not_utf8_is_what_app_svelte_reads() {
        let e = MeddError::NotUtf8 {
            path: PathBuf::from("/w/x.md"),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains(r#""kind":"NotUtf8""#), "{json}");
        assert!(json.contains(r#""path":"/w/x.md""#), "{json}");
    }

    #[test]
    fn io_and_outside_workspace_are_what_app_svelte_reads() {
        let io = serde_json::to_string(&MeddError::Io {
            path: PathBuf::from("/w/x.md"),
            message: "boom".to_string(),
        })
        .unwrap();
        assert!(io.contains(r#""kind":"Io""#), "{io}");
        assert!(io.contains(r#""message":"boom""#), "{io}");

        let outside = serde_json::to_string(&MeddError::OutsideWorkspace {
            path: PathBuf::from("/elsewhere"),
        })
        .unwrap();
        assert!(
            outside.contains(r#""kind":"OutsideWorkspace""#),
            "{outside}"
        );
    }
}

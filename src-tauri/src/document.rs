//! Read, atomic write, content hashing, compare-and-swap (architecture.md §3).
//!
//! This is the single most dangerous module in the product: everything in it touches a real
//! file someone cares about. No UI calls it yet — plan-v0.1.md increment 2 is deliberately
//! Rust-only, so nothing outside `#[cfg(test)]` calls `DocumentStore` until increment 3 wires it
//! into `commands.rs`. The `dead_code` allow below goes away with that first caller.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::MeddError;

/// The hash of a document's content, as medd last saw it on disk. Compared by value, never
/// parsed or interpreted — it exists to answer one question: "is what's on disk now what I
/// think is there?" (architecture.md §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        ContentHash(blake3::hash(bytes).to_hex().to_string())
    }
}

/// Tracks, for every document medd has read or written, the hash of its content as medd last
/// saw it on disk — keyed by canonical path, guarded by a single mutex.
///
/// The mutex is held for the full duration of a `write()` — CAS check, disk write, and hash
/// bookkeeping — not acquired per-field. That serialises all reads and writes across every open
/// document, not just concurrent writers of the same path. For v0.1's handful of tabs and
/// kilobyte-scale Markdown files that's an unmeasurable cost, and it's what makes "the new hash
/// is recorded before the lock is released" trivially true rather than something a per-path lock
/// table would have to go out of its way to guarantee. Worth revisiting if increment 8's tabs or
/// increment 12's large-file work ever make a single fsync a perceptible stall for an unrelated
/// tab.
pub struct DocumentStore {
    last_known: Mutex<HashMap<PathBuf, ContentHash>>,
}

impl DocumentStore {
    pub fn new() -> Self {
        DocumentStore {
            last_known: Mutex::new(HashMap::new()),
        }
    }

    /// Reads `path`, resolving symlinks first so the tracked key is always the real file
    /// (architecture.md §4: paths crossing the boundary are canonical). Begins tracking it.
    pub fn read(&self, path: &Path) -> Result<(String, ContentHash), MeddError> {
        let canonical = canonicalize(path)?;
        let bytes = fs::read(&canonical).map_err(|e| io_err(&canonical, e))?;
        let content = String::from_utf8(bytes).map_err(|_| MeddError::NotUtf8 {
            path: canonical.clone(),
        })?;
        let hash = ContentHash::of(content.as_bytes());
        self.last_known
            .lock()
            .unwrap()
            .insert(canonical, hash.clone());
        Ok((content, hash))
    }

    /// Compare-and-swap write (architecture.md §3): re-reads and re-hashes `path` first: a
    /// mismatch against `expected_hash` rejects the write with `Conflict` and touches nothing on
    /// disk. Otherwise writes atomically and records the new hash before the lock is released.
    pub fn write(
        &self,
        path: &Path,
        content: &str,
        expected_hash: &ContentHash,
    ) -> Result<ContentHash, MeddError> {
        let canonical = canonicalize(path)?;
        let mut last_known = self.last_known.lock().unwrap();

        let current_bytes = fs::read(&canonical).map_err(|e| io_err(&canonical, e))?;
        let current_hash = ContentHash::of(&current_bytes);
        if current_hash != *expected_hash {
            return Err(MeddError::Conflict {
                current_content: String::from_utf8_lossy(&current_bytes).into_owned(),
                hash: current_hash,
            });
        }

        atomic_write(&canonical, content.as_bytes())?;

        let new_hash = ContentHash::of(content.as_bytes());
        last_known.insert(canonical, new_hash.clone());
        Ok(new_hash)
        // `last_known` drops here, after the insert above: the new hash is recorded before the
        // lock is released.
    }
}

/// Resolves symlinks and relativity before anything touches disk. This is what makes writing
/// through a symlink update the real file rather than replacing the symlink itself — `rename()`
/// unlinks whatever directory entry it's given, so renaming onto the symlink's own path would
/// silently turn it into a plain file. Staging the temp file in the *canonical* target's
/// directory instead means the symlink is never touched, and also avoids an `EXDEV` failure if
/// the symlink and its target live on different filesystems.
fn canonicalize(path: &Path) -> Result<PathBuf, MeddError> {
    path.canonicalize().map_err(|e| io_err(path, e))
}

fn io_err(path: &Path, e: std::io::Error) -> MeddError {
    MeddError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

/// Writes `content` to a temp file beside `target`, fsyncs it, copies `target`'s permissions
/// onto it, then `rename()`s it over `target`. `target` must already exist (compare-and-swap
/// always re-reads it first) and must already be canonical — callers within this module only.
fn atomic_write(target: &Path, content: &[u8]) -> Result<(), MeddError> {
    let dir = target.parent().ok_or_else(|| MeddError::Io {
        path: target.to_path_buf(),
        message: "path has no parent directory".to_string(),
    })?;
    let file_name = target.file_name().ok_or_else(|| MeddError::Io {
        path: target.to_path_buf(),
        message: "path has no file name".to_string(),
    })?;
    let tmp_path = dir.join(format!(".medd-{}.tmp", file_name.to_string_lossy()));

    stage_temp_file(&tmp_path, content).map_err(|e| io_err(&tmp_path, e))?;

    let outcome = (|| {
        let perms = fs::metadata(target)
            .map_err(|e| io_err(target, e))?
            .permissions();
        fs::set_permissions(&tmp_path, perms).map_err(|e| io_err(&tmp_path, e))?;
        fs::rename(&tmp_path, target).map_err(|e| io_err(target, e))?;
        Ok(())
    })();

    if outcome.is_err() {
        // Best-effort: don't leave litter in the workspace if a step after staging failed.
        // The primary error is what the caller sees either way.
        let _ = fs::remove_file(&tmp_path);
    }
    outcome
}

/// The only step that touches disk before the rename. Isolated so a test can call it directly
/// and prove the original is untouched at every point up to (but not including) the rename —
/// which is the actual crash-safety property: `rename()` itself is atomic by the OS's own
/// guarantee, so the only window worth testing is everything that happens before it.
fn stage_temp_file(tmp_path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(tmp_path)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn leftover_temp_files(dir: &Path) -> Vec<PathBuf> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(".medd-"))
                    .unwrap_or(false)
            })
            .collect()
    }

    #[test]
    fn temp_write_leaves_original_untouched_before_rename() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "original content").unwrap();

        let tmp_path = dir.path().join(".medd-note.md.tmp");
        stage_temp_file(&tmp_path, b"new content").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "original content");
        assert_eq!(fs::read_to_string(&tmp_path).unwrap(), "new content");
    }

    #[test]
    fn permissions_survive_rename() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "original").unwrap();
        let mut perms = fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o640);
        fs::set_permissions(&target, perms).unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&target).unwrap();
        store.write(&target, "updated", &hash).unwrap();

        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    #[test]
    fn stale_hash_rejected_and_disk_untouched() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "v1").unwrap();
        let store = DocumentStore::new();
        let (_, hash_v1) = store.read(&target).unwrap();

        fs::write(&target, "v2 - changed externally").unwrap();

        let result = store.write(&target, "attempted overwrite", &hash_v1);
        assert!(matches!(result, Err(MeddError::Conflict { .. })));
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "v2 - changed externally"
        );
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn conflict_carries_current_disk_content() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "v1").unwrap();
        let store = DocumentStore::new();
        let (_, hash_v1) = store.read(&target).unwrap();

        fs::write(&target, "v2 - changed externally").unwrap();

        match store.write(&target, "attempted overwrite", &hash_v1) {
            Err(MeddError::Conflict {
                current_content,
                hash,
            }) => {
                assert_eq!(current_content, "v2 - changed externally");
                assert_eq!(hash, ContentHash::of(b"v2 - changed externally"));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn write_then_read_recognises_own_hash() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "v1").unwrap();
        let store = DocumentStore::new();
        let (_, hash1) = store.read(&target).unwrap();

        let written_hash = store.write(&target, "v2", &hash1).unwrap();

        let (content2, hash2) = store.read(&target).unwrap();
        assert_eq!(content2, "v2");
        assert_eq!(hash2, written_hash);
    }

    #[test]
    fn non_utf8_content_returns_error_not_panic() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("binary.md");
        fs::write(&target, [0xff, 0xfe, 0x00, 0xff]).unwrap();
        let store = DocumentStore::new();
        let result = store.read(&target);
        assert!(matches!(result, Err(MeddError::NotUtf8 { .. })));
    }

    #[test]
    fn empty_file_reads_and_writes_cleanly() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("empty.md");
        fs::write(&target, "").unwrap();
        let store = DocumentStore::new();
        let (content, hash) = store.read(&target).unwrap();
        assert_eq!(content, "");

        let new_hash = store.write(&target, "no longer empty", &hash).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "no longer empty");
        assert_ne!(new_hash, hash);
    }

    #[test]
    fn missing_trailing_newline_preserved_exactly() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("no-newline.md");
        fs::write(&target, "line one\nline two").unwrap();
        let store = DocumentStore::new();
        let (content, _) = store.read(&target).unwrap();
        assert_eq!(content, "line one\nline two");
        assert!(!content.ends_with('\n'));
    }

    #[test]
    fn write_through_symlink_updates_target_and_preserves_link() {
        let dir = tempdir().unwrap();
        let real = dir.path().join("real.md");
        fs::write(&real, "v1").unwrap();
        let link = dir.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&link).unwrap();
        store.write(&link, "v2 via symlink", &hash).unwrap();

        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "v2 via symlink");
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn write_through_symlink_across_directories() {
        let real_dir = tempdir().unwrap();
        let link_dir = tempdir().unwrap();
        let real = real_dir.path().join("real.md");
        fs::write(&real, "v1").unwrap();
        let link = link_dir.path().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&link).unwrap();
        store.write(&link, "v2", &hash).unwrap();

        assert_eq!(fs::read_to_string(&real).unwrap(), "v2");
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(leftover_temp_files(real_dir.path()).is_empty());
        assert!(leftover_temp_files(link_dir.path()).is_empty());
    }
}

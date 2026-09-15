//! Read, atomic write, content hashing, compare-and-swap (architecture.md §3).
//!
//! This is the single most dangerous module in the product: everything in it touches a real
//! file someone cares about. `read()` is wired to `document_read` (increment 3); `write()` is
//! wired to `document_write` and `check_external_change`/`is_tracked` back the watcher's own-
//! write suppression (increment 7).

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::MeddError;

/// The hash of a document's content, as medd last saw it on disk. Compared by value, never
/// parsed or interpreted — it exists to answer one question: "is what's on disk now what I
/// think is there?" (architecture.md §3). Always the hash of the *raw bytes actually on disk* —
/// CRLF and all — never of the LF-normalised text the frontend works with, so it always describes
/// what a subsequent read would actually find there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        ContentHash(blake3::hash(bytes).to_hex().to_string())
    }
}

/// A document's line-ending convention, detected on read and restored on write (architecture.md
/// §3, increment-7 review finding 1). Only two variants exist: a lone `\r` (classic Mac-era line
/// endings) is folded into `Lf` at detection time rather than given a third representation — see
/// `detect_line_ending`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

/// Counts line-break *events* rather than raw byte occurrences, so a `\r\n` pair is one event,
/// not two — an `other`-only count that didn't subtract the CRLF pairs out of the raw `\r`/`\n`
/// tallies would double-count every CRLF document as if it were half-CRLF, half-LF. A lone `\r`
/// (pre-OS-X Mac) counts toward `other`, alongside a lone `\n`: this codebase draws the line at
/// two conventions, not three. A file with no line breaks at all falls out as `Lf` for lack of any
/// evidence either way, which is harmless — there's nothing in it for a line-ending choice to act
/// on until the next write introduces one.
fn detect_line_ending(text: &str) -> LineEnding {
    let crlf = text.matches("\r\n").count();
    let lone_cr = text.matches('\r').count() - crlf;
    let lone_lf = text.matches('\n').count() - crlf;
    if crlf > lone_cr + lone_lf {
        LineEnding::Crlf
    } else {
        LineEnding::Lf
    }
}

/// Collapses every line-break convention to bare `\n` — what the frontend is guaranteed to see,
/// matching what CodeMirror does internally to whatever it's handed (architecture.md §3: "the
/// frontend never sees anything but LF and never learns line endings exist"). Order matters: `\r\n`
/// is collapsed first so the lone-`\r` pass afterward doesn't split already-handled pairs in two.
fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// The inverse of `normalize_to_lf`, applied just before a write. Input is always LF-only (it came
/// from the frontend, or from `normalize_to_lf` itself), so a plain global substitution is exact —
/// no risk of doubling an existing `\r` that normalisation has already removed. A **mixed-ending**
/// source file therefore comes out fully consistent after its first write through medd: whichever
/// convention was dominant at read time wins for the entire file, not just the lines that
/// originally used it. That's a deliberate, documented consequence of collapsing to one
/// in-memory representation, not an oversight.
fn restore_line_ending(text: &str, ending: LineEnding) -> String {
    match ending {
        LineEnding::Lf => text.to_string(),
        LineEnding::Crlf => text.replace('\n', "\r\n"),
    }
}

/// Tracks, for every document medd has read or written, the hash of its content as medd last
/// saw it on disk — keyed by canonical path, guarded by a single mutex. Line-ending convention is
/// deliberately *not* stored here: `write()` and `check_external_change()` each detect it fresh
/// from bytes they've just proven are actually on disk, rather than trusting a value recorded at
/// some earlier, possibly-evicted point — see `write()`'s doc comment for why that matters once
/// `document_close` exists.
///
/// The mutex is held for the full duration of both `read()` and `write()` — including the disk
/// I/O, not just the map bookkeeping — not acquired per-field. That serialises all reads and
/// writes across every open document, not just concurrent access to the same path. For v0.1's
/// handful of tabs and kilobyte-scale Markdown files that's an unmeasurable cost, and it's what
/// makes "the new hash is recorded before the lock is released" trivially true rather than
/// something a per-path lock table would have to go out of its way to guarantee. It is also load-
/// bearing for `read()`: taking the lock only around the final insert would let a concurrent
/// `write()`'s newer hash be clobbered by a `read()` that already had the older bytes in hand —
/// see the `read`/`write` race test below, which fails reliably without the full-duration lock.
/// Worth revisiting if increment 8's tabs or increment 12's large-file work ever make a single
/// fsync a perceptible stall for an unrelated tab.
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
    ///
    /// The lock is held across the disk read itself, not just the final insert: taking it only
    /// at the end would let a concurrent `write()` commit a newer hash that this read then
    /// clobbers with the older one it already had in hand, leaving `last_known` disagreeing with
    /// disk (architecture.md §3).
    ///
    /// The returned content is always LF-normalised (architecture.md §3, review finding 1):
    /// CodeMirror normalises line breaks internally regardless, so returning the raw convention
    /// here would leave the frontend's retained `EditorState` and its `lastSyncedText` in two
    /// different conventions from the moment a CRLF document opens — exactly the mismatch that
    /// made `computeMinimalChange` corrupt a document on external reload.
    pub fn read(&self, path: &Path) -> Result<(String, ContentHash), MeddError> {
        let canonical = canonicalize(path)?;
        let mut last_known = self.last_known.lock().unwrap();
        let bytes = fs::read(&canonical).map_err(|e| MeddError::io(&canonical, e))?;
        let raw = String::from_utf8(bytes).map_err(|_| MeddError::NotUtf8 {
            path: canonical.clone(),
        })?;
        let hash = ContentHash::of(raw.as_bytes());
        last_known.insert(canonical, hash.clone());
        Ok((normalize_to_lf(&raw), hash))
    }

    /// Compare-and-swap write (architecture.md §3): re-reads and re-hashes `path` first: a
    /// mismatch against `expected_hash` rejects the write with `Conflict` and touches nothing on
    /// disk. Otherwise restores `content` (always LF, per `read`'s contract) to the convention
    /// those just-re-read bytes are using, writes *that* atomically, and hashes and records what
    /// was actually written — never the LF form the frontend sent — before the lock is released.
    ///
    /// The convention is detected from `current_bytes` itself, not from anything recorded earlier
    /// — deliberately, even though every real write follows a `document_read` that could have
    /// remembered one. Two changes queued alongside this one (`document_close` evicting a path's
    /// tracking entry the instant a tab closes, and a close-time flush that writes *at* that same
    /// moment) would otherwise create an ordering hazard: if the evict reaches Rust first, a
    /// remembered value would already be gone, and falling back to some default would silently
    /// convert a CRLF file out from under the write meant to protect the user's last edit.
    /// `current_bytes` needs no such fallback: by the time this line runs, the hash check above
    /// has already proven it *is* what's on disk, which makes it the one source of truth for the
    /// file's convention that no eviction could ever invalidate.
    pub fn write(
        &self,
        path: &Path,
        content: &str,
        expected_hash: &ContentHash,
    ) -> Result<ContentHash, MeddError> {
        let canonical = canonicalize(path)?;
        let mut last_known = self.last_known.lock().unwrap();

        let current_bytes = fs::read(&canonical).map_err(|e| MeddError::io(&canonical, e))?;
        let current_hash = ContentHash::of(&current_bytes);
        let current_raw = String::from_utf8_lossy(&current_bytes);
        if current_hash != *expected_hash {
            return Err(MeddError::Conflict {
                current_content: normalize_to_lf(&current_raw),
                hash: current_hash,
            });
        }

        let line_ending = detect_line_ending(&current_raw);
        let disk_content = restore_line_ending(content, line_ending);

        atomic_write(&canonical, disk_content.as_bytes())?;

        let new_hash = ContentHash::of(disk_content.as_bytes());
        last_known.insert(canonical, new_hash.clone());
        Ok(new_hash)
        // `last_known` drops here, after the insert above: the new hash is recorded before the
        // lock is released.
    }

    /// Whether `path` is a document medd is currently tracking (has been read or written at
    /// least once and not since closed). Used by the watcher to decide whether a changed path
    /// needs the hash-comparison dance below, or just folds into a generic `tree:changed`.
    pub fn is_tracked(&self, path: &Path) -> bool {
        self.last_known
            .lock()
            .unwrap()
            .contains_key(&tracking_key(path))
    }

    /// Compares a watcher-reported `path` against the hash medd last recorded for it
    /// (architecture.md §3). Returns `None` if `path` isn't tracked at all, *or* if it is and
    /// the content matches — the own-write-echo case, which must be silent and is the common
    /// one. Only a genuine external change or deletion produces `Some`, and a `Changed` payload's
    /// content is always LF-normalised, same as `read()` and for the same reason.
    ///
    /// Deliberately does not require the caller to canonicalise first: a deleted path can no
    /// longer be canonicalised (the syscall needs the target to exist), so this falls back to
    /// treating the raw watcher-reported path as the tracking key when canonicalisation fails.
    /// That's exactly right for the common case (no symlink in the middle of the watched tree,
    /// where the raw path already equals what was canonicalised at open time) and is a known,
    /// accepted gap for the rarer one (a tracked document reached through a mid-tree symlink
    /// whose link — not target — gets deleted): see `tracking_key`.
    pub fn check_external_change(&self, path: &Path) -> Option<ExternalChange> {
        let key = tracking_key(path);
        let mut last_known = self.last_known.lock().unwrap();
        let known_hash = last_known.get(&key)?.clone();

        match fs::read(&key) {
            Ok(bytes) => {
                let current_hash = ContentHash::of(&bytes);
                if current_hash == known_hash {
                    None // our own write, echoing back — the common case, and it must be cheap
                } else {
                    // Lossy on invalid UTF-8, same as before finding 1 (increment-7 review finding
                    // 6, not addressed here) — but `normalize_to_lf` only ever sees the *result*
                    // of that conversion, which is always valid UTF-8 regardless of how lossy it
                    // was, so it's safe to run unconditionally.
                    let raw = String::from_utf8_lossy(&bytes).into_owned();
                    last_known.insert(key, current_hash.clone());
                    Some(ExternalChange::Changed {
                        content: normalize_to_lf(&raw),
                        hash: current_hash,
                    })
                }
            }
            Err(_) => {
                last_known.remove(&key);
                Some(ExternalChange::Removed)
            }
        }
    }
}

/// What a watcher-reported change to a *tracked* document turned out to be, once compared
/// against the hash medd last recorded (architecture.md §3). Own-write echoes aren't a variant
/// here at all — `check_external_change` returns `None` for those, so there's nothing for a
/// caller to accidentally forget to ignore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChange {
    Changed { content: String, hash: ContentHash },
    Removed,
}

/// The key `last_known` is tracked under for `path`: canonical when `path` still exists (the
/// normal case, and what `read`/`write` themselves key by), falling back to `path` verbatim when
/// it doesn't (a deletion — canonicalising requires the target to exist, so there is nothing
/// left to resolve).
fn tracking_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Resolves symlinks and relativity before anything touches disk. This is what makes writing
/// through a symlink update the real file rather than replacing the symlink itself — `rename()`
/// unlinks whatever directory entry it's given, so renaming onto the symlink's own path would
/// silently turn it into a plain file. Staging the temp file in the *canonical* target's
/// directory instead means the symlink is never touched, and also avoids an `EXDEV` failure if
/// the symlink and its target live on different filesystems.
fn canonicalize(path: &Path) -> Result<PathBuf, MeddError> {
    path.canonicalize().map_err(|e| MeddError::io(path, e))
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

    stage_temp_file(&tmp_path, content).map_err(|e| MeddError::io(&tmp_path, e))?;

    let outcome = (|| {
        let perms = fs::metadata(target)
            .map_err(|e| MeddError::io(target, e))?
            .permissions();
        fs::set_permissions(&tmp_path, perms).map_err(|e| MeddError::io(&tmp_path, e))?;
        fs::rename(&tmp_path, target).map_err(|e| MeddError::io(target, e))?;
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
    use std::sync::Arc;
    use std::thread;
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

    /// A `read()` racing a `write()` must never leave `last_known` disagreeing with disk: a
    /// `read()` that took its unlocked bytes before the `write()` landed, but only acquires the
    /// lock afterwards, would overwrite the write's fresh hash with its own stale one. Repeated
    /// 400 times, fresh state each time, because this is a timing-dependent interleaving rather
    /// than something a single run can prove absent — with the lock covering the whole read this
    /// passes reliably; without it, it fails on close to every iteration.
    #[test]
    fn concurrent_read_and_write_keep_last_known_consistent_with_disk() {
        for i in 0..400 {
            let dir = tempdir().unwrap();
            let target = dir.path().join("note.md");
            fs::write(&target, "v1").unwrap();

            let store = Arc::new(DocumentStore::new());
            let (_, hash1) = store.read(&target).unwrap();
            let canonical = target.canonicalize().unwrap();

            let reader = {
                let store = Arc::clone(&store);
                let target = target.clone();
                thread::spawn(move || {
                    let _ = store.read(&target);
                })
            };
            let writer = {
                let store = Arc::clone(&store);
                let target = target.clone();
                thread::spawn(move || {
                    store.write(&target, "v2", &hash1).unwrap();
                })
            };

            reader.join().unwrap();
            writer.join().unwrap();

            let on_disk_hash = ContentHash::of(fs::read(&target).unwrap().as_slice());
            let tracked = store.last_known.lock().unwrap().get(&canonical).cloned();
            assert_eq!(
                tracked,
                Some(on_disk_hash),
                "iteration {i}: last_known disagrees with what's actually on disk"
            );
        }
    }

    // Line-ending handling (increment-7 review finding 1): CodeMirror normalises whatever it's
    // handed to LF internally regardless of what medd hands it, so a CRLF document read verbatim
    // put two different conventions in play between the retained EditorState and lastSyncedText
    // — exactly the mismatch that corrupted a document on external reload. `read()` and `write()`
    // now agree: the frontend only ever sees LF, and the file's own convention is restored right
    // before it touches disk.

    #[test]
    fn detect_line_ending_prefers_crlf_when_it_dominates() {
        assert_eq!(detect_line_ending("a\r\nb\r\nc"), LineEnding::Crlf);
    }

    #[test]
    fn detect_line_ending_prefers_lf_when_it_dominates() {
        assert_eq!(detect_line_ending("a\nb\nc"), LineEnding::Lf);
    }

    #[test]
    fn detect_line_ending_with_no_line_breaks_defaults_to_lf() {
        assert_eq!(detect_line_ending("no breaks here"), LineEnding::Lf);
    }

    #[test]
    fn detect_line_ending_folds_lone_cr_into_lf_not_a_third_convention() {
        // Classic Mac-era endings. The ruling is explicit: fold into Lf, don't invent a third path.
        assert_eq!(detect_line_ending("a\rb\rc"), LineEnding::Lf);
    }

    #[test]
    fn detect_line_ending_mixed_file_dominant_convention_wins() {
        // Three CRLF breaks, one lone LF: CRLF dominates.
        assert_eq!(detect_line_ending("a\r\nb\r\nc\r\nd\ne"), LineEnding::Crlf);
        // The reverse: three LF breaks, one CRLF.
        assert_eq!(detect_line_ending("a\nb\nc\nd\r\ne"), LineEnding::Lf);
    }

    #[test]
    fn normalize_to_lf_collapses_every_convention() {
        assert_eq!(normalize_to_lf("a\r\nb\rc\nd"), "a\nb\nc\nd");
    }

    #[test]
    fn restore_line_ending_converts_lf_back_to_crlf() {
        assert_eq!(
            restore_line_ending("a\nb\nc", LineEnding::Crlf),
            "a\r\nb\r\nc"
        );
    }

    #[test]
    fn restore_line_ending_lf_is_identity() {
        assert_eq!(restore_line_ending("a\nb\nc", LineEnding::Lf), "a\nb\nc");
    }

    #[test]
    fn read_returns_lf_normalised_content_for_a_crlf_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "line one\r\nline two\r\n").unwrap();

        let store = DocumentStore::new();
        let (content, _) = store.read(&target).unwrap();

        assert_eq!(content, "line one\nline two\n");
    }

    #[test]
    fn hash_describes_the_raw_crlf_bytes_not_the_normalised_text() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        let raw = b"line one\r\nline two\r\n";
        fs::write(&target, raw).unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&target).unwrap();

        assert_eq!(hash, ContentHash::of(raw));
    }

    #[test]
    fn write_restores_crlf_for_a_crlf_document_round_trip() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\r\nb\r\nc\r\n").unwrap();
        let store = DocumentStore::new();
        let (content, hash) = store.read(&target).unwrap();
        assert_eq!(content, "a\nb\nc\n");

        // The frontend edits the LF text and sends LF back — never having learned CRLF exists.
        let edited = content.replace('b', "B");
        store.write(&target, &edited, &hash).unwrap();

        assert_eq!(
            fs::read(&target).unwrap(),
            b"a\r\nB\r\nc\r\n",
            "the file's own convention must survive a round trip through medd untouched apart from the edit"
        );
    }

    #[test]
    fn write_preserves_crlf_even_with_no_tracking_entry_for_the_path() {
        // The leader's review: `document_close` (not yet implemented) will remove a path's
        // tracking entry the instant its tab closes, and a close-time flush can legitimately issue
        // a write *at* that same moment. If the line-ending convention were looked up from that
        // entry, an evict-before-flush ordering would find nothing and silently default -- turning
        // the write meant to protect the user's last edit into the thing that corrupts their file.
        // Detecting from the just-re-read, hash-proven `current_bytes` instead means there is
        // nothing here for such an eviction to race against: no `store.read()` call happens below
        // at all, so there is no tracking entry for this path for the whole life of this test.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\r\nb\r\nc\r\n").unwrap();
        let hash = ContentHash::of(b"a\r\nb\r\nc\r\n");
        let store = DocumentStore::new();

        store.write(&target, "a\nB\nc\n", &hash).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"a\r\nB\r\nc\r\n");
    }

    #[test]
    fn write_does_not_touch_an_lf_document_convention() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\nb\nc\n").unwrap();
        let store = DocumentStore::new();
        let (content, hash) = store.read(&target).unwrap();

        store
            .write(&target, &content.replace('b', "B"), &hash)
            .unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"a\nB\nc\n");
    }

    #[test]
    fn a_mixed_ending_file_is_fully_normalised_by_its_first_write() {
        // Documented consequence, not a bug: collapsing to one in-memory representation means the
        // dominant convention at read time wins for the *whole* file on the next write, not just
        // the lines that already used it.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\r\nb\nc\r\nd\r\n").unwrap(); // 3 CRLF, 1 lone LF -> CRLF dominant
        let store = DocumentStore::new();
        let (content, hash) = store.read(&target).unwrap();
        assert_eq!(content, "a\nb\nc\nd\n");

        store.write(&target, &content, &hash).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"a\r\nb\r\nc\r\nd\r\n");
    }

    #[test]
    fn a_clean_external_reload_of_a_crlf_document_matches_disk_exactly() {
        // The actual failure the review found: computeMinimalChange comparing LF-normalised
        // EditorState text against raw CRLF disk content produced a spurious blank line. Proven
        // here at the boundary that made it possible — `check_external_change`'s payload must
        // already be LF, matching what a freshly opened tab's `EditorState` would hold.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\r\nb\r\nc\r\n").unwrap();
        let store = DocumentStore::new();
        store.read(&target).unwrap();

        fs::write(&target, "a\r\nB\r\nc\r\n").unwrap(); // external tool changes one letter

        match store.check_external_change(&target) {
            Some(ExternalChange::Changed { content, .. }) => {
                assert_eq!(content, "a\nB\nc\n");
            }
            other => panic!("expected Changed, got {other:?}"),
        }
    }

    #[test]
    fn a_rejected_cas_writes_conflict_content_is_also_lf_normalised() {
        // The Conflict payload feeds the same Reload path as a watcher event (doc.ts), so it
        // needs the same guarantee — otherwise the corruption reappears via the other route into
        // applyExternalContent.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "a\r\nb\r\nc\r\n").unwrap();
        let store = DocumentStore::new();
        let (_, stale_hash) = store.read(&target).unwrap();

        fs::write(&target, "a\r\nB\r\nc\r\n").unwrap();

        match store.write(&target, "irrelevant", &stale_hash) {
            Err(MeddError::Conflict {
                current_content, ..
            }) => {
                assert_eq!(current_content, "a\nB\nc\n");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}

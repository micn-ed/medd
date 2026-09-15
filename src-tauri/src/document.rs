//! Read, atomic write, content hashing, compare-and-swap (architecture.md §3).
//!
//! This is the single most dangerous module in the product: everything in it touches a real
//! file someone cares about. `read()` is wired to `document_read` (increment 3); `write()` is
//! wired to `document_write` and `check_external_change`/`is_tracked` back the watcher's own-
//! write suppression (increment 7).

use std::collections::{HashMap, HashSet};
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

/// The prefix and suffix medd's staging files are named with. Owned here, in one place, because
/// two separate things have to agree about them: `atomic_write`, which creates them, and
/// `sweep_abandoned_temps`, which recognises them. (The watcher will want the same recogniser when
/// it stops folding medd's own writes into `tree:changed`.) A pattern held as a string literal in
/// more than one place is a shape this project has already been bitten by repeatedly, so the
/// round-trip between these two functions is pinned by a test rather than by care.
const TEMP_PREFIX: &str = ".medd-";
const TEMP_SUFFIX: &str = ".tmp";

/// The staging file name used while writing the document called `target_file_name`.
fn temp_file_name(target_file_name: &str) -> String {
    format!("{TEMP_PREFIX}{target_file_name}{TEMP_SUFFIX}")
}

/// The inverse of `temp_file_name`: the document name a staging file belongs to, or `None` if
/// `name` is not one of medd's staging files at all. An empty target (`.medd-.tmp`) is `None` —
/// there is no document it could belong to.
fn target_of_temp(name: &str) -> Option<&str> {
    let target = name.strip_prefix(TEMP_PREFIX)?.strip_suffix(TEMP_SUFFIX)?;
    (!target.is_empty()).then_some(target)
}

/// Removes abandoned staging files from `dir`, returning the paths it removed.
///
/// **Invariant: a staging file is removed only if no process holds its lock and its target
/// document exists in the same directory.** Both halves are load-bearing and neither is a proxy
/// for the other:
///
/// - **Nobody holds its lock.** `atomic_write` holds an exclusive advisory lock on the staging
///   file from before it writes a byte until after the rename, so a lock that cannot be acquired
///   means some process is mid-write *right now* — this one, or a second instance during the
///   two-window race ADR-003 documents. The kernel releases the lock when that process dies, so
///   "acquirable" is exactly "the writer is gone", answered directly rather than inferred from a
///   timestamp or a process id. A recycled pid would have made us skip a file permanently; a
///   released lock cannot.
/// - **The target document exists.** medd's own litter always has its target present: the litter
///   exists *because* the rename never happened, which leaves the original untouched. A staging
///   file with no corresponding document is therefore not ours, whatever it is named, and is left
///   alone.
///
/// Residual risk, stated rather than assumed away: a file a *user* happened to name
/// `.medd-<something>.tmp`, sitting beside a file named `<something>`, would match both guards and
/// be removed. There is no way to distinguish it, medd cannot open it (the tree hides dotfiles),
/// and the name is specific enough that this is accepted rather than guarded further — a `.md`-only
/// guard would be a guess about future scope that silently stops sweeping the day medd edits
/// anything else.
///
/// Never fails: the write that triggered this has already succeeded, and a cleanup problem must
/// not be reported as a write problem. An unreadable directory, or a file that will not unlink,
/// is skipped.
pub fn sweep_abandoned_temps(dir: &Path) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return removed;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(target) = target_of_temp(name) else {
            continue;
        };
        if !dir.join(target).exists() {
            continue; // not medd's litter: our own always has its target present
        }

        let path = entry.path();
        let Ok(file) = fs::File::open(&path) else {
            continue;
        };
        if file.try_lock().is_err() {
            continue; // someone is writing it right now
        }
        // The lock is dropped with `file` at the end of this iteration; unlinking while holding it
        // is safe, and holding it until then is what stops a concurrent writer claiming the path
        // between the check and the unlink.
        if fs::remove_file(&path).is_ok() {
            removed.push(path);
        }
    }

    removed
}

/// Bounds `sweep_abandoned_temps` to once per directory per session.
///
/// The bound is sound because of what creates litter: a staging file survives only when the
/// process writing it died, so **a live session creates no litter except by ending.** Within one
/// run, a directory that has been swept clean stays clean, and re-scanning it on every autosave
/// would put a `read_dir` on the write path for no possible gain — which §8's cold-start
/// discipline says not to do.
///
/// Crash recovery is unaffected: the litter from a previous run is removed by the first write this
/// run makes to that directory.
pub struct TempSweeper {
    swept: Mutex<HashSet<PathBuf>>,
}

impl TempSweeper {
    pub fn new() -> Self {
        TempSweeper {
            swept: Mutex::new(HashSet::new()),
        }
    }

    /// Sweeps `dir` if it has not already been swept this session. Infallible by design — see
    /// `sweep_abandoned_temps`.
    pub fn sweep_once(&self, dir: &Path) {
        if !self.swept.lock().unwrap().insert(dir.to_path_buf()) {
            return;
        }
        for path in sweep_abandoned_temps(dir) {
            // An editor that deletes files should be able to say which ones it deleted.
            eprintln!("medd: removed abandoned staging file {}", path.display());
        }
    }
}

impl Default for TempSweeper {
    fn default() -> Self {
        Self::new()
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
    let tmp_path = dir.join(temp_file_name(&file_name.to_string_lossy()));

    // `staged` is held until after the rename below, deliberately: it carries the advisory lock
    // `sweep_abandoned_temps` tests, so releasing it earlier would open a window in which a
    // concurrent sweep could judge this very write abandoned and unlink the file out from under
    // the rename. It stays locked briefly over the *target* inode after the rename, which nothing
    // else locks, so that costs nothing.
    let staged = stage_temp_file(&tmp_path, content).map_err(|e| MeddError::io(&tmp_path, e))?;

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
        // The primary error is what the caller sees either way. Litter this *cannot* clean up —
        // the process dying before reaching here — is what `sweep_abandoned_temps` exists for.
        let _ = fs::remove_file(&tmp_path);
    }
    drop(staged);
    outcome
}

/// The only step that touches disk before the rename. Isolated so a test can call it directly
/// and prove the original is untouched at every point up to (but not including) the rename —
/// which is the actual crash-safety property: `rename()` itself is atomic by the OS's own
/// guarantee, so the only window worth testing is everything that happens before it.
///
/// Returns the open handle rather than dropping it, because it holds an exclusive advisory lock on
/// the staging file that must outlive the rename — that lock is the whole mechanism
/// `sweep_abandoned_temps` uses to tell "being written right now" from "abandoned by a process
/// that died".
///
/// The open is deliberately not `File::create`: that truncates first, so a second instance racing
/// for the same staging path would destroy the first's in-progress content *before* discovering it
/// could not take the lock. Opening without truncating, locking, and only then truncating means a
/// lost race fails having touched nothing.
fn stage_temp_file(tmp_path: &Path, content: &[u8]) -> std::io::Result<fs::File> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(tmp_path)?;
    file.try_lock().map_err(std::io::Error::from)?;
    file.set_len(0)?;
    file.write_all(content)?;
    file.sync_all()?;
    Ok(file)
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
        let staged = stage_temp_file(&tmp_path, b"new content").unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), "original content");
        assert_eq!(fs::read_to_string(&tmp_path).unwrap(), "new content");
        drop(staged);
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

    // --- The staging-file sweep (architecture.md §3's temp-file litter) ---------------------
    //
    // The property under test throughout: a staging file is removed only if no process holds its
    // lock and its target document exists in the same directory. Each test below removes exactly
    // one of those conditions and asserts the file survives.

    #[test]
    fn temp_name_round_trips_through_target_of_temp() {
        // The two functions are inverses, and this is what keeps them that way: `atomic_write`
        // creates names with one and the sweep recognises them with the other, so a change to
        // either that isn't mirrored in the other is caught here rather than by the sweep quietly
        // ceasing to recognise medd's own litter.
        for target in [
            "note.md",
            "a.md",
            "spaces in name.md",
            ".hidden.md",
            "no-extension",
        ] {
            let temp = temp_file_name(target);
            assert_eq!(
                target_of_temp(&temp),
                Some(target),
                "round trip failed for {target:?} via {temp:?}"
            );
        }
    }

    #[test]
    fn target_of_temp_rejects_names_that_are_not_ours() {
        assert_eq!(target_of_temp("note.md"), None);
        assert_eq!(target_of_temp("note.md.tmp"), None); // right suffix, no prefix
        assert_eq!(target_of_temp(".medd-note.md"), None); // right prefix, no suffix
        assert_eq!(target_of_temp(".medd-.tmp"), None); // no document it could belong to
        assert_eq!(target_of_temp(""), None);
    }

    #[test]
    fn sweeps_an_abandoned_staging_file() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "the document").unwrap();
        let litter = dir.path().join(temp_file_name("note.md"));
        fs::write(&litter, "abandoned half-write").unwrap();

        let removed = sweep_abandoned_temps(dir.path());

        assert_eq!(removed, vec![litter.clone()]);
        assert!(!litter.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "the document");
    }

    #[test]
    fn leaves_a_staging_file_whose_target_is_missing() {
        // Not medd's litter: ours always has its target present, because the litter exists
        // precisely *because* the rename never happened.
        let dir = tempdir().unwrap();
        let stray = dir.path().join(temp_file_name("never-existed.md"));
        fs::write(&stray, "someone else's file").unwrap();

        let removed = sweep_abandoned_temps(dir.path());

        assert!(removed.is_empty());
        assert!(stray.exists());
    }

    #[test]
    fn leaves_a_staging_file_another_process_holds() {
        // The lock is taken by *this test*, not by production code, and held across the sweep
        // call. That is deliberate: a test that trusted the predicate could pass in a world where
        // the lock check never ran, whereas this one cannot — the thing it asserts about is the
        // thing it is doing.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "the document").unwrap();
        let in_flight = dir.path().join(temp_file_name("note.md"));
        fs::write(&in_flight, "being written right now").unwrap();

        let holder = fs::File::open(&in_flight).unwrap();
        holder
            .try_lock()
            .expect("the test must hold the lock for this to mean anything");

        let removed = sweep_abandoned_temps(dir.path());

        assert!(
            removed.is_empty(),
            "a staging file under an active lock must never be removed"
        );
        assert!(in_flight.exists());
        drop(holder);

        // And once the holder is gone — which is what a crashed writer looks like to the kernel —
        // the same file is swept.
        assert_eq!(sweep_abandoned_temps(dir.path()), vec![in_flight]);
    }

    #[test]
    fn leaves_unrelated_files_alone() {
        let dir = tempdir().unwrap();
        for name in [
            "note.md",
            ".hidden",
            "note.md.tmp",
            "backup.tmp",
            ".medd-note.md",
        ] {
            fs::write(dir.path().join(name), "x").unwrap();
        }

        let removed = sweep_abandoned_temps(dir.path());

        assert!(
            removed.is_empty(),
            "swept something it should not have: {removed:?}"
        );
        for name in [
            "note.md",
            ".hidden",
            "note.md.tmp",
            "backup.tmp",
            ".medd-note.md",
        ] {
            assert!(dir.path().join(name).exists(), "{name} was removed");
        }
    }

    #[test]
    fn sweeps_litter_from_a_genuinely_interrupted_write() {
        // End-to-end against the real staging protocol rather than a hand-made filename: stage a
        // file exactly as `atomic_write` does, then drop it without renaming — which is what a
        // process dying mid-write leaves behind, lock released by the kernel and all.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "v1").unwrap();

        let tmp_path = dir.path().join(temp_file_name("note.md"));
        let staged = stage_temp_file(&tmp_path, b"v2, never renamed").unwrap();
        drop(staged); // the "crash"

        assert!(
            tmp_path.exists(),
            "precondition: the interrupted write left litter"
        );

        let removed = sweep_abandoned_temps(dir.path());

        assert_eq!(removed, vec![tmp_path]);
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "v1",
            "sweeping must never touch the document itself"
        );
        assert!(leftover_temp_files(dir.path()).is_empty());
    }

    #[test]
    fn a_second_staging_attempt_fails_without_destroying_the_first() {
        // Two instances racing for the same staging path (ADR-003's two-window race). The loser
        // must fail having touched nothing — `File::create` would have truncated the winner's
        // in-progress content before discovering it could not take the lock.
        let dir = tempdir().unwrap();
        let tmp_path = dir.path().join(temp_file_name("note.md"));

        let first = stage_temp_file(&tmp_path, b"the winner's content").unwrap();
        let second = stage_temp_file(&tmp_path, b"the loser's content");

        assert!(
            second.is_err(),
            "the second staging attempt must not succeed"
        );
        assert_eq!(
            fs::read_to_string(&tmp_path).unwrap(),
            "the winner's content",
            "the loser truncated the winner's staged content"
        );
        drop(first);
    }

    #[test]
    fn sweep_once_visits_a_directory_only_once_per_session() {
        // A live session creates no litter except by ending, so a directory swept clean stays
        // clean — re-scanning it on every autosave would put a read_dir on the write path for no
        // possible gain.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        fs::write(&target, "the document").unwrap();

        let sweeper = TempSweeper::new();
        let first_litter = dir.path().join(temp_file_name("note.md"));
        fs::write(&first_litter, "x").unwrap();
        sweeper.sweep_once(dir.path());
        assert!(!first_litter.exists(), "the first sweep should have run");

        // Litter appearing afterwards is not swept again this session: the second call is a no-op.
        fs::write(&first_litter, "x").unwrap();
        sweeper.sweep_once(dir.path());
        assert!(
            first_litter.exists(),
            "sweep_once must not re-scan a directory it has already swept"
        );

        // A different directory is still swept.
        let other = tempdir().unwrap();
        fs::write(other.path().join("note.md"), "doc").unwrap();
        let other_litter = other.path().join(temp_file_name("note.md"));
        fs::write(&other_litter, "x").unwrap();
        sweeper.sweep_once(other.path());
        assert!(!other_litter.exists());
    }

    #[test]
    fn sweeping_an_unreadable_directory_is_a_noop_not_an_error() {
        // The write that triggered the sweep has already succeeded; a cleanup problem must never
        // be reported as a write problem, so this returns nothing rather than failing.
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert!(sweep_abandoned_temps(&missing).is_empty());
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

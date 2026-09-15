//! The atomic-write protocol: stage beside the target, fsync, rename over it.
//!
//! One owner, because two modules need it and the property that matters is structural. `document.rs`
//! writes documents through `DocumentStore::write`, which compare-and-swaps first; `state.rs`
//! writes `settings.json` and `state.json`, for which a content hash is meaningless. Before this
//! module the write lived privately in `document.rs`, which made "no document write bypasses the
//! compare-and-swap" true *by construction* — the only thing that could reach the write CASed
//! first. Making it `pub` there would have turned that into a convention; moving it here keeps it
//! structural, because `DocumentStore::write` is still the only function in the crate that writes
//! a document.
//!
//! The staging-file naming lives here for the same reason: it is a property of the protocol rather
//! than of documents. The watcher was already importing a staging-file predicate from
//! `document.rs`, which was the design pointing at this boundary before anyone moved it.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::error::MeddError;

/// The prefix and suffix medd's staging files are named with. Owned here because they are
/// properties of the *protocol*, not of documents: `write` creates them, `is_staging_file`
/// recognises them for the watcher, and `document.rs`'s sweep recognises them for cleanup. That
/// the watcher was importing a staging-file predicate from `document.rs` was the design pointing
/// at this boundary before anyone moved it. A pattern held as a string literal in more than one
/// place is a shape this project has been bitten by repeatedly, so the round-trip between the two
/// functions below is pinned by a test rather than by care.
pub(crate) const TEMP_PREFIX: &str = ".medd-";
pub(crate) const TEMP_SUFFIX: &str = ".tmp";

/// The staging file name used while writing the document called `target_file_name`.
pub(crate) fn temp_file_name(target_file_name: &str) -> String {
    format!("{TEMP_PREFIX}{target_file_name}{TEMP_SUFFIX}")
}

/// The inverse of `temp_file_name`: the document name a staging file belongs to, or `None` if
/// `name` is not one of medd's staging files at all. An empty target (`.medd-.tmp`) is `None` —
/// there is no document it could belong to.
pub(crate) fn target_of_temp(name: &str) -> Option<&str> {
    let target = name.strip_prefix(TEMP_PREFIX)?.strip_suffix(TEMP_SUFFIX)?;
    (!target.is_empty()).then_some(target)
}

/// Whether `path` is one of medd's own staging files.
///
/// The watcher needs this: an atomic write is not a single-path event. FSEvents reports the
/// staging file alongside the document, so without this a write of medd's own would surface as a
/// change to something in the workspace — which is what own-write suppression exists to prevent,
/// arriving by a path the content hash never sees, because the staging file is not a tracked
/// document and has no hash to compare.
///
/// Expressed over `target_of_temp` rather than over the prefix and suffix directly, so the
/// staging-file naming still has exactly one owner (see `TEMP_PREFIX`).
pub(crate) fn is_staging_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .and_then(target_of_temp)
        .is_some()
}

/// What `write` does when `target` does not exist.
///
/// Two callers, opposite answers, and neither is a sensible default for the other — which is why
/// this is a parameter rather than a policy baked into the function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WhenAbsent {
    /// Create it with `mode`. What `state.rs` needs: both of §7's files are absent on a fresh
    /// install, and the previous version's unconditional permissions read made the first
    /// `state.json` write fail with `NotFound`.
    ///
    /// `dead_code` is allowed because this variant's only non-test caller is `state.rs`, which is
    /// increment 11 — it exists now because §7's claim that both state files go through this path
    /// was unimplementable without it, and that was the blocker this module was extracted to
    /// clear. **Remove the allow when `state.rs` constructs it**; clippy will not tell you the
    /// allow has become unnecessary, so it is written down here instead.
    #[allow(dead_code)]
    Create { mode: u32 },
    /// Refuse. What `document.rs` needs: a compare-and-swap write has just seen the document
    /// exist, so an absent target here means it was deleted underneath us, and creating it would
    /// silently recreate a file the user deleted (D-11).
    Fail,
}

/// Writes `content` to a staging file beside `target`, fsyncs it, gives it `target`'s permissions,
/// then `rename()`s it over `target`. `target` must already be canonical.
///
/// **`target` need not exist.** It always does for a document — a compare-and-swap write has just
/// re-read it — but both of §7's state files are absent on a fresh install, and the previous
/// version read the target's permissions unconditionally, so the first `state.json` write returned
/// `NotFound`. When there is no target to inherit from, `new_file_mode` is used instead.
///
/// **Whether an absent target may be created is the caller's policy, not this function's.** It was
/// briefly a mode parameter — create it, with these permissions — and that was wrong for
/// documents. `DocumentStore::write` re-reads the file to compare hashes, so it only reaches here
/// having just seen the document exist; if the document is deleted in the window between that read
/// and the rename, creating it would **silently recreate a file the user deleted**, which is the
/// one thing D-11 says never to do. The old code failed there by accident, because it read the
/// target's permissions unconditionally. So the policy is explicit per caller: documents pass
/// `Fail`, state files pass `Create`.
///
/// This module exists so that property has one owner. Exposing `document.rs`'s private version
/// would have made "no document write bypasses the compare-and-swap" true only by everyone
/// remembering — it holds structurally today because the one function that can reach this CASes
/// first, and `DocumentStore::write` is still the only function in the crate that writes a
/// document.
pub(crate) fn write(target: &Path, content: &[u8], absent: WhenAbsent) -> Result<(), MeddError> {
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
        // Inherit the target's permissions when there is a target; otherwise the caller's mode.
        // `NotFound` is the only error treated as "no target" — anything else (a permissions
        // problem, an I/O fault) is a real failure and must not be silently downgraded into
        // creating a file with default-ish permissions.
        let perms = match fs::metadata(target) {
            Ok(meta) => meta.permissions(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => match absent {
                WhenAbsent::Create { mode } => fs::Permissions::from_mode(mode),
                WhenAbsent::Fail => return Err(MeddError::io(target, e)),
            },
            Err(e) => return Err(MeddError::io(target, e)),
        };
        fs::set_permissions(&tmp_path, perms).map_err(|e| MeddError::io(&tmp_path, e))?;
        fs::rename(&tmp_path, target).map_err(|e| MeddError::io(target, e))?;
        Ok(())
    })();

    if outcome.is_err() {
        // Best-effort: don't leave litter behind if a step after staging failed.
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
pub(crate) fn stage_temp_file(tmp_path: &Path, content: &[u8]) -> std::io::Result<fs::File> {
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
    use tempfile::tempdir;

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
    fn creates_an_absent_target_with_the_callers_mode() {
        // architecture.md §7 says both state files go through this path, and both are absent on a
        // fresh install. The previous version read the target's permissions unconditionally, so
        // the first `state.json` write returned NotFound -- on a fresh install only, which is the
        // configuration the person building it is least likely to be in.
        let dir = tempdir().unwrap();
        let target = dir.path().join("state.json");
        assert!(!target.exists(), "precondition: nothing to inherit from");

        write(&target, b"{}", WhenAbsent::Create { mode: 0o600 }).unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}");
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn an_existing_targets_permissions_win_over_the_callers_mode() {
        // The mode is for *creation* only. A document the user has chmodded must keep whatever
        // they chose, so passing a mode must never overwrite an existing file's permissions.
        let dir = tempdir().unwrap();
        let target = dir.path().join("note.md");
        std::fs::write(&target, "v1").unwrap();
        let mut perms = std::fs::metadata(&target).unwrap().permissions();
        perms.set_mode(0o640);
        std::fs::set_permissions(&target, perms).unwrap();

        write(&target, b"v2", WhenAbsent::Create { mode: 0o600 }).unwrap();

        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640,
            "the caller's mode must not touch an existing file"
        );
    }

    #[test]
    fn a_missing_parent_directory_is_a_real_failure() {
        // Not the NotFound-discrimination test it was first named for: a missing parent makes
        // `metadata(target)` return NotFound as well, and staging fails before the permissions
        // block is reached, so the mutant that collapses every error into "absent" survives it.
        // Renamed to what it actually establishes; the discriminating test is below.
        let dir = tempdir().unwrap();
        let unreachable = dir.path().join("no-such-dir").join("x.json");

        let result = write(&unreachable, b"{}", WhenAbsent::Create { mode: 0o600 });

        assert!(
            result.is_err(),
            "a missing parent directory is a real failure"
        );
    }

    #[test]
    fn an_error_that_is_not_not_found_is_never_treated_as_absent() {
        // The discriminating case, and it took a surviving mutant to find one. A symlink loop
        // makes `metadata` fail with `ELOOP` while the staging file — a different name in the
        // same, perfectly good directory — is created fine. So the permissions block *is* reached
        // with a non-NotFound error, which is the only way to tell "no target to inherit from"
        // apart from "something is wrong here".
        //
        // Collapsing the two would be the same shape as carried finding 5, where every read
        // failure was treated as a deletion: it turns a fault into a silent state change. Here it
        // would rename a fresh file over a broken symlink and call it success.
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::os::unix::fs::symlink(&b, &a).unwrap();
        std::os::unix::fs::symlink(&a, &b).unwrap();
        let kind = std::fs::metadata(&a).unwrap_err().kind();
        assert_ne!(
            kind,
            std::io::ErrorKind::NotFound,
            "precondition: the loop must fail with something other than NotFound, got {kind:?}"
        );

        let result = write(&a, b"content", WhenAbsent::Create { mode: 0o600 });

        assert!(
            result.is_err(),
            "a target that cannot be stat'd for a reason other than absence is a failure"
        );
    }

    #[test]
    fn when_absent_fail_refuses_to_create_the_target() {
        // What documents pass, and why the policy belongs to the caller. `DocumentStore::write`
        // only reaches here having just re-read the document for its hash comparison, so an absent
        // target means it was deleted underneath us — and creating it would silently recreate a
        // file the user deleted, which D-11 forbids in as many words. The previous version got
        // this right *by accident*, by reading permissions unconditionally; a bare mode parameter
        // would have quietly turned that accident into a recreation.
        let dir = tempdir().unwrap();
        let target = dir.path().join("deleted.md");

        let result = write(&target, b"resurrected", WhenAbsent::Fail);

        assert!(result.is_err(), "Fail must not create the target");
        assert!(!target.exists(), "and must leave nothing behind");
        assert!(
            std::fs::read_dir(dir.path()).unwrap().next().is_none(),
            "not even a staging file"
        );
    }
}

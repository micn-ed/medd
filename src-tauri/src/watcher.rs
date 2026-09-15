//! `notify` wrapper, event coalescing, own-write suppression (architecture.md §6).
//!
//! What's watched: the workspace root, recursively, plus the parent directory of each open loose
//! document (D-15), non-recursively. Raw events are coalesced over ~100ms by
//! `notify-debouncer-full` before anything else sees them, and delivered over an mpsc channel —
//! the debouncer itself runs the underlying watcher on its own thread, so a filesystem event
//! storm cannot block command handling. `run_event_loop` (main.rs's job to spawn) drains that
//! channel, checks each surviving path against `DocumentStore`, and emits the Tauri events the
//! frontend's conflict/dirty state machine (`doc/`) reacts to.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, RecommendedCache};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::document::{ContentHash, DocumentStore, ExternalChange};
use crate::quickopen::FileIndex;
use crate::workspace::{is_ignored_name, Workspace};

const COALESCE_WINDOW: Duration = Duration::from_millis(100);

pub struct FsWatcher {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
}

impl FsWatcher {
    pub fn new() -> notify::Result<(Self, mpsc::Receiver<DebounceEventResult>)> {
        let (tx, rx) = mpsc::channel();
        let debouncer = new_debouncer(COALESCE_WINDOW, None, tx)?;
        Ok((FsWatcher { debouncer }, rx))
    }

    pub fn watch_recursive(&mut self, path: &Path) -> notify::Result<()> {
        self.debouncer.watch(path, RecursiveMode::Recursive)
    }

    pub fn watch_non_recursive(&mut self, path: &Path) -> notify::Result<()> {
        self.debouncer.watch(path, RecursiveMode::NonRecursive)
    }

    /// Best-effort: unwatching a path notify never watched (or already stopped watching) is not
    /// a caller error worth propagating — the net effect either way is "not watched any more".
    pub fn unwatch(&mut self, path: &Path) {
        let _ = self.debouncer.unwatch(path);
    }
}

/// True if `path` is noise the watcher should never surface from *within* `workspace_root` —
/// inside `.git/`, `node_modules/`, `target/`, or a dotfile directory somewhere between the root
/// and the changed entry (`is_ignored_name`, `workspace.rs` — shared with `dir_list` and
/// `quickopen`'s walk so the three don't quietly disagree about what "ignored" means).
/// Deliberately root-relative rather than checked against the absolute path: a
/// workspace that itself lives under, or even *is*, a dotfile directory (someone opens their
/// `~/.dotfiles` to edit its README) must not have everything inside it silently ignored just
/// because the root's own name happens to start with a dot. `path` not being under
/// `workspace_root` at all (a loose document's own watch, which has no "root" in this sense)
/// answers `false` — that call belongs to whoever is scoping tree:changed, not to this function.
pub fn is_ignored_in_workspace(workspace_root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(workspace_root) else {
        return false;
    };
    let mut components: Vec<_> = relative.components().collect();
    components.pop(); // the changed entry's own name is never filtered for its own name
    components
        .iter()
        .any(|c| is_ignored_name(&c.as_os_str().to_string_lossy()))
}

/// Reduces a debounced batch to the distinct paths worth considering at all — a single
/// `DebouncedEvent` can carry more than one path (a rename carries both endpoints), and a batch
/// can carry many events for the same path. Ignoring (workspace-relative) and tracked-document
/// classification happen afterwards, once the caller knows which root (if any) each path falls
/// under.
pub fn paths_to_check(events: &[notify_debouncer_full::DebouncedEvent]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut paths = Vec::new();
    for event in events {
        for path in &event.paths {
            if seen.insert(path.clone()) {
                paths.push(path.clone());
            }
        }
    }
    paths
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentChangedPayload {
    path: PathBuf,
    content: String,
    hash: ContentHash,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DocumentRemovedPayload {
    path: PathBuf,
}

/// Drains debounced batches for the lifetime of the app, translating them into the events
/// architecture.md §4 documents: `document:changed-on-disk`, `document:removed-on-disk`, and a
/// coalesced `tree:changed` for everything else within the workspace root. Meant to run on its
/// own thread (main.rs's job to spawn it there) — it blocks on `rx` between batches.
pub fn run_event_loop(rx: mpsc::Receiver<DebounceEventResult>, app: AppHandle) {
    for result in rx {
        let Ok(events) = result else { continue };

        let store = app.state::<DocumentStore>();
        let workspace_root = app
            .state::<Mutex<Option<Workspace>>>()
            .lock()
            .unwrap()
            .as_ref()
            .map(|ws| ws.root().to_path_buf());

        let mut tree_changed = false;

        for path in paths_to_check(&events) {
            if let Some(root) = &workspace_root {
                if is_ignored_in_workspace(root, &path) {
                    continue;
                }
            }

            match store.check_external_change(&path) {
                Some(ExternalChange::Changed { content, hash }) => {
                    let _ = app.emit(
                        "document:changed-on-disk",
                        DocumentChangedPayload {
                            path,
                            content,
                            hash,
                        },
                    );
                }
                Some(ExternalChange::Removed) => {
                    let _ = app.emit("document:removed-on-disk", DocumentRemovedPayload { path });
                    // A tracked file's removal changes quick-open's file list even though it's
                    // not "untracked" — the branch below never sees it, so this arm invalidates
                    // the cache on its own rather than relying on tree_changed to cover it.
                    app.state::<FileIndex>().invalidate();
                }
                None => {
                    // Untracked: only counts toward the tree if it's actually within the
                    // workspace tree. A change under a loose document's own watched directory
                    // is neither shown in, nor part of, the tree, so it stays silent — there is
                    // nothing for tree:changed to mean there.
                    if let Some(root) = &workspace_root {
                        if path.starts_with(root) && !store.is_tracked(&path) {
                            tree_changed = true;
                        }
                    }
                }
            }
        }

        if tree_changed {
            let _ = app.emit("tree:changed", ());
            // A new or moved-in file changes quick-open's file list — invalidate rather than
            // patch incrementally, since a whole-cache rebuild is cheap enough (plan-v0.1.md §9)
            // that tracking the delta precisely isn't worth a second source of truth.
            app.state::<FileIndex>().invalidate();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_anything_under_a_dotfile_directory_within_the_workspace() {
        let root = Path::new("/workspace");
        assert!(is_ignored_in_workspace(root, &root.join(".git/HEAD")));
        assert!(is_ignored_in_workspace(
            root,
            &root.join(".obsidian/workspace.json")
        ));
    }

    #[test]
    fn ignores_anything_under_node_modules_within_the_workspace() {
        let root = Path::new("/workspace");
        assert!(is_ignored_in_workspace(
            root,
            &root.join("node_modules/pkg/index.js")
        ));
    }

    #[test]
    fn does_not_ignore_an_ordinary_document() {
        let root = Path::new("/workspace");
        assert!(!is_ignored_in_workspace(root, &root.join("notes/todo.md")));
    }

    #[test]
    fn does_not_ignore_a_dotfile_that_is_itself_the_changed_entry() {
        let root = Path::new("/workspace");
        // Not a document medd would ever open, but the filter is about ancestry, not the leaf.
        assert!(!is_ignored_in_workspace(root, &root.join(".env")));
    }

    #[test]
    fn does_not_ignore_a_workspace_whose_own_root_is_a_dotfile_directory() {
        // A `~/.dotfiles` repo, opened deliberately to edit its README — the root's own name
        // must never count against anything inside it.
        let root = Path::new("/Users/me/.dotfiles");
        assert!(!is_ignored_in_workspace(root, &root.join("README.md")));
        assert!(!is_ignored_in_workspace(root, &root.join("nvim/init.lua")));
    }

    #[test]
    fn a_path_outside_the_root_entirely_is_not_this_functions_call() {
        let root = Path::new("/workspace");
        assert!(!is_ignored_in_workspace(
            root,
            Path::new("/elsewhere/loose.md")
        ));
    }

    #[test]
    fn paths_to_check_deduplicates_across_events() {
        use notify::{Event, EventKind};
        use notify_debouncer_full::DebouncedEvent;
        use std::time::Instant;

        let now = Instant::now();
        let make = |path: &str| {
            DebouncedEvent::new(
                Event::new(EventKind::Any).add_path(PathBuf::from(path)),
                now,
            )
        };

        let events = vec![
            make("/workspace/a.md"),
            make("/workspace/a.md"), // same path again, e.g. two writes in the window
            make("/workspace/b.md"),
        ];

        let result = paths_to_check(&events);
        assert_eq!(
            result,
            vec![
                PathBuf::from("/workspace/a.md"),
                PathBuf::from("/workspace/b.md")
            ]
        );
    }

    // The two tests below drive a *real* notify watcher against a real temp directory — this is
    // the actual claim plan-v0.1.md increment 7 asks for ("a write followed by its own watcher
    // event produces no notification"), not just the hash-comparison logic in isolation. They
    // wait past the real ~100ms coalescing window, so they're slower than a unit test and
    // depend on FSEvents actually delivering — acceptable for a suite that runs occasionally,
    // and there is no way to test "the watcher, for real" without it being genuinely real.
    //
    // Deliberately not using tempfile's default tempdir() naming here: it prefixes directories
    // with a dot (`.tmpXXXXXXXX`), which is exactly the "workspace root is itself a dotfile
    // directory" case above — using the default would make these tests depend on that being
    // handled correctly rather than testing own-write suppression in isolation.

    use crate::document::{DocumentStore, ExternalChange};
    use std::time::Duration;
    use tempfile::Builder;

    #[test]
    fn own_write_produces_no_notification() {
        let dir = Builder::new()
            .prefix("medd-watcher-test-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&file).unwrap();

        let (mut watcher, rx) = FsWatcher::new().unwrap();
        watcher.watch_non_recursive(dir.path()).unwrap();

        store.write(&file, "v2", &hash).unwrap();

        // No batch at all within the window is a valid "no notification" outcome too — the
        // point under test is that nothing document-facing fires, not that notify necessarily
        // reports something.
        if let Ok(Ok(events)) = rx.recv_timeout(Duration::from_millis(500)) {
            for path in paths_to_check(&events) {
                assert!(
                    store.check_external_change(&path).is_none(),
                    "medd's own write should be suppressed, not reported as external, for {path:?}"
                );
            }
        }
    }

    #[test]
    fn foreign_write_produces_exactly_one_notification_with_correct_content() {
        let dir = Builder::new()
            .prefix("medd-watcher-test-")
            .tempdir()
            .unwrap();
        let file = dir.path().join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let store = DocumentStore::new();
        store.read(&file).unwrap();

        let (mut watcher, rx) = FsWatcher::new().unwrap();
        watcher.watch_non_recursive(dir.path()).unwrap();

        // An external tool (Neovim, git) writing the file directly — not through DocumentStore,
        // so there is no matching last_known hash to echo against.
        std::fs::write(&file, "v2 from elsewhere").unwrap();

        let events = rx
            .recv_timeout(Duration::from_millis(500))
            .expect("expected a debounced batch for the foreign write")
            .expect("watcher reported an error instead of events");

        let mut changes = Vec::new();
        for path in paths_to_check(&events) {
            if let Some(ExternalChange::Changed { content, .. }) =
                store.check_external_change(&path)
            {
                changes.push(content);
            }
        }

        assert_eq!(
            changes,
            vec!["v2 from elsewhere".to_string()],
            "expected exactly one document-changed notification, with the foreign content"
        );
    }
}

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

use crate::document::{is_staging_file, ContentHash, DocumentStore, ExternalChange};
use crate::quickopen::FileIndex;
use crate::workspace::{is_within_ignored, Workspace};

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

/// Whether a watcher-reported `path` sits inside an ignored part of `workspace_root` — checking
/// its **ancestors only**, never its own name.
///
/// The workspace-definition half of this question is `workspace::is_within_ignored`, shared with
/// `dir_list` and quick-open's walk. What stays here is the one thing that is a decision about
/// *watching* rather than about what a workspace contains: **a changed entry is never filtered for
/// its own name.** The watcher's job is to report that something happened, and an entry medd would
/// not open can still be the reason the tree needs relisting — a `.env` appearing is a real change
/// to a real directory. Ancestry is about where the change is; the leaf is about what changed, and
/// only the first decides whether medd cares.
///
/// One line over the shared predicate, not a second implementation of it — which is the point: if
/// the ignore rules change, they change in one place, and this file keeps only its own reason.
pub fn is_ignored_ancestor(workspace_root: &Path, path: &Path) -> bool {
    match path.parent() {
        Some(parent) => is_within_ignored(workspace_root, parent),
        None => false,
    }
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

/// What a debounced batch turned out to mean — the complete set of events the frontend should
/// receive for it.
///
/// Produced by `decide`, consumed by `run_event_loop`. Splitting the two is what makes any of this
/// testable: every observable choice the watcher makes used to live inside a function reachable
/// only through a spawned thread holding an `AppHandle`, so the watcher's entire contract with the
/// frontend was untested while both of its tests were named for notifications it never produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEvent {
    DocumentChanged {
        path: PathBuf,
        content: String,
        hash: ContentHash,
    },
    DocumentRemoved {
        path: PathBuf,
    },
    TreeChanged,
}

/// Drops any path in `paths` that another path in the same batch sits beneath.
///
/// FSEvents reports a directory alongside whatever changed inside it, so a batch routinely carries
/// both `…/notes` and `…/notes/todo.md`. The directory is then an untracked path under the root and
/// counts as a tree change — which means **every autosave used to emit `tree:changed`**, twice
/// over: once for the containing directory and once for the staging file. Harmless only because
/// nothing is wired to that event yet; with W-6 in v0.3 it would relist every expanded directory
/// on every debounce tick.
///
/// A path cannot sit beneath a non-directory, so this needs no filesystem access to be correct —
/// the redundancy is visible in the batch itself. And it loses nothing: whatever changed inside the
/// directory is reported in the same batch and decides on its own behalf. A directory reported
/// *alone* — `mkdir` with nothing in it, or a directory removed — is kept, because then there is
/// no inner path to speak for it.
///
/// This is why `decide` takes a batch and returns a batch. Per-path it could not exist.
fn drop_redundant_ancestors(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| {
            !paths
                .iter()
                .any(|q| q.as_path() != p.as_path() && q.starts_with(p))
        })
        .cloned()
        .collect()
}

/// Turns one debounced batch into the complete set of events for it (architecture.md §4).
///
/// **Batch in, batch out.** `TreeChanged` is a fold across the whole batch — it is emitted at most
/// once however many paths contributed — and `drop_redundant_ancestors` is a comparison *between*
/// paths. A per-path signature would push both back into the shell, which is where they were and
/// why they were untested.
///
/// No `AppHandle`, so a test can drive this against a real `DocumentStore` and a real temp
/// directory and assert on the *emitted set* rather than on one path's classification in
/// isolation. That distinction is not academic: the previous tests asserted
/// `check_external_change(path).is_none()` for each path a real write reported, which is true of a
/// staging file because it is untracked — so they passed while the write they were named for
/// emitted two `tree:changed` events.
pub fn decide(
    events: &[notify_debouncer_full::DebouncedEvent],
    workspace_root: Option<&Path>,
    store: &DocumentStore,
) -> Vec<WatcherEvent> {
    let mut out = Vec::new();
    let mut tree_changed = false;

    for path in drop_redundant_ancestors(paths_to_check(events)) {
        if let Some(root) = workspace_root {
            if is_ignored_ancestor(root, &path) {
                continue;
            }
        }

        // medd's own staging file. Own-write suppression works by content hash, which cannot see
        // this: the staging file is not a tracked document, so it has no hash to compare against
        // and lands in the untracked branch below as a tree change.
        if is_staging_file(&path) {
            continue;
        }

        match store.check_external_change(&path) {
            Some(ExternalChange::Changed { content, hash }) => {
                out.push(WatcherEvent::DocumentChanged {
                    path,
                    content,
                    hash,
                });
            }
            Some(ExternalChange::Removed) => {
                out.push(WatcherEvent::DocumentRemoved { path });
                // A tracked document's deletion *is* a tree change, and the untracked branch
                // below never sees it — so without this the only deletions the tree could not
                // learn about were of the documents the user currently has open.
                tree_changed = true;
            }
            None => {
                // Untracked, or an own-write echo. Only counts toward the tree if it is actually
                // within the workspace: a change under a loose document's own watched directory
                // (D-15) is neither shown in nor part of the tree, so there is nothing for
                // `tree:changed` to mean there.
                if let Some(root) = workspace_root {
                    if path.starts_with(root) && !store.is_tracked(&path) {
                        tree_changed = true;
                    }
                }
            }
        }
    }

    if tree_changed {
        out.push(WatcherEvent::TreeChanged);
    }

    out
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

/// Drains debounced batches for the lifetime of the app and performs the effects `decide` asks
/// for. Meant to run on its own thread (main.rs's job to spawn it there) — it blocks on `rx`
/// between batches.
///
/// **A shell contains no decisions** (architecture.md §2). Every branch here is on a
/// `WatcherEvent` variant, and each arm does only I/O — the variant already carries the decision.
/// Anything that needs a reason belongs in `decide`, which can be tested.
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

        for event in decide(&events, workspace_root.as_deref(), &store) {
            match event {
                WatcherEvent::DocumentChanged {
                    path,
                    content,
                    hash,
                } => {
                    let _ = app.emit(
                        "document:changed-on-disk",
                        DocumentChangedPayload {
                            path,
                            content,
                            hash,
                        },
                    );
                }
                WatcherEvent::DocumentRemoved { path } => {
                    let _ = app.emit("document:removed-on-disk", DocumentRemovedPayload { path });
                    app.state::<FileIndex>().invalidate();
                }
                WatcherEvent::TreeChanged => {
                    let _ = app.emit("tree:changed", ());
                    app.state::<FileIndex>().invalidate();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_entry_is_never_ignored_for_its_own_name() {
        // The watcher's own decision, and the only part of the ignore question that lives in this
        // file. `workspace::is_within_ignored` would say `true` for both of these -- correctly,
        // for its own callers. Here they must be `false`: something changed inside a directory
        // medd cares about, and what changed is not what decides that.
        let root = Path::new("/workspace");
        assert!(!is_ignored_ancestor(root, &root.join(".env")));
        assert!(!is_ignored_ancestor(root, &root.join("node_modules")));
    }

    #[test]
    fn an_ignored_ancestor_is_still_ignored() {
        // The composition is doing its job: the shared predicate's answer for the *parent*.
        let root = Path::new("/workspace");
        assert!(is_ignored_ancestor(root, &root.join(".git/HEAD")));
        assert!(is_ignored_ancestor(
            root,
            &root.join("node_modules/pkg/index.js")
        ));
        assert!(is_ignored_ancestor(root, &root.join("target/debug/medd")));
        assert!(!is_ignored_ancestor(root, &root.join("notes/todo.md")));
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

    // --- drop_redundant_ancestors: pure, and the reason `decide` is batch-shaped -----------

    #[test]
    fn a_directory_reported_alongside_its_contents_is_dropped() {
        let paths = vec![
            PathBuf::from("/w"),
            PathBuf::from("/w/notes"),
            PathBuf::from("/w/notes/todo.md"),
        ];
        assert_eq!(
            drop_redundant_ancestors(paths),
            vec![PathBuf::from("/w/notes/todo.md")]
        );
    }

    #[test]
    fn a_directory_reported_alone_is_kept() {
        // `mkdir` with nothing in it, or a directory removed: no inner path speaks for it, so it
        // has to speak for itself. This is what stops the ancestor-drop from silently losing new
        // empty directories.
        let paths = vec![PathBuf::from("/w"), PathBuf::from("/w/newdir")];
        assert_eq!(
            drop_redundant_ancestors(paths),
            vec![PathBuf::from("/w/newdir")]
        );
    }

    #[test]
    fn siblings_are_both_kept() {
        let paths = vec![PathBuf::from("/w/a.md"), PathBuf::from("/w/b.md")];
        assert_eq!(drop_redundant_ancestors(paths.clone()), paths);
    }

    // --- decide: the emitted set, against a real store and a real filesystem ---------------

    fn batch(paths: &[&Path]) -> Vec<notify_debouncer_full::DebouncedEvent> {
        use notify::{Event, EventKind};
        use notify_debouncer_full::DebouncedEvent;
        use std::time::Instant;
        paths
            .iter()
            .map(|p| {
                DebouncedEvent::new(
                    Event::new(EventKind::Any).add_path(p.to_path_buf()),
                    Instant::now(),
                )
            })
            .collect()
    }

    #[test]
    fn a_tracked_documents_deletion_is_also_a_tree_change() {
        // The second bug the extraction exposed. `check_external_change` reports `Removed`, which
        // never reached the untracked branch that sets `tree_changed` -- so the only deletions the
        // tree could not learn about were of the documents the user currently has open, which is
        // the worst possible subset.
        let dir = tempfile::Builder::new()
            .prefix("medd-decide-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let store = DocumentStore::new();
        store.read(&file).unwrap();
        std::fs::remove_file(&file).unwrap();

        let events = decide(&batch(&[&root, &file]), Some(&root), &store);

        assert_eq!(
            events,
            vec![
                WatcherEvent::DocumentRemoved { path: file.clone() },
                WatcherEvent::TreeChanged,
            ]
        );
    }

    #[test]
    fn a_staging_file_reported_alone_produces_nothing() {
        let dir = tempfile::Builder::new()
            .prefix("medd-decide-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::write(root.join("note.md"), "v1").unwrap();
        let staging = root.join(".medd-note.md.tmp");
        std::fs::write(&staging, "half a write").unwrap();

        let store = DocumentStore::new();
        let events = decide(&batch(&[&staging]), Some(&root), &store);

        assert!(
            events.is_empty(),
            "medd's own staging file is not news: {events:?}"
        );
    }

    #[test]
    fn a_new_file_from_elsewhere_is_one_tree_change() {
        let dir = tempfile::Builder::new()
            .prefix("medd-decide-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let created = root.join("fresh.md");
        std::fs::write(&created, "x").unwrap();

        let store = DocumentStore::new();
        let events = decide(&batch(&[&root, &created]), Some(&root), &store);

        assert_eq!(events, vec![WatcherEvent::TreeChanged]);
    }

    // The two tests below drive a *real* notify watcher against a real temp directory, and assert
    // on the **complete set of events `decide` produces** rather than on one path's classification
    // in isolation. That is the whole point: the tests these replace looped over the paths a real
    // write reported and asserted `check_external_change(path).is_none()` for each -- which is
    // true of a staging file because it is untracked -- so they passed while the write they were
    // named for emitted two `tree:changed` events. The paths were real; the assertion could not
    // see an aggregate.
    //
    // They wait out the real ~100ms coalescing window and depend on FSEvents actually delivering,
    // so they are slower than a unit test. There is no way to test "the watcher, for real" without
    // it being genuinely real.
    //
    // Deliberately not using tempfile's default naming: it prefixes directories with a dot, which
    // is the "workspace root is itself a dotfile directory" case -- using the default would make
    // these depend on that being handled correctly rather than testing own-write suppression.

    fn drain(
        rx: &mpsc::Receiver<DebounceEventResult>,
    ) -> Vec<notify_debouncer_full::DebouncedEvent> {
        // Drain until quiet, not just the first batch: "exactly one notification" has to mean
        // exactly one across everything the write produced, which is a claim about the whole
        // sequence and cannot be made from one batch.
        let mut all = Vec::new();
        while let Ok(Ok(events)) = rx.recv_timeout(Duration::from_millis(500)) {
            all.extend(events);
        }
        all
    }

    #[test]
    fn one_real_write_emits_no_events_at_all() {
        // The acceptance criterion: one real `document_write`, a real watcher, a real store, and
        // the complete emitted set must be **empty**. An autosave is not news to anyone.
        let dir = tempfile::Builder::new()
            .prefix("medd-watcher-test-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let store = DocumentStore::new();
        let (_, hash) = store.read(&file).unwrap();

        let (mut watcher, rx) = FsWatcher::new().unwrap();
        watcher.watch_recursive(&root).unwrap();

        store.write(&file, "v2", &hash).unwrap();

        let events = decide(&drain(&rx), Some(&root), &store);

        assert!(
            events.is_empty(),
            "medd's own write must be invisible to the frontend, got {events:?}"
        );
    }

    #[test]
    fn one_foreign_write_emits_exactly_one_document_changed() {
        let dir = tempfile::Builder::new()
            .prefix("medd-watcher-test-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let store = DocumentStore::new();
        store.read(&file).unwrap();

        let (mut watcher, rx) = FsWatcher::new().unwrap();
        watcher.watch_recursive(&root).unwrap();

        // An external tool (Neovim, git) writing directly -- not through DocumentStore, so there
        // is no matching hash to echo against.
        std::fs::write(&file, "v2 from elsewhere").unwrap();

        let events = decide(&drain(&rx), Some(&root), &store);

        assert_eq!(
            events,
            vec![WatcherEvent::DocumentChanged {
                path: file.clone(),
                content: "v2 from elsewhere".to_string(),
                hash: ContentHash::of(b"v2 from elsewhere"),
            }],
            "a foreign write is exactly one notification, and no tree change"
        );
    }
}

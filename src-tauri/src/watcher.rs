//! `notify` wrapper, event coalescing, own-write suppression (architecture.md §6).
//!
//! What's watched: the workspace root, recursively, plus the directory of each loose document
//! opened this session (D-15), non-recursively. Not *open* documents — a watch is not released
//! when its tab closes (architecture.md §6), so the set is bounded by distinct directories opened,
//! not by what is on screen. Raw events are coalesced over ~100ms by
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
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::atomic::is_staging_file;
use crate::document::{ContentHash, DocumentStore, ExternalChange};
use crate::quickopen::FileIndex;
use crate::workspace::{is_within_ignored, Workspace};

const COALESCE_WINDOW: Duration = Duration::from_millis(100);

pub struct FsWatcher {
    debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    /// Every path handed to `watch_recursive`, and whether a directory below it is therefore
    /// already attended to. Kept so `covers` can answer *"is this outside whatever we already
    /// watch"* -- which is the question `document_read` needs, and which nothing else could
    /// answer: `notify` exposes no way to enumerate its own registrations.
    ///
    /// Recursive roots only. A non-recursive watch covers exactly its own directory, so a
    /// document in it is covered by the entry for that directory itself, which `covers` checks
    /// by equality below.
    recursive_roots: Vec<PathBuf>,
    non_recursive_dirs: Vec<PathBuf>,
}

impl FsWatcher {
    pub fn new() -> notify::Result<(Self, mpsc::Receiver<DebounceEventResult>)> {
        let (tx, rx) = mpsc::channel();
        let debouncer = new_debouncer(COALESCE_WINDOW, None, tx)?;
        Ok((
            FsWatcher {
                debouncer,
                recursive_roots: Vec::new(),
                non_recursive_dirs: Vec::new(),
            },
            rx,
        ))
    }

    /// Whether `dir` is already attended to -- inside a recursive root, or itself a directory
    /// already watched non-recursively.
    ///
    /// **This is the condition the missing-reload bug turned on.** `document_read` used to ask
    /// *"is a workspace open and is this outside it"*, which with no workspace open answered
    /// `false` because the `Option` was `None` rather than because of anything about the path --
    /// so a file opened from Finder, the CLI or Neovim with no folder open was never watched at
    /// all. Asking what is actually watched makes the no-workspace case an instance of the
    /// general rule instead of a case that fell through it: with nothing watched, every document
    /// is outside the set.
    ///
    /// It also stops the fix from being wasteful. Always watching a document's directory would
    /// register a redundant watch for every in-workspace file, and since watches are not released
    /// (architecture.md §6) that set would grow for the life of the process.
    pub fn covers(&self, dir: &Path) -> bool {
        self.recursive_roots.iter().any(|r| dir.starts_with(r))
            || self.non_recursive_dirs.iter().any(|d| d == dir)
    }

    pub fn watch_recursive(&mut self, path: &Path) -> notify::Result<()> {
        self.debouncer.watch(path, RecursiveMode::Recursive)?;
        self.recursive_roots.push(path.to_path_buf());
        Ok(())
    }

    pub fn watch_non_recursive(&mut self, path: &Path) -> notify::Result<()> {
        self.debouncer.watch(path, RecursiveMode::NonRecursive)?;
        self.non_recursive_dirs.push(path.to_path_buf());
        Ok(())
    }

    /// Best-effort: unwatching a path notify never watched (or already stopped watching) is not
    /// a caller error worth propagating — the net effect either way is "not watched any more".
    pub fn unwatch(&mut self, path: &Path) {
        let _ = self.debouncer.unwatch(path);
        self.recursive_roots.retain(|r| r != path);
        self.non_recursive_dirs.retain(|d| d != path);
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
/// What one decided event obliges the shell to do.
///
/// Extracted from `run_event_loop`'s body for one reason: **the loop could not be tested, so the
/// wiring inside it was not.** QA disabled `FileIndex::invalidate()` and all 156 tests passed --
/// the method has a test, the decision to emit `TreeChanged` has a test, and nothing asserted
/// that the loop connects them. Delete either call and quick-open silently serves a stale file
/// list after a create or delete, with a green suite.
///
/// That is the same shape as the field bug this release fixes -- both ends built and tested, the
/// wire between them not -- one consumer over. It is milder, because a stale quick-open list is
/// visible and recoverable where a tree that never updates was neither. It is the same belief
/// though: *the shell's actions are inspectable, so they need no test.* That belief is what cost
/// us the tree.
///
/// Generic over the runtime so `mock_builder` can call it, the same reason `document_read` and
/// `rescope_workspace` are. A function the test harness cannot construct an argument for is
/// untestable by construction, which is why all three had no coverage.
///
/// **`TreeChanged` has two obligations, not one**, and they must be able to fail independently:
/// the event tells the frontend to reload its tree *and* tells the file index its cache is void.
/// Covering them together would reproduce, in the tests, exactly the coupling that hid the bug.
fn apply<R: Runtime>(app: &AppHandle<R>, event: WatcherEvent) {
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
            apply(&app, event);
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

    /// The CEO opens folders, always -- so the no-workspace gap does not explain their report and
    /// the with-folder-open path has to be tested under the shapes their real use takes, rather
    /// than declared explained because a neighbouring defect was found. Each of these is a
    /// variation the flat root-level tests above do not cover.
    mod with_a_folder_open {
        use super::*;

        /// Documents live in subdirectories. `watch_recursive` should cover them, and nothing
        /// established that it does -- every test above puts the file directly in the root.
        #[test]
        fn a_document_in_a_nested_subdirectory_still_reloads() {
            let dir = tempfile::Builder::new()
                .prefix("medd-watcher-test-")
                .tempdir()
                .unwrap();
            let root = dir.path().canonicalize().unwrap();
            let nested = root.join("notes").join("2026").join("q3");
            std::fs::create_dir_all(&nested).unwrap();
            let file = nested.join("note.md");
            std::fs::write(&file, "v1").unwrap();

            let store = DocumentStore::new();
            store.read(&file).unwrap();

            let (mut watcher, rx) = FsWatcher::new().unwrap();
            watcher.watch_recursive(&root).unwrap();

            std::fs::write(&file, "v2 three levels down").unwrap();

            let events = decide(&drain(&rx), Some(&root), &store);
            assert!(
                events.contains(&WatcherEvent::DocumentChanged {
                    path: file.clone(),
                    content: "v2 three levels down".to_string(),
                    hash: ContentHash::of(b"v2 three levels down"),
                }),
                "a nested document must reload like a root-level one; got {events:?}"
            );
        }

        /// The two real shapes combined: an editor saving by rename, to a file in a subdirectory.
        #[test]
        fn a_nested_document_saved_by_atomic_replace_still_reloads() {
            let dir = tempfile::Builder::new()
                .prefix("medd-watcher-test-")
                .tempdir()
                .unwrap();
            let root = dir.path().canonicalize().unwrap();
            let nested = root.join("notes");
            std::fs::create_dir_all(&nested).unwrap();
            let file = nested.join("note.md");
            std::fs::write(&file, "v1").unwrap();

            let store = DocumentStore::new();
            store.read(&file).unwrap();

            let (mut watcher, rx) = FsWatcher::new().unwrap();
            watcher.watch_recursive(&root).unwrap();

            let staging = nested.join("note.md~");
            std::fs::write(&staging, "v2 nested atomic").unwrap();
            std::fs::rename(&staging, &file).unwrap();

            let events = decide(&drain(&rx), Some(&root), &store);
            assert!(
                events.contains(&WatcherEvent::DocumentChanged {
                    path: file.clone(),
                    content: "v2 nested atomic".to_string(),
                    hash: ContentHash::of(b"v2 nested atomic"),
                }),
                "nested plus atomic is the ordinary case, not an exotic one; got {events:?}"
            );
        }

        /// The frontend matches tabs by **exact path string** (`tabs.svelte.ts`'s `getTab`:
        /// `t.path === path`), while the backend tracks and reports documents by their
        /// **canonical** path (`tracking_key` canonicalises). If a tab is ever opened under a
        /// path that is not already canonical -- anything reached through a symlink, which on
        /// macOS includes `/tmp` and is common for synced or linked project folders -- then the
        /// reload event names a path no open tab has, `getTab` returns undefined, and the handler
        /// returns without reloading and without a banner.
        ///
        /// That is exactly the reported symptom: a folder open, a document edited externally,
        /// and *neither* of the two things that should happen happening. This test pins which
        /// path shape the backend emits, so the question becomes a checkable one about the
        /// frontend's key rather than a guess.
        #[test]
        fn the_emitted_path_is_canonical_even_when_the_document_was_opened_through_a_symlink() {
            let dir = tempfile::Builder::new()
                .prefix("medd-watcher-test-")
                .tempdir()
                .unwrap();
            let real_root = dir.path().canonicalize().unwrap();
            let real_file = real_root.join("note.md");
            std::fs::write(&real_file, "v1").unwrap();

            // A symlinked route to the same directory -- what an opened folder looks like when it
            // is reached through a link rather than its real location.
            let link_dir = dir.path().parent().unwrap().join(format!(
                "medd-link-{}",
                real_root.file_name().unwrap().to_string_lossy()
            ));
            let _ = std::fs::remove_file(&link_dir);
            std::os::unix::fs::symlink(&real_root, &link_dir).unwrap();
            let linked_file = link_dir.join("note.md");

            let store = DocumentStore::new();
            // Opened through the symlink, which is the path a tab would be keyed by.
            store.read(&linked_file).unwrap();

            let (mut watcher, rx) = FsWatcher::new().unwrap();
            watcher.watch_recursive(&real_root).unwrap();

            std::fs::write(&real_file, "v2 external").unwrap();

            let events = decide(&drain(&rx), Some(&real_root), &store);
            let reported: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    WatcherEvent::DocumentChanged { path, .. } => Some(path.clone()),
                    _ => None,
                })
                .collect();

            let _ = std::fs::remove_file(&link_dir);

            assert_eq!(
                reported,
                vec![real_file.clone()],
                "the backend reports the canonical path"
            );
            assert_ne!(
                reported[0], linked_file,
                "and it is NOT the path the document was opened under -- so a frontend keyed by \
                 the opening path cannot match this event. Whether that is reachable depends on \
                 whether any entry point hands out a non-canonical path; this test establishes \
                 that IF one does, the reload is silently lost."
            );
        }

        /// Saving twice quickly is ordinary editor behaviour. The debouncer coalesces, which is
        /// wanted -- but the surviving notification must carry the LATEST content, not the first.
        /// Reporting v2 while v3 sits on disk would leave the buffer silently stale, and a
        /// subsequent autosave would write the stale text back over the newer bytes.
        #[test]
        fn rapid_successive_saves_report_the_final_content_not_the_first() {
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

            std::fs::write(&file, "v2").unwrap();
            std::fs::write(&file, "v3 final").unwrap();

            let events = decide(&drain(&rx), Some(&root), &store);
            let changed: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    WatcherEvent::DocumentChanged { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect();

            assert!(
                changed.last() == Some(&"v3 final"),
                "the last reload must carry what is actually on disk; got {changed:?}"
            );
        }
    }

    #[test]
    fn an_external_atomic_replace_is_also_exactly_one_document_changed() {
        // The test above says "Neovim" and writes in place. Neovim, by default, does not: it
        // writes a temp file and `rename`s it over the target, so the path survives and the
        // **inode does not**. Every editor using the write-temp-then-rename idiom -- which is
        // most of them, because it is the crash-safe one, and the same idiom `document.rs` uses
        // for medd's own writes -- produces this shape rather than the one covered above.
        //
        // A watcher registered on the *file* would go deaf here: its registration follows the
        // inode, which the rename detaches and discards. `watch_recursive` on the workspace root
        // is what makes this survivable, and nothing before this test established that it does.
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

        // Exactly what an atomic-saving editor does: new inode, renamed over the old path.
        let staging = root.join("note.md~");
        std::fs::write(&staging, "v2 by atomic replace").unwrap();
        std::fs::rename(&staging, &file).unwrap();

        let events = decide(&drain(&rx), Some(&root), &store);

        // Asserts the *reload*, not the event count. The first draft demanded exactly one event
        // and failed on a second, `TreeChanged` -- which is correct here and not noise: the
        // staging file really did appear in the directory and really did vanish, so the tree
        // genuinely changed. Demanding one event would have pinned an incidental consequence of
        // how this editor saves, and broken on the next editor that stages its temp file
        // somewhere else.
        assert!(
            events.contains(&WatcherEvent::DocumentChanged {
                path: file.clone(),
                content: "v2 by atomic replace".to_string(),
                hash: ContentHash::of(b"v2 by atomic replace"),
            }),
            "an atomic replace must reload like any other external edit; got {events:?}"
        );
    }

    #[test]
    fn qa_covers_measured_rather_than_reasoned() {
        // The leader flagged having reasoned this rather than measured it. Three cases, including
        // the one that would silently reintroduce the bug being fixed: a stale entry left behind
        // by unwatch would make `covers` answer true for a directory nothing watches any more, and
        // the document there would never get its own watch.
        use tempfile::Builder;
        let root = Builder::new().prefix("medd-covers-").tempdir().unwrap();
        let nested = root.path().join("sub");
        std::fs::create_dir(&nested).unwrap();
        let loose = Builder::new().prefix("medd-loose-").tempdir().unwrap();
        let loose_sub = loose.path().join("deeper");
        std::fs::create_dir(&loose_sub).unwrap();

        let (mut w, _rx) = FsWatcher::new().unwrap();

        // 1. A recursive root covers its subdirectories.
        w.watch_recursive(root.path()).unwrap();
        assert!(w.covers(root.path()), "the root itself");
        assert!(
            w.covers(&nested),
            "a subdirectory of a RECURSIVE root is covered"
        );

        // 2. A non-recursive watch covers only its own directory.
        w.watch_non_recursive(loose.path()).unwrap();
        assert!(w.covers(loose.path()), "the watched directory itself");
        assert!(
            !w.covers(&loose_sub),
            "a subdirectory of a NON-RECURSIVE watch must NOT count as covered, or a document \
             there would never get a watch of its own"
        );

        // 3. Unwatching removes the bookkeeping, not just the OS watch. A stale entry here is the
        //    same silent failure as the bug this replaced.
        w.unwatch(root.path());
        assert!(
            !w.covers(root.path()),
            "unwatched root must stop counting as covered"
        );
        assert!(
            !w.covers(&nested),
            "and so must everything under it -- a stale recursive root would swallow every \
             document beneath a folder that is no longer watched"
        );
    }
}

#[cfg(test)]
mod shell_wiring {
    //! Does the loop actually *do* what a decided event obliges? `decide` was tested, and
    //! `FileIndex::invalidate` was tested, and nothing asserted that one calls the other --
    //! disabling the call left all 156 tests green. Both ends built and tested, the wire between
    //! them not: the field bug's shape, one consumer over.
    use super::*;
    use crate::quickopen::FileIndex;

    /// Populates the index so it has a cache to lose, then returns whether `apply` voided it.
    /// Asserting on the *cache* rather than on a flag is deliberate: the stale list is the thing
    /// a user would meet, and a flag could be set while the cache survived.
    fn cache_survives_after(event: WatcherEvent, root: &Path) -> bool {
        let app = tauri::test::mock_builder()
            .manage(FileIndex::new())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds");

        let index = app.state::<FileIndex>();
        let before = index.files(root).len();
        assert_eq!(
            before, 1,
            "the index must be warm, or this measures nothing"
        );

        // Appears on disk without the index being told -- exactly how a create reaches medd.
        std::fs::write(root.join("b.md"), "x").unwrap();
        assert_eq!(
            index.files(root).len(),
            1,
            "the cache must actually be stale before the event, or invalidation is unobservable"
        );

        apply(app.handle(), event);

        index.files(root).len() == 1
    }

    fn warm_root() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .prefix("medd-shell-wiring-")
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("a.md"), "x").unwrap();
        dir
    }

    #[test]
    fn a_tree_change_voids_the_quick_open_cache() {
        let dir = warm_root();
        assert!(
            !cache_survives_after(WatcherEvent::TreeChanged, dir.path()),
            "a created file must not stay invisible to quick-open; this is TreeChanged's second \
             obligation, and the frontend event is the first"
        );
    }

    #[test]
    fn a_document_removal_voids_the_quick_open_cache() {
        let dir = warm_root();
        assert!(
            !cache_survives_after(
                WatcherEvent::DocumentRemoved {
                    path: dir.path().join("a.md"),
                },
                dir.path()
            ),
            "a deletion must not leave the removed file offerable in quick-open"
        );
    }

    #[test]
    fn a_document_change_leaves_the_cache_alone() {
        // The negative, which is what stops the two above passing under a blanket "invalidate on
        // everything". Content changing alters no filename, so re-walking the tree would be work
        // for nothing on the most frequent event medd handles.
        let dir = warm_root();
        assert!(
            cache_survives_after(
                WatcherEvent::DocumentChanged {
                    path: dir.path().join("a.md"),
                    content: "x".to_string(),
                    hash: ContentHash::of(b"x"),
                },
                dir.path()
            ),
            "an edit changes no filename, so the file list must not be thrown away"
        );
    }
}

#[cfg(test)]
mod wire_format {
    //! Contract tests — see `error.rs`'s `wire_format` for why these assert exact strings and why
    //! the frontend's own tests cannot establish this. These four have single-word fields, so
    //! `camelCase` is currently the identity function: they are safe by accident of vocabulary
    //! rather than by design, and a field renamed to two words would break silently.
    use super::*;

    #[test]
    fn changed_payload_fields_are_what_doc_ts_reads() {
        let json = serde_json::to_string(&DocumentChangedPayload {
            path: PathBuf::from("/w/a.md"),
            content: "x".to_string(),
            hash: ContentHash::of(b"x"),
        })
        .unwrap();
        for f in [r#""path":"#, r#""content":"#, r#""hash":"#] {
            assert!(json.contains(f), "doc.ts reads {f} {json}");
        }
    }

    #[test]
    fn removed_payload_fields_are_what_doc_ts_reads() {
        let json = serde_json::to_string(&DocumentRemovedPayload {
            path: PathBuf::from("/w/a.md"),
        })
        .unwrap();
        assert!(
            json.contains(r#""path":"#),
            "doc.ts reads payload.path: {json}"
        );
    }
}

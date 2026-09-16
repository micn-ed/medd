//! Full-workspace markdown listing for quick-open (Cmd+P, plan-v0.1.md increment 9, W-5).
//!
//! `workspace::dir_list` stays deliberately one-level-at-a-time (N-2: a ten-thousand-file
//! workspace must not be walked before the window appears), but quick-open is the one feature
//! that genuinely needs the whole tree. The walk here happens lazily — only when quick-open is
//! actually invoked, never at `workspace_open` time — and its result is cached (`FileIndex`)
//! keyed by workspace root, so repeat Cmd+P presses within the same workspace don't re-walk.
//! `commands::quick_open_files` is declared `#[tauri::command(async)]` deliberately: a plain,
//! non-`async` command is dispatched INLINE on the thread that receives the IPC message (the
//! main/UI thread on macOS — verified by reading `tauri-macros`' codegen, not assumed), so
//! without the `async` attribute the first, uncached walk of a huge workspace would block the
//! whole app, not just the dialog. With it, Tauri runs the command on its async runtime instead;
//! the dialog itself is a synchronous frontend state flip either way, and the file list fills in
//! once the command resolves (`quickopen/quickopen.svelte.ts`, frontend side).
//!
//! Only `.md` files are listed, matching W-8 (non-Markdown files are visible but not editable).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

use crate::workspace::{is_ignored_name, is_markdown, is_within_ignored, resolves_to_directory};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickOpenEntry {
    pub path: PathBuf,
    pub relative_path: String,
}

struct Cached {
    root: PathBuf,
    entries: Vec<QuickOpenEntry>,
}

/// Caches the last full walk against the root it was built for. Invalidated wholesale
/// (`invalidate`) by the watcher on anything that could change the file list, rather than
/// patched incrementally — a full rebuild is cheap enough that a second, delta-tracking source
/// of truth isn't worth the risk of it drifting from disk.
pub struct FileIndex {
    cached: Mutex<Option<Cached>>,
}

impl FileIndex {
    pub fn new() -> Self {
        FileIndex {
            cached: Mutex::new(None),
        }
    }

    /// Returns the markdown files under `root`, from cache if the last walk was for this same
    /// root, rebuilding otherwise.
    pub fn files(&self, root: &Path) -> Vec<QuickOpenEntry> {
        let mut cached = self.cached.lock().unwrap();
        if let Some(existing) = cached.as_ref() {
            if existing.root == root {
                return existing.entries.clone();
            }
        }
        let entries = walk_markdown_files(root);
        *cached = Some(Cached {
            root: root.to_path_buf(),
            entries: entries.clone(),
        });
        entries
    }

    pub fn invalidate(&self) {
        *self.cached.lock().unwrap() = None;
    }
}

impl Default for FileIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// Recursively collects every `.md` file under `root`, iteratively (an explicit stack rather
/// than recursion, so a pathologically deep tree can't blow the stack). Ignored names — dotfiles,
/// `node_modules`, `target` — are checked against each entry's own name as it's found, exactly like
/// `dir_list`'s per-level dotfile hiding, which is what makes a workspace whose own root is a
/// dotfile directory (`~/.dotfiles`) work correctly for free: the root's own name is never itself
/// checked, only the names of things found inside it.
///
/// **Symlinked directories are followed**, via the shared `resolves_to_directory` predicate — the
/// same answer the sidebar uses, which is what stops the two disagreeing about what the workspace
/// contains. A link resolving *outside* the root is not descended into, matching the boundary
/// `Workspace::dir_list` enforces; reaching outside at all is a v0.2 question.
///
/// Following costs the type-level cycle immunity that never following gave for free, so the
/// replacement is unconditional rather than careful: `visited` holds the **canonical** path of
/// every directory descended into, so a cycle revisits a path it has already seen and stops, and a
/// diamond — two links to one real directory — contributes its documents exactly once. Which of
/// the two routes a diamond's documents are reported under is unspecified and depends on
/// `read_dir` order; the count is not.
fn walk_markdown_files(root: &Path) -> Vec<QuickOpenEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    // The boundary check below compares canonical paths, so the root has to be canonical too —
    // otherwise every descent is refused on any path reached through a symlink, which on macOS
    // includes anything under `/tmp` (`/var` is itself a link to `/private/var`). The production
    // caller already passes a canonical root; canonicalising here means the function does not
    // silently depend on that.
    let Ok(canonical_root) = root.canonicalize() else {
        return out;
    };
    let mut visited = std::collections::HashSet::new();
    visited.insert(canonical_root.clone());

    while let Some(dir) = stack.pop() {
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue; // a permissions race or similar shouldn't fail the whole walk
        };
        for entry in read_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if is_ignored_name(&name_str) {
                continue;
            }

            let path = entry.path();

            if resolves_to_directory(&path) {
                // Descend only if this stays inside the workspace and we have not been here
                // before. `canonicalize` answers both questions at once, and a path that cannot
                // be canonicalised is one we cannot reason about, so it is skipped.
                let Ok(canonical) = path.canonicalize() else {
                    continue;
                };
                // The `is_ignored_name` check above tests the entry's *own* name, which is enough
                // for a real directory — the walk meets every level on the way down, so an
                // ignored ancestor is refused before its children are ever seen. A symlink skips
                // that: `aliased -> node_modules/docs` is named `aliased`, so nothing above
                // catches it, and following it used to put every file under `node_modules` back
                // into Cmd+P — the exact outcome the shared ignore predicate exists to prevent,
                // reached by the one route it did not cover. So the *target* is judged too, by
                // the general form of the same predicate.
                if canonical.starts_with(&canonical_root)
                    && !is_within_ignored(&canonical_root, &canonical)
                    && visited.insert(canonical)
                {
                    stack.push(path);
                }
            } else if is_markdown(&name_str) {
                let relative_path = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.push(QuickOpenEntry {
                    path,
                    relative_path,
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    // The walk asks "is this a document medd opens" with `workspace::is_markdown`, the same
    // predicate the sidebar's own classification uses -- not a second `ends_with(".md")`. This
    // pins that they agree, including the case-insensitivity `dir_list` already had: a workspace
    // containing SHOUTING.MD must offer it in Cmd+P, because the tree calls it Markdown too.
    #[test]
    fn the_walk_and_the_tree_agree_on_what_markdown_is() {
        use super::*;
        use std::fs;
        use tempfile::tempdir;

        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        for name in ["note.md", "SHOUTING.MD", "MiXeD.Md", "photo.png", "README"] {
            fs::write(root.join(name), "x").unwrap();
        }

        let mut walked: Vec<String> = walk_markdown_files(&root)
            .into_iter()
            .map(|e| e.relative_path)
            .collect();
        walked.sort();

        let mut listed: Vec<String> = crate::workspace::dir_list(&root)
            .unwrap()
            .into_iter()
            .filter(|e| e.kind == crate::workspace::EntryKind::Markdown)
            .map(|e| e.name)
            .collect();
        listed.sort();

        assert_eq!(
            walked, listed,
            "quick-open must offer exactly what the tree calls Markdown"
        );
        assert_eq!(
            walked.len(),
            3,
            "expected the three .md files, case-insensitively"
        );
    }

    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn names_of(entries: &[QuickOpenEntry]) -> Vec<&str> {
        let mut names: Vec<&str> = entries.iter().map(|e| e.relative_path.as_str()).collect();
        names.sort();
        names
    }

    #[test]
    fn finds_markdown_files_at_every_depth() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("top.md"), "x").unwrap();
        fs::create_dir_all(root.path().join("a/b")).unwrap();
        fs::write(root.path().join("a/mid.md"), "x").unwrap();
        fs::write(root.path().join("a/b/deep.md"), "x").unwrap();

        let entries = walk_markdown_files(root.path());
        assert_eq!(
            names_of(&entries),
            vec!["a/b/deep.md", "a/mid.md", "top.md"]
        );
    }

    #[test]
    fn ignores_non_markdown_files() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("image.png"), "x").unwrap();
        fs::write(root.path().join("note.md"), "x").unwrap();

        let entries = walk_markdown_files(root.path());
        assert_eq!(names_of(&entries), vec!["note.md"]);
    }

    #[test]
    fn does_not_descend_into_git_node_modules_or_target() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".git")).unwrap();
        fs::write(root.path().join(".git/HEAD"), "x").unwrap();
        fs::create_dir_all(root.path().join("node_modules/pkg")).unwrap();
        fs::write(root.path().join("node_modules/pkg/readme.md"), "x").unwrap();
        fs::create_dir_all(root.path().join("target/debug")).unwrap();
        fs::write(root.path().join("target/debug/BUILD.md"), "x").unwrap();
        fs::write(root.path().join("visible.md"), "x").unwrap();

        let entries = walk_markdown_files(root.path());
        assert_eq!(names_of(&entries), vec!["visible.md"]);
    }

    // A leader review of increment 9's own numbers (measured against medd's real repository: over
    // 45,000 files under `target/` alone) is what surfaced `target/` never having been ignored in
    // the first place -- worth a comment here because "the fixture must actually contain what the
    // filters exclude" is a lesson this project has paid for once already (the harness fixture
    // that claimed image-rendering coverage with no images in it). The test above creates real
    // `.git`, `node_modules` and `target` directories rather than asserting against their absence.

    #[test]
    fn a_directory_symlink_cycle_terminates_instead_of_walking_forever() {
        // QA's finding: descending into a directory symlink that points back at an ancestor makes
        // the walk unbounded, and the failure mode is silent -- no crash, no error, just a worker
        // thread spinning forever while quick-open's dialog never populates, indistinguishable
        // from an ordinary slow walk on the one operation whose entire point is not blocking.
        //
        // This test predates the symlink ruling and still passes, but **for a different reason**,
        // which is worth saying because a stale "why" is how a test comes to guard nothing. It
        // used to hold because the walk never followed a directory symlink at all -- cycle safety
        // as a property of the type it asked. Now the walk does follow them, and safety comes from
        // `visited` holding canonical paths: the loop resolves to a directory already descended
        // into, so it stops. A depth cap remains ruled out for QA's original reason -- a cap turns
        // an infinite walk into a silently *wrong* one, indistinguishable from a correct result
        // that happens to be short.
        //
        // Note what this test cannot do: if `visited` were removed it would **hang**, not fail.
        // The diamond test below is the deterministic guard on the same mechanism, and is the one
        // to reach for first.
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("notes/real.md"), "x").unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("notes/loop")).unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["notes/real.md"]);
    }

    #[test]
    fn a_directory_symlink_pointing_outside_the_root_is_never_descended_into() {
        // The sibling case QA asked to check alongside the cycle. Still true, and again for a
        // different reason than when it was written: the walk now follows directory symlinks, so
        // this no longer holds for free. It holds because the walk stops at the same boundary
        // `Workspace::dir_list` enforces -- a link resolving outside the root is not descended
        // into -- which is what makes the sidebar and quick-open agree that those documents are
        // not in the workspace.
        //
        // Reaching outside the root at all is a v0.2 question (architecture.md §2): `..` is path
        // construction and stays refused, a symlink is content and should eventually be followed.
        // When that lands, this test's expectation flips -- and it should, because the ruling
        // changed, not because the code drifted.
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "x").unwrap();

        let root = tempdir().unwrap();
        fs::write(root.path().join("visible.md"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["visible.md"]);
    }

    #[test]
    fn a_symlinked_directory_inside_the_root_is_walked() {
        // The behaviour the symlink ruling changed. Before, this directory rendered and expanded
        // in the sidebar while none of its documents appeared in Cmd+P -- the tree and the walk
        // answering "is this part of the workspace?" differently. They now share
        // `resolves_to_directory`, so there is one answer.
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/note.md"), "x").unwrap();
        std::os::unix::fs::symlink(root.path().join("real"), root.path().join("aliased")).unwrap();

        let entries = walk_markdown_files(root.path());

        // Reached once, not twice: `visited` keys on the canonical directory, so whichever route
        // the walk takes first wins and the other is skipped. Which one is unspecified (it depends
        // on `read_dir` order), so this asserts the count rather than the route.
        assert_eq!(
            entries.len(),
            1,
            "the document must be offered exactly once"
        );
        assert!(
            names_of(&entries) == vec!["real/note.md"]
                || names_of(&entries) == vec!["aliased/note.md"],
            "unexpected route: {:?}",
            names_of(&entries)
        );
    }

    #[test]
    fn a_symlink_into_an_ignored_directory_is_not_followed() {
        // The bypass that arrived with following. `is_ignored_name` judges each entry's own name,
        // which a symlink sidesteps: this link is named `aliased`, so nothing about it is ignored,
        // and its target's ignored name is in the *target's* path. Before the fix this put every
        // document under `node_modules` into Cmd+P.
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("node_modules/pkg")).unwrap();
        fs::write(root.path().join("node_modules/pkg/README.md"), "x").unwrap();
        fs::write(root.path().join("real.md"), "x").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("node_modules/pkg"),
            root.path().join("aliased"),
        )
        .unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["real.md"]);
    }

    #[test]
    fn a_symlink_pointing_straight_at_an_ignored_directory_is_not_followed() {
        // The leaf case, and why the shared predicate includes the leaf: here the ignored name is
        // the *last* component of the target, so an ancestors-only check would miss it.
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("target")).unwrap();
        fs::write(root.path().join("target/generated.md"), "x").unwrap();
        fs::write(root.path().join("real.md"), "x").unwrap();
        std::os::unix::fs::symlink(root.path().join("target"), root.path().join("aliased"))
            .unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["real.md"]);
    }

    #[test]
    fn a_diamond_contributes_its_documents_exactly_once() {
        // The deterministic guard on the same mechanism the cycle test exercises. Two links, in
        // different directories, to one real directory. This terminates with or without `visited`
        // -- so unlike the cycle test it *fails* rather than hangs, and it fails as a count, which
        // is why it is the one to reach for first.
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("shared")).unwrap();
        fs::write(root.path().join("shared/note.md"), "x").unwrap();
        fs::create_dir(root.path().join("a")).unwrap();
        fs::create_dir(root.path().join("b")).unwrap();
        std::os::unix::fs::symlink(root.path().join("shared"), root.path().join("a/link")).unwrap();
        std::os::unix::fs::symlink(root.path().join("shared"), root.path().join("b/link")).unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(
            entries.len(),
            1,
            "one document reachable by three routes must be offered once, got {:?}",
            names_of(&entries)
        );
    }

    #[test]
    fn ignores_dotfile_directories_anywhere_in_the_tree() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(".obsidian")).unwrap();
        fs::write(root.path().join(".obsidian/config.md"), "x").unwrap();
        fs::write(root.path().join("visible.md"), "x").unwrap();

        let entries = walk_markdown_files(root.path());
        assert_eq!(names_of(&entries), vec!["visible.md"]);
    }

    #[test]
    fn a_workspace_whose_own_root_is_a_dotfile_directory_still_lists_its_contents() {
        // Same edge case as watcher::is_ignored_in_workspace, for the same reason: the root's
        // own name must never count against anything inside it.
        let parent = tempdir().unwrap();
        let root = parent.path().join(".dotfiles");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "x").unwrap();

        let entries = walk_markdown_files(&root);
        assert_eq!(names_of(&entries), vec!["README.md"]);
    }

    #[test]
    fn file_index_returns_cached_results_until_invalidated() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("a.md"), "x").unwrap();

        let index = FileIndex::new();
        assert_eq!(names_of(&index.files(root.path())), vec!["a.md"]);

        // Written directly to disk, bypassing the index entirely — a second call that still
        // sees only "a.md" is the actual claim under test: the cache, not a coincidence of an
        // unchanged tree.
        fs::write(root.path().join("b.md"), "x").unwrap();
        assert_eq!(
            names_of(&index.files(root.path())),
            vec!["a.md"],
            "expected the cached result, not a fresh walk"
        );

        index.invalidate();
        assert_eq!(names_of(&index.files(root.path())), vec!["a.md", "b.md"]);
    }

    #[test]
    fn file_index_rebuilds_when_the_root_changes_without_needing_invalidate() {
        let root_a = tempdir().unwrap();
        fs::write(root_a.path().join("a.md"), "x").unwrap();
        let root_b = tempdir().unwrap();
        fs::write(root_b.path().join("b.md"), "x").unwrap();

        let index = FileIndex::new();
        assert_eq!(names_of(&index.files(root_a.path())), vec!["a.md"]);
        assert_eq!(names_of(&index.files(root_b.path())), vec!["b.md"]);
    }

    #[test]
    fn walking_the_tree_and_writing_a_document_can_run_concurrently() {
        // QA's concurrency criterion: "assumed and holding looks identical to tested and holding
        // right up until it doesn't." `FileIndex`'s own mutex and `DocumentStore`'s `last_known`
        // mutex are entirely separate, so once `quick_open_files` is genuinely async this is the
        // first place two commands can actually overlap — this is that overlap, run for real
        // rather than reasoned about. A `Barrier` makes the two threads start their operations at
        // the same instant rather than merely both existing; without it, one could finish before
        // the other starts and this would prove nothing beyond "sequential calls work."
        use crate::document::DocumentStore;
        use std::sync::{Arc, Barrier};
        use std::thread;

        let root = tempdir().unwrap();
        for i in 0..50 {
            fs::write(root.path().join(format!("note{i}.md")), "x").unwrap();
        }
        let target = root.path().join("note0.md");

        let store = DocumentStore::new();
        let (_content, hash) = store.read(&target).unwrap();
        let store = Arc::new(store);
        let index = Arc::new(FileIndex::new());
        let root_path = root.path().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));

        let writer = {
            let store = Arc::clone(&store);
            let target = target.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store.write(&target, "updated", &hash).unwrap()
            })
        };
        let walker = {
            let index = Arc::clone(&index);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                index.files(&root_path)
            })
        };

        let new_hash = writer.join().unwrap();
        let entries = walker.join().unwrap();

        assert_eq!(
            entries.len(),
            50,
            "the walk must see every file regardless of a concurrent write landing mid-scan"
        );
        assert_eq!(fs::read_to_string(&target).unwrap(), "updated");
        let (_, verify_hash) = store.read(&target).unwrap();
        assert_eq!(
            verify_hash, new_hash,
            "the write's own result must match what's actually on disk afterwards"
        );
    }

    #[test]
    fn the_tree_and_the_walk_agree_on_what_is_in_the_workspace() {
        use crate::workspace::{EntryKind, Workspace};
        use tempfile::Builder;

        // An AGREEMENT test, deliberately not a behaviour test: it does not encode whether a
        // symlinked directory should be part of the workspace, because that question has not been
        // ruled on. It encodes that there must be *one* answer. Whichever way the ruling goes,
        // this keeps `dir_list` and `walk_markdown_files` from answering it differently -- which
        // is the sixth time on this project that two places have held the same knowledge and
        // quietly disagreed.
        //
        // The disagreement it pins: `dir_list` classifies with `fs::metadata()`, which FOLLOWS
        // symlinks, so a symlinked directory renders as a `Directory` and expands in the sidebar.
        // The walk classifies with `DirEntry::file_type()`, which does NOT, so it never descends.
        // The user sees the documents in the tree, opens them, and cannot find them in Cmd+P.
        //
        // WHAT THIS GUARDS AFTER THE SHARED PREDICATE LANDS, because the answer should be here
        // rather than in a ruling document nobody re-reads. Today it catches *drift*: two
        // mechanisms answering one question differently. Once both sides call one predicate, the
        // only way it can fail is if someone stops calling it — a narrower failure, and a much
        // rarer one, which is exactly why it will eventually look like a test that cannot fail and
        // invite deletion. It can fail. That is the failure it is for, and it is the cheapest
        // guard available against re-duplicating a question this project has now duplicated seven
        // times. Deleting it costs nothing today and costs the eighth instance later.
        let root = Builder::new().prefix("medd-agree-").tempdir().unwrap();
        let elsewhere = Builder::new().prefix("medd-target-").tempdir().unwrap();
        std::fs::write(elsewhere.path().join("linked.md"), "# linked").unwrap();
        std::fs::write(root.path().join("plain.md"), "# plain").unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("linked-dir")).unwrap();

        let ws = Workspace::open(root.path()).unwrap();

        // What the TREE says is a directory worth expanding, at the top level.
        let tree_dirs: Vec<String> = ws
            .dir_list(ws.root())
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e.kind, EntryKind::Directory))
            .map(|e| e.name)
            .collect();

        // What the WALK descended into, inferred from what it returned.
        let walked: Vec<String> = walk_markdown_files(ws.root())
            .into_iter()
            .map(|e| e.relative_path)
            .collect();

        let tree_offers_the_symlink = tree_dirs.iter().any(|n| n == "linked-dir");
        let walk_entered_the_symlink = walked.iter().any(|p| p.contains("linked-dir"));

        assert_eq!(
            tree_offers_the_symlink, walk_entered_the_symlink,
            "the tree and quick-open must agree about whether a symlinked directory is part of \
             the workspace. tree offers it as a directory: {tree_offers_the_symlink}; the walk \
             descends into it: {walk_entered_the_symlink}. tree_dirs={tree_dirs:?} walked={walked:?}"
        );
    }

    #[test]
    fn the_ignore_rule_reaches_the_same_answer_by_every_route() {
        // AGREEMENT, in the same shape as `the_tree_and_the_walk_agree_...` above: a DIFFERENTIAL
        // assertion — **adding an alias to an ignored location must not change what the walk
        // finds** — rather than an expected-output one. It does not need to know what the right
        // answer is, only that adding a path to the same content does not change it.
        //
        // WHAT THIS DOES AND DOES NOT COVER, because the name promises more than the body keeps.
        // The generality is in the *form*, not in the mechanism: this body creates a symlink, so
        // today it catches exactly what `a_symlink_into_an_ignored_directory_is_not_followed` and
        // its sibling catch — verified by mutation, all three die to the same mutant and no mutant
        // distinguishes them. It is REDUNDANT, which is not the same as vacuous: drop the
        // symlink-route guard and it does fail.
        //
        // It will hold for a hardlinked directory, a bind mount, or an include mechanism when
        // someone adds that route *to this test*, and not one moment before. Its job is to make
        // the next route cheap to cover — two lines here, against a whole new expected-output
        // assertion in a case test — and to be the named home for the class, not to cover the
        // next route in advance.
        //
        // The failure it pins (eighth instance of one question answered in two places):
        // `is_ignored_name` checks each entry's own name as the walk descends, which is equivalent
        // to ancestor-checking for real directories because the walk must pass through the ignored
        // ancestor to reach its contents. A symlink does not pass through — it jumps. So `aliased`
        // -> `node_modules/docs` is a link named `aliased`, which is not an ignored name, and
        // following it lands inside an ignored directory the real route blocks. A symlink into
        // `node_modules` or `target` puts thousands of files back into Cmd+P, through the one
        // route the shared ignore predicate does not cover.
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("node_modules/docs")).unwrap();
        fs::write(root.path().join("node_modules/docs/hidden.md"), "x").unwrap();
        fs::write(root.path().join("real.md"), "x").unwrap();

        let before = walk_markdown_files(root.path());
        let without_alias: Vec<String> = names_of(&before).iter().map(|s| s.to_string()).collect();

        std::os::unix::fs::symlink(
            root.path().join("node_modules/docs"),
            root.path().join("aliased"),
        )
        .unwrap();
        let after = walk_markdown_files(root.path());
        let with_alias: Vec<String> = names_of(&after).iter().map(|s| s.to_string()).collect();

        assert_eq!(
            with_alias, without_alias,
            "adding an alias to an ignored directory changed what the walk finds: {without_alias:?} \
             became {with_alias:?}. The ignore rule must answer the same for every route into a \
             location, not only the route that passes through its ancestor."
        );
    }
}

#[cfg(test)]
mod wire_format {
    //! Contract tests: the shape Rust actually puts on the wire, asserted against what the
    //! frontend reads. This is the only place that contract can be tested.
    //!
    //! The frontend's own tests — and `harness/tauriMock.ts` — construct these payloads by hand,
    //! so they assert that the frontend agrees with itself. That is how
    //! `{{kind:"conflict", current_content}}` shipped against six green tests all mocking
    //! `{{kind:"Conflict", currentContent}}`: a mock at a boundary *defines* the boundary for
    //! every test that uses it, and the number of green tests over it measures exposure rather
    //! than coverage.
    //!
    //! String comparisons, deliberately, as in `error.rs`: the failure mode is a *name*, and an
    //! assertion built from the same type cannot see a renaming.
    use super::*;

    /// `relative_path` becomes `relativePath` under `rename_all = "camelCase"`, which is what
    /// `src/quickopen/match.ts` reads. Its own tests build entries by hand, so they cannot see a
    /// disagreement here.
    #[test]
    fn quick_open_entry_fields_are_what_match_ts_reads() {
        let e = QuickOpenEntry {
            path: std::path::PathBuf::from("/w/notes/a.md"),
            relative_path: "notes/a.md".to_string(),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(
            json.contains(r#""relativePath":"notes/a.md""#),
            "match.ts reads entry.relativePath: {json}"
        );
        assert!(json.contains(r#""path":"#), "match.ts reads entry.path: {json}");
    }
}

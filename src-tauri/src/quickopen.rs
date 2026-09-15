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

use crate::workspace::is_ignored_name;

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
/// `node_modules` — are checked against each entry's own name as it's found, exactly like
/// `dir_list`'s per-level dotfile hiding, which is what makes a workspace whose own root is a
/// dotfile directory (`~/.dotfiles`) work correctly for free: the root's own name is never itself
/// checked, only the names of things found inside it. Directory symlinks are never followed
/// (`DirEntry::file_type` reports the link's own type, not its target's), which sidesteps a
/// symlink cycle turning this into an infinite walk; a symlinked `.md` *file* is still listed.
fn walk_markdown_files(root: &Path) -> Vec<QuickOpenEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];

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
            let Ok(file_type) = entry.file_type() else {
                continue;
            };

            if file_type.is_dir() {
                stack.push(path);
            } else if name_str.to_lowercase().ends_with(".md") {
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
        // `walk_markdown_files` never follows directory symlinks at all (see its own doc comment:
        // `DirEntry::file_type` reports the link's own type, not its target's) -- cycle safety by
        // never traversing a link's target, not by a depth cap, which QA is right to rule out: a
        // cap converts an infinite walk into a silently *wrong* one, indistinguishable from a
        // correct result that happens to be short.
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("notes")).unwrap();
        fs::write(root.path().join("notes/real.md"), "x").unwrap();
        std::os::unix::fs::symlink(root.path(), root.path().join("notes/loop")).unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["notes/real.md"]);
    }

    #[test]
    fn a_directory_symlink_pointing_outside_the_root_is_never_descended_into() {
        // The sibling case QA asked to check alongside the cycle: `workspace::classify` already
        // resolves before comparing against the root, so an escaping symlink can't disguise
        // itself as root-relative there. This is the same property for the walk, and it needs no
        // extra code to hold -- `walk_markdown_files` never follows *any* directory symlink,
        // inward-pointing or outward, so an escape hatch here would be a regression, not a
        // missing feature. Demonstrated rather than assumed: a real directory (with a real .md
        // file in it) sits outside the workspace, linked from inside it.
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "x").unwrap();

        let root = tempdir().unwrap();
        fs::write(root.path().join("visible.md"), "x").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();

        let entries = walk_markdown_files(root.path());

        assert_eq!(names_of(&entries), vec!["visible.md"]);
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
}

//! Root, tree enumeration, path classification (architecture.md §2, plan-v0.1.md increment 3).
//!
//! This is where the security boundary for later increments lives: `classify()` decides whether
//! a path is inside the workspace root, outside it (a loose file, D-15), or a directory —
//! and getting that wrong for a `..`-escape or an out-of-root symlink is exactly what would later
//! let the render pipeline's asset protocol (increment 5) serve files it shouldn't. Nothing calls
//! `classify()` yet — no command needs the distinction until increment 5 or increment 10's
//! `route_open()` — so it's `#[allow(dead_code)]` down at its own definition, same pattern as
//! `document.rs`'s `write()`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::MeddError;

/// An open workspace: just its canonical root. Canonicalises independently of `document.rs` —
/// each module stays correct without relying on the other having prepared its inputs.
pub struct Workspace {
    root: PathBuf,
}

/// Where a path sits relative to the open workspace. `Directory` takes priority over the other
/// two regardless of location: a directory is never a document to open, whether it's inside the
/// root or not (this is the split `routing.rs` will reuse in increment 10 to decide whether an
/// incoming path opens a tab or replaces the workspace).
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PathClass {
    /// A file inside the workspace root, carrying its path relative to that root.
    RootRelative(PathBuf),
    /// A file outside the workspace root — a loose document (D-15). Also what a path resolves to
    /// when it escapes the root via `..` or a symlink pointing elsewhere: canonicalisation
    /// happens before this comparison, so neither trick disguises itself as root-relative.
    Loose,
    /// A directory, wherever it sits. Not a document; opening one means opening a workspace.
    Directory,
}

impl Workspace {
    /// Opens `root` as the workspace, canonicalising it (so a symlinked root behaves like any
    /// other canonical path) and rejecting anything that isn't a directory.
    pub fn open(root: &Path) -> Result<Self, MeddError> {
        let canonical = root.canonicalize().map_err(|e| MeddError::io(root, e))?;
        if !canonical.is_dir() {
            return Err(MeddError::Io {
                path: canonical,
                message: "not a directory".to_string(),
            });
        }
        Ok(Workspace { root: canonical })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Classifies `path` against this workspace's root. Canonicalises first, so `..` segments
    /// and symlinks are resolved before the root comparison — never trust the text of the path
    /// the caller handed in.
    #[allow(dead_code)]
    pub fn classify(&self, path: &Path) -> Result<PathClass, MeddError> {
        let canonical = path.canonicalize().map_err(|e| MeddError::io(path, e))?;

        if canonical.is_dir() {
            return Ok(PathClass::Directory);
        }

        match canonical.strip_prefix(&self.root) {
            Ok(relative) => Ok(PathClass::RootRelative(relative.to_path_buf())),
            Err(_) => Ok(PathClass::Loose),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Directory,
    Markdown,
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    pub name: String,
    pub path: PathBuf,
    pub kind: EntryKind,
}

/// Lists one level of `dir` — never recurses, so an enormous workspace never gets walked before
/// the window appears (N-2). Hidden entries (dotfiles, `.git`, editor state directories) are
/// omitted by default: see the increment-3 report for why.
pub fn dir_list(dir: &Path) -> Result<Vec<TreeEntry>, MeddError> {
    let canonical_dir = dir.canonicalize().map_err(|e| MeddError::io(dir, e))?;
    if !canonical_dir.is_dir() {
        return Err(MeddError::Io {
            path: canonical_dir,
            message: "not a directory".to_string(),
        });
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(&canonical_dir).map_err(|e| MeddError::io(&canonical_dir, e))? {
        let Ok(entry) = entry else {
            // A single unreadable entry (e.g. a permissions race) shouldn't fail the listing.
            continue;
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with('.') {
            continue;
        }

        let path = canonical_dir.join(&name);
        let kind = match fs::metadata(&path) {
            Ok(meta) if meta.is_dir() => EntryKind::Directory,
            Ok(_) if name_str.to_lowercase().ends_with(".md") => EntryKind::Markdown,
            // Includes a dangling symlink: metadata() follows the link and fails, so it's shown
            // but inert rather than dropped from the listing or treated as a crash.
            _ => EntryKind::Other,
        };

        entries.push(TreeEntry {
            name: name_str.into_owned(),
            path,
            kind,
        });
    }

    entries.sort_by(|a, b| {
        let a_is_dir = a.kind == EntryKind::Directory;
        let b_is_dir = b.kind == EntryKind::Directory;
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn classify_root_relative_file() {
        let root = tempdir().unwrap();
        let file = root.path().join("note.md");
        fs::write(&file, "hello").unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        assert_eq!(
            ws.classify(&file).unwrap(),
            PathClass::RootRelative(PathBuf::from("note.md"))
        );
    }

    #[test]
    fn classify_root_relative_nested_file() {
        let root = tempdir().unwrap();
        fs::create_dir(root.path().join("sub")).unwrap();
        let file = root.path().join("sub").join("note.md");
        fs::write(&file, "hello").unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        assert_eq!(
            ws.classify(&file).unwrap(),
            PathClass::RootRelative(PathBuf::from("sub/note.md"))
        );
    }

    #[test]
    fn classify_loose_file_outside_root() {
        let root = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        let file = elsewhere.path().join("note.md");
        fs::write(&file, "hello").unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        assert_eq!(ws.classify(&file).unwrap(), PathClass::Loose);
    }

    #[test]
    fn classify_dot_dot_escape_is_loose_not_root_relative() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let outside = parent.path().join("outside.md");
        fs::write(&outside, "hello").unwrap();

        let ws = Workspace::open(&root).unwrap();
        // Built from the workspace's own canonical root (not the tempdir's raw path — macOS
        // temp directories are themselves reached through a /var -> /private/var symlink, which
        // would confound this test with a different discrepancy than the one under test), so the
        // only thing that could make this misclassify is the `..` segment itself: textually
        // nested under the root, but resolving outside it once canonicalised.
        let escaping = ws.root().join("../outside.md");
        assert_eq!(ws.classify(&escaping).unwrap(), PathClass::Loose);
    }

    #[test]
    fn classify_symlink_pointing_outside_root_is_loose() {
        let root = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();
        let real = elsewhere.path().join("real.md");
        fs::write(&real, "hello").unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        // The link lives directly under the workspace's own canonical root, so its path is
        // already textually root-prefixed — a naive strip_prefix on the raw path would call this
        // root-relative. Only resolving the symlink before comparing reveals it points outside.
        let link = ws.root().join("link.md");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(ws.classify(&link).unwrap(), PathClass::Loose);
    }

    #[test]
    fn classify_directory_inside_root() {
        let root = tempdir().unwrap();
        let sub = root.path().join("sub");
        fs::create_dir(&sub).unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        assert_eq!(ws.classify(&sub).unwrap(), PathClass::Directory);
    }

    #[test]
    fn classify_directory_outside_root_is_directory_not_loose() {
        let root = tempdir().unwrap();
        let elsewhere = tempdir().unwrap();

        let ws = Workspace::open(root.path()).unwrap();
        assert_eq!(ws.classify(elsewhere.path()).unwrap(), PathClass::Directory);
    }

    #[test]
    fn classify_nonexistent_path_errors_sanely() {
        let root = tempdir().unwrap();
        let missing = root.path().join("nope.md");

        let ws = Workspace::open(root.path()).unwrap();
        let result = ws.classify(&missing);
        assert!(matches!(result, Err(MeddError::Io { .. })));
    }

    #[test]
    fn open_rejects_a_file_as_workspace_root() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("note.md");
        fs::write(&file, "hello").unwrap();

        let result = Workspace::open(&file);
        assert!(matches!(result, Err(MeddError::Io { .. })));
    }

    #[test]
    fn open_rejects_a_nonexistent_root() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope");

        let result = Workspace::open(&missing);
        assert!(matches!(result, Err(MeddError::Io { .. })));
    }

    #[test]
    fn open_resolves_a_symlinked_root_to_its_real_directory() {
        let real_dir = tempdir().unwrap();
        let parent = tempdir().unwrap();
        let link = parent.path().join("workspace-link");
        std::os::unix::fs::symlink(real_dir.path(), &link).unwrap();

        let ws = Workspace::open(&link).unwrap();
        assert_eq!(ws.root(), real_dir.path().canonicalize().unwrap());
    }

    #[test]
    fn dir_list_returns_one_level_only() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("top.md"), "hello").unwrap();
        let sub = root.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("nested.md"), "hello").unwrap();

        let entries = dir_list(root.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["sub", "top.md"]);
    }

    #[test]
    fn dir_list_hides_dotfiles_by_default() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("visible.md"), "hello").unwrap();
        fs::write(root.path().join(".hidden.md"), "hello").unwrap();
        fs::create_dir(root.path().join(".git")).unwrap();

        let entries = dir_list(root.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["visible.md"]);
    }

    #[test]
    fn dir_list_classifies_markdown_directories_and_other() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("doc.md"), "hello").unwrap();
        fs::write(root.path().join("image.png"), "hello").unwrap();
        fs::create_dir(root.path().join("folder")).unwrap();

        let entries = dir_list(root.path()).unwrap();
        let kind_of = |name: &str| entries.iter().find(|e| e.name == name).unwrap().kind;
        assert_eq!(kind_of("doc.md"), EntryKind::Markdown);
        assert_eq!(kind_of("image.png"), EntryKind::Other);
        assert_eq!(kind_of("folder"), EntryKind::Directory);
    }

    #[test]
    fn dir_list_sorts_directories_before_files_alphabetically() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("b.md"), "hello").unwrap();
        fs::write(root.path().join("a.md"), "hello").unwrap();
        fs::create_dir(root.path().join("z-dir")).unwrap();
        fs::create_dir(root.path().join("a-dir")).unwrap();

        let entries = dir_list(root.path()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["a-dir", "z-dir", "a.md", "b.md"]);
    }

    #[test]
    fn dir_list_shows_a_dangling_symlink_as_inert_rather_than_failing() {
        let root = tempdir().unwrap();
        let target = root.path().join("gone.md");
        fs::write(&target, "hello").unwrap();
        let link = root.path().join("dangling.md");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        fs::remove_file(&target).unwrap();

        let entries = dir_list(root.path()).unwrap();
        let entry = entries.iter().find(|e| e.name == "dangling.md").unwrap();
        assert_eq!(entry.kind, EntryKind::Other);
    }

    #[test]
    fn dir_list_rejects_a_file_path() {
        let root = tempdir().unwrap();
        let file = root.path().join("note.md");
        fs::write(&file, "hello").unwrap();

        assert!(matches!(dir_list(&file), Err(MeddError::Io { .. })));
    }
}

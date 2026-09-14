//! The `#[tauri::command]` surface (architecture.md §4). This is the only module in the
//! workspace that knows Tauri's command macros exist — everything else stays plain Rust,
//! directly unit-tested, and commands.rs stays a thin translation layer over it.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

use crate::document::{ContentHash, DocumentStore};
use crate::error::MeddError;
use crate::workspace::{self, TreeEntry, Workspace};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub root: PathBuf,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadResult {
    pub content: String,
    pub hash: ContentHash,
}

/// Native folder picker (D-14). `None` if the user cancels.
#[tauri::command]
pub fn workspace_pick(app: tauri::AppHandle) -> Option<PathBuf> {
    let file_path = app.dialog().file().blocking_pick_folder()?;
    file_path.into_path().ok()
}

/// Sets the workspace root, replacing whatever was open before. No watching yet — `watcher.rs`
/// is increment 7.
#[tauri::command]
pub fn workspace_open(
    path: PathBuf,
    workspace: State<'_, Mutex<Option<Workspace>>>,
) -> Result<WorkspaceInfo, MeddError> {
    let ws = Workspace::open(&path)?;
    let info = WorkspaceInfo {
        root: ws.root().to_path_buf(),
        name: ws
            .root()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ws.root().to_string_lossy().into_owned()),
    };
    *workspace.lock().unwrap() = Some(ws);
    Ok(info)
}

/// One level of the tree, lazily (plan-v0.1.md increment 3) — never walks the whole workspace.
#[tauri::command]
pub fn dir_list(path: PathBuf) -> Result<Vec<TreeEntry>, MeddError> {
    workspace::dir_list(&path)
}

/// Opens a document; begins tracking it for the compare-and-swap write path that lands in
/// increment 7.
#[tauri::command]
pub fn document_read(
    path: PathBuf,
    store: State<'_, DocumentStore>,
) -> Result<ReadResult, MeddError> {
    let (content, hash) = store.read(&path)?;
    Ok(ReadResult { content, hash })
}

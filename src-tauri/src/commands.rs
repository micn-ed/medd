//! The `#[tauri::command]` surface (architecture.md §4). This is the only module in the
//! workspace that knows Tauri's command macros exist — everything else stays plain Rust,
//! directly unit-tested, and commands.rs stays a thin translation layer over it.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::document::{ContentHash, DocumentStore};
use crate::error::MeddError;
use crate::workspace::{PathClass, TreeEntry, Workspace};

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
///
/// Scopes the asset protocol to the new root and revokes it from the old one (plan-v0.1.md §5):
/// "nothing wider" means the allow-list should track the *current* workspace, not accumulate
/// every workspace opened in a session.
#[tauri::command]
pub fn workspace_open(
    app: AppHandle,
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

    let mut guard = workspace.lock().unwrap();
    let scope = app.asset_protocol_scope();
    if let Some(previous) = guard.as_ref() {
        let _ = scope.forbid_directory(previous.root(), true);
    }
    let _ = scope.allow_directory(ws.root(), true);

    *guard = Some(ws);
    Ok(info)
}

/// One level of the tree, lazily (plan-v0.1.md increment 3) — never walks the whole workspace.
/// Scoped to the open workspace's root: nothing legitimate needs to list outside it.
#[tauri::command]
pub fn dir_list(
    path: PathBuf,
    workspace: State<'_, Mutex<Option<Workspace>>>,
) -> Result<Vec<TreeEntry>, MeddError> {
    let guard = workspace.lock().unwrap();
    let ws = guard.as_ref().ok_or_else(|| MeddError::Io {
        path: path.clone(),
        message: "no workspace open".to_string(),
    })?;
    ws.dir_list(&path)
}

/// Opens a document; begins tracking it for the compare-and-swap write path that lands in
/// increment 7.
///
/// If the path is loose (D-15: outside the open workspace, or no workspace open at all) and a
/// workspace *is* open, scopes the asset protocol to that document's own directory so its
/// relative images can resolve — root-relative documents need no extra scoping, since the whole
/// workspace root was already scoped by `workspace_open`. This is the first real caller of
/// `Workspace::classify`, which has sat tested-but-unused since increment 3.
#[tauri::command]
pub fn document_read(
    app: AppHandle,
    path: PathBuf,
    store: State<'_, DocumentStore>,
    workspace: State<'_, Mutex<Option<Workspace>>>,
) -> Result<ReadResult, MeddError> {
    let (content, hash) = store.read(&path)?;

    if let Some(ws) = workspace.lock().unwrap().as_ref() {
        if matches!(ws.classify(&path), Ok(PathClass::Loose)) {
            if let Ok(canonical) = path.canonicalize() {
                if let Some(dir) = canonical.parent() {
                    let _ = app.asset_protocol_scope().allow_directory(dir, false);
                }
            }
        }
    }

    Ok(ReadResult { content, hash })
}

/// Hands an http(s) link to the system browser (R-6). Calls the opener plugin's Rust API
/// directly rather than its own IPC-gated command, same as `workspace_pick` — which means its
/// scope system isn't in play, so the scheme check below is medd's own: without it, a bug
/// upstream of this command (or a compromised renderer) could ask the OS to open an arbitrary
/// local path or a `file://`/custom-scheme URL instead of the http(s) link this command exists
/// for. The render pipeline only ever classifies http(s) links as "external" in the first place,
/// but this command shouldn't rely on that alone.
#[tauri::command]
pub fn open_external(app: AppHandle, url: String) -> Result<(), MeddError> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(MeddError::Io {
            path: PathBuf::from(&url),
            message: "only http(s) URLs may be opened externally".to_string(),
        });
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| MeddError::Io {
            path: PathBuf::new(),
            message: e.to_string(),
        })
}

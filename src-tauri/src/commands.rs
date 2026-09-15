//! The `#[tauri::command]` surface (architecture.md §4). This is the only module in the
//! workspace that knows Tauri's command macros exist — everything else stays plain Rust,
//! directly unit-tested, and commands.rs stays a thin translation layer over it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::document::{ContentHash, DocumentStore, TempSweeper};
use crate::error::MeddError;
use crate::quickopen::{FileIndex, QuickOpenEntry};
use crate::quit::QuitCoordinator;
use crate::watcher::FsWatcher;
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

/// Sets the workspace root, replacing whatever was open before.
///
/// Scopes the asset protocol to the new root and revokes it from the old one (plan-v0.1.md §5):
/// "nothing wider" means the allow-list should track the *current* workspace, not accumulate
/// every workspace opened in a session. Starts watching the new root recursively (architecture.md
/// §6) and stops watching the old one, for the same reason.
#[tauri::command]
pub fn workspace_open(
    app: AppHandle,
    path: PathBuf,
    workspace: State<'_, Mutex<Option<Workspace>>>,
    watcher: State<'_, Mutex<FsWatcher>>,
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

    // The previous root is copied out of the guard by this one statement, not an `if let` on the
    // lock — an `if let` scrutinee's temporary lives for the whole arm on edition 2021, which is
    // exactly the trap that used to hold this same guard across `rescope_workspace` below. A plain
    // `let` binding's temporary drops at the end of the statement, so the guard is gone before
    // `rescope_workspace` — which never receives one — runs the asset-scope and FSEvents calls.
    let previous_root = workspace
        .lock()
        .unwrap()
        .as_ref()
        .map(|w| w.root().to_path_buf());

    rescope_workspace(&app, &watcher, previous_root, info.root.clone());

    *workspace.lock().unwrap() = Some(ws);
    Ok(info)
}

/// Points the asset-protocol scope and the FSEvents watch at `new_root`, releasing `old_root`
/// first if there was one. Takes both roots as owned `PathBuf`s, deliberately, not `&Path` — a
/// borrow only proves the guard isn't held *today*; an owned value proves it structurally, because
/// nothing can borrow a `MutexGuard` and hand out something that outlives it. `&Path` is the more
/// idiomatic signature and the one a reviewer's eye slides past, which is exactly why it's the
/// wrong choice here: it would leave this call site free to go back to passing a reference straight
/// out of the guard with nothing failing, restoring the hazard invisibly. The one clone this costs
/// per workspace switch is the point, not overhead to claw back.
///
/// Lock order here is workspace-then-watcher (the workspace guard above is already released by
/// the time this takes the watcher lock), matching `document_read`'s — the only other site that
/// takes both. Keeping that order consistent is what rules out an AB/BA deadlock between them; a
/// third site taking both locks should follow the same order rather than inventing its own.
fn rescope_workspace(
    app: &AppHandle,
    watcher: &Mutex<FsWatcher>,
    old_root: Option<PathBuf>,
    new_root: PathBuf,
) {
    let scope = app.asset_protocol_scope();
    let mut fs_watcher = watcher.lock().unwrap();
    if let Some(old_root) = &old_root {
        let _ = scope.forbid_directory(old_root, true);
        fs_watcher.unwatch(old_root);
    }
    let _ = scope.allow_directory(&new_root, true);
    let _ = fs_watcher.watch_recursive(&new_root);
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

/// Opens a document; begins tracking it for the compare-and-swap write path (`document_write`,
/// below).
///
/// If the path is loose (D-15: outside the open workspace, or no workspace open at all) and a
/// workspace *is* open, scopes the asset protocol to that document's own directory so its
/// relative images can resolve, and starts a non-recursive watch on that same directory
/// (architecture.md §6) — root-relative documents need neither: the whole workspace root is
/// already scoped and watched by `workspace_open`. This is the first real caller of
/// `Workspace::classify`, which sat tested-but-unused from increment 3 to increment 5.
#[tauri::command]
pub fn document_read(
    app: AppHandle,
    path: PathBuf,
    store: State<'_, DocumentStore>,
    workspace: State<'_, Mutex<Option<Workspace>>>,
    watcher: State<'_, Mutex<FsWatcher>>,
) -> Result<ReadResult, MeddError> {
    let (content, hash) = store.read(&path)?;

    // `is_loose` is a plain `let`, not an `if let` on the lock guard itself — an `if let`
    // scrutinee's temporary lives for the whole arm on edition 2021 (src-tauri is 2021), which is
    // exactly what used to hold this guard across the canonicalize/scope/watch calls below. A
    // plain `let` binding's temporary drops at the end of its own statement, so nothing is held by
    // the time `scope_and_watch_loose_document` — which never receives a lock — runs.
    let is_loose = workspace
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|ws| matches!(ws.classify(&path), Ok(PathClass::Loose)));

    if is_loose {
        if let Ok(canonical) = path.canonicalize() {
            if let Some(dir) = canonical.parent() {
                scope_and_watch_loose_document(&app, &watcher, dir.to_path_buf());
            }
        }
    }

    Ok(ReadResult { content, hash })
}

/// Scopes the asset protocol to `dir` and starts a non-recursive watch on it, for a loose
/// document's own directory. Takes `dir` as an owned `PathBuf`, not `&Path`, for the same reason
/// as `rescope_workspace`: an owned value can't have been borrowed out of a `MutexGuard`, so this
/// signature is what makes "never called while the workspace lock is held" a property the compiler
/// enforces rather than one this comment merely asserts.
///
/// Lock order: this only ever takes the watcher lock, and only after `document_read` has already
/// released the workspace lock — the same workspace-then-watcher order as `rescope_workspace`,
/// just with the first half finished before this function is even called.
fn scope_and_watch_loose_document(app: &AppHandle, watcher: &Mutex<FsWatcher>, dir: PathBuf) {
    let _ = app.asset_protocol_scope().allow_directory(&dir, false);
    let _ = watcher.lock().unwrap().watch_non_recursive(&dir);
}

/// Compare-and-swap write (architecture.md §3), called by the frontend's autosave debounce
/// (`doc/`, plan-v0.1.md increment 7). A thin wrapper — every actual safety property (atomicity,
/// the hash re-check, recording the new hash before the write's own watcher event can arrive)
/// lives in `DocumentStore::write`, tested independently of any command since increment 2.
#[tauri::command]
pub fn document_write(
    path: PathBuf,
    content: String,
    expected_hash: ContentHash,
    store: State<'_, DocumentStore>,
    sweeper: State<'_, TempSweeper>,
) -> Result<ContentHash, MeddError> {
    let hash = store.write(&path, &content, &expected_hash)?;

    // Litter is created by writes, so the set of directories that can hold an abandoned staging
    // file is exactly the set medd has written to — which makes here the narrowest place that
    // still covers all of it, and keeps `dir_list` a read that only reads. Deliberately *after*
    // `store.write` returns, so the store's mutex is released first: that one lock serialises
    // every read and write across every open document, and a `read_dir` plus a lock attempt per
    // entry inside it would put directory-scan latency on every write in every tab.
    if let Some(dir) = path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        sweeper.sweep_once(&dir);
    }

    Ok(hash)
}

/// Every `.md` file in the open workspace, for quick-open (Cmd+P, plan-v0.1.md increment 9, W-5).
/// Deliberately the one command that walks the whole tree — `dir_list` stays one-level-at-a-time
/// for N-2.
///
/// `#[tauri::command(async)]` is load-bearing, not a style choice: a plain command is dispatched
/// INLINE on the thread that receives the IPC message — the main/UI thread on macOS — with no
/// `spawn_blocking` involved (verified by reading `tauri-macros`' `body_blocking` codegen directly,
/// not assumed from the docs). Without `async` here, an uncached first walk of a large workspace
/// would block the whole app, not just the dialog, for exactly as long as the walk this command
/// exists to keep off the UI thread. `FileIndex` still caches the result per workspace root so
/// repeat calls are cheap regardless.
///
/// The workspace root is copied out of the guard by a plain `let` before `index.files` runs, the
/// same shape as `rescope_workspace`/`scope_and_watch_loose_document`: now that this command is
/// genuinely async and can overlap with `dir_list`/`document_read`/`workspace_open`, holding the
/// workspace lock for the walk's duration would block every one of them for as long as the walk
/// takes — the exact hazard those two fixes exist to prevent, just newly reachable here too.
#[tauri::command(async)]
pub fn quick_open_files(
    workspace: State<'_, Mutex<Option<Workspace>>>,
    index: State<'_, FileIndex>,
) -> Result<Vec<QuickOpenEntry>, MeddError> {
    let root = workspace
        .lock()
        .unwrap()
        .as_ref()
        .map(|ws| ws.root().to_path_buf())
        .ok_or_else(|| MeddError::Io {
            path: PathBuf::new(),
            message: "no workspace open".to_string(),
        })?;
    Ok(index.files(&root))
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

/// The frontend's signal that it has flushed every pending autosave and awaited quiescence
/// (`doc/doc.ts`'s `flushAll` + `waitForAllQuiescent`), in response to `app:before-quit`
/// (plan-v0.1.md's fifth blocker). Only ever *shortens* the wait a `RunEvent::ExitRequested`
/// handler in `main.rs` is already bounding on its own timer — this command has no power to make
/// medd wait any longer than that bound allows, only to end the wait early once there is nothing
/// left to lose by exiting now.
#[tauri::command]
pub fn quit_ready(coordinator: State<'_, QuitCoordinator>) {
    coordinator.signal_ready();
}

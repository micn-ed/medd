//! The `#[tauri::command]` surface (architecture.md §4). This is the only module in the
//! workspace that knows Tauri's command macros exist — everything else stays plain Rust,
//! directly unit-tested, and commands.rs stays a thin translation layer over it.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::document::{ContentHash, DocumentStore, TempSweeper};
use crate::error::MeddError;
use crate::quickopen::{FileIndex, QuickOpenEntry};
use crate::quit::QuitCoordinator;
use crate::routing::{PendingOpens, RouteTarget};
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
///
/// **`async` is load-bearing, not decoration.** A plain `#[tauri::command]` is
/// `ExecutionContext::Blocking`, which runs the handler inline on the thread dispatching the IPC
/// message — the main thread. `blocking_pick_folder` then waits there for a panel that needs the
/// main run loop to pump in order to appear, so the app deadlocks the moment the user clicks
/// *Open Folder…* and never recovers. The plugin documents exactly this: its non-blocking
/// `pick_folder` "should be used when running on the main thread to avoid deadlocks with the event
/// loop", and the blocking variant is "for use in other contexts". `async` makes this one of those
/// other contexts by moving it to the threadpool, leaving the main thread free to run the panel.
///
/// This takes no locks, so it is safe under the concurrency that `async` admits (architecture.md
/// §2: no lock is held across a filesystem or OS call).
#[tauri::command(async)]
pub fn workspace_pick(app: tauri::AppHandle) -> Option<PathBuf> {
    let file_path = app.dialog().file().blocking_pick_folder()?;
    file_path.into_path().ok()
}

/// Sets the workspace root, replacing whatever was open before.
///
/// Grants the asset protocol the new root, and **revokes nothing** — see `rescope_workspace`'s
/// body for why, and D-17 for the decision. This comment used to say the opposite, and said it
/// here, on the public command, while the explanation of the absence sits one call down in the
/// helper. That ordering is a trap: reading top-down you meet a documented intent to revoke,
/// find no revoke in the body, and conclude one was lost — so the obvious repair is to re-add
/// `forbid_directory`, which is the defect (`9c72059`). **The scope accumulates every workspace
/// root opened this session, by design and not by omission**, because `FsScope` has no revocation
/// to offer; `asset_scope_revocation`'s two tests pin both halves.
///
/// Starts watching the new root recursively and stops watching the old one (architecture.md §6).
/// The watch half genuinely *can* be released — `unwatch` is real, idempotent, and restores prior
/// state — which is exactly why the two halves no longer share a lifecycle.
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
fn rescope_workspace<R: Runtime>(
    app: &AppHandle<R>,
    watcher: &Mutex<FsWatcher>,
    old_root: Option<PathBuf>,
    new_root: PathBuf,
) {
    let scope = app.asset_protocol_scope();
    let mut fs_watcher = watcher.lock().unwrap();
    if let Some(old_root) = &old_root {
        // **No `forbid_directory` here, deliberately.** It reads as the counterpart to the grant
        // below and is not one: `FsScope`'s four mutators all push onto monotonic sets -- nothing
        // in the type removes from either -- and `is_allowed` checks the forbidden patterns first,
        // returning `false` on a match without ever consulting the allow list. A forbid is
        // therefore permanent and outranks every later grant.
        //
        // Calling it here made revisiting a workspace break it for the rest of the process: open
        // `/a`, switch to `/c`, switch back to `/a`, and the re-grant on the next line is
        // outranked by the standing forbid, so every image in that folder stays unreadable until
        // medd restarts. Under D-4 and I-3 -- an editor open for days -- returning to a folder is
        // ordinary use. `asset_scope_revocation` pins this.
        //
        // The consequence, recorded rather than hidden: the old root stays readable. That is the
        // scope being monotonic, which it already was; see architecture.md §11 for the bound and
        // for why the CSP, not this scope, is what keeps a wider read from becoming an exfiltration
        // path. A boundary that tightens on close cannot be built from this API at all, which is a
        // specification question and not something this call site can fix by being more careful.
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
pub fn document_read<R: Runtime>(
    app: AppHandle<R>,
    path: PathBuf,
    store: State<'_, DocumentStore>,
    workspace: State<'_, Mutex<Option<Workspace>>>,
    watcher: State<'_, Mutex<FsWatcher>>,
) -> Result<ReadResult, MeddError> {
    let (content, hash) = store.read(&path)?;

    // **Two questions, deliberately separate, because they have different answers and different
    // costs of being wrong** (D-17, architecture.md §6 and §11). They used to be one call, and the
    // generous half decided for both.
    //
    // *What do we watch?* Whatever is not already watched. Missing one means a document silently
    // stops reloading, so this half is generous. Asking `covers` rather than asking the workspace
    // is the fix for the reported bug: the old condition was "is a workspace open **and** is this
    // outside it", which answered `false` for a document opened with no folder open because the
    // `Option` was `None` -- not because of anything about the path. Asking what is actually
    // watched makes that case an instance of the rule rather than one that falls through it.
    //
    // *What may the WebView read?* Only a document genuinely outside the workspace. A grant cannot
    // be revoked -- `asset_scope_revocation` pins that -- so a wrong one is permanent and this
    // half stays stingy.
    if let Ok(canonical) = path.canonicalize() {
        if let Some(dir) = canonical.parent() {
            // The watch, without consulting the workspace at all.
            {
                let mut fs_watcher = watcher.lock().unwrap();
                if !fs_watcher.covers(dir) {
                    let _ = fs_watcher.watch_non_recursive(dir);
                }
            }

            // The scope, which does still need the workspace: "is this inside the granted root"
            // is what keeps the stingy half stingy. `is_loose` is a plain `let`, not an `if let`
            // on the guard -- an `if let` scrutinee's temporary lives for the whole arm on
            // edition 2021, which is what used to hold this guard across the syscall below.
            let is_loose = workspace
                .lock()
                .unwrap()
                .as_ref()
                .is_some_and(|ws| matches!(ws.classify(&path), Ok(PathClass::Loose)));

            if is_loose {
                let _ = app.asset_protocol_scope().allow_directory(dir, false);
            }
        }
    }

    Ok(ReadResult { content, hash })
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

/// The frontend's signal that it has attached its `open:request` listener and is ready to
/// receive launches (plan-v0.1.md increment 10, ADR-003). Drains whatever `main.rs`'s listeners
/// routed before this call — a cold launch from Finder or the CLI goes through this every time;
/// it is the normal path, not the exception. Safe to call more than once (a WebView reload, dev
/// HMR, or a crash-reload all call this again): see `PendingOpens::mark_ready_and_drain`'s own
/// doc comment for why a second call correctly returns nothing rather than re-opening everything.
#[tauri::command]
pub fn frontend_ready(pending: State<'_, PendingOpens>) -> Vec<RouteTarget> {
    pending.mark_ready_and_drain()
}

#[cfg(test)]
mod command_shape {
    //! A source-level check, because the property it guards has no runtime observable.
    //!
    //! The folder picker deadlocked every copy of medd on every click: `workspace_pick` was a
    //! plain `#[tauri::command]`, so it ran inline on the thread dispatching the IPC message --
    //! the main thread -- and `blocking_pick_folder` then waited *there* for a panel that needs
    //! the main run loop in order to appear. The app waited for itself.
    //!
    //! No test we have or could write catches that: it is a deadlock between a dependency and the
    //! platform event loop, invisible to unit tests, to the browser harness, and to mutation. So
    //! the class is made structurally detectable instead of behaviourally testable -- the same
    //! move as requiring an owned root in a signature rather than trusting a call site: when a
    //! requirement has no observable, check whether it is really a constraint on *capability*,
    //! and if it is, enforce the shape.
    //!
    //! The rule: **a command whose body calls a `blocking_*` platform API must be declared
    //! `#[tauri::command(async)]`.** Async commands run on the threadpool, where blocking is safe;
    //! plain ones run on the main thread, where it is a deadlock.
    //!
    //! This reads its own source deliberately. The attribute is erased by the macro, so there is
    //! nothing left at runtime to assert against.

    /// The command surface only -- everything before this test module. `include_str!` pulls in
    /// this module too, and its own prose contains both `#[tauri::command(async)]` and
    /// `blocking_*`, which the scan happily matched as real code. Caught because the check was
    /// run against known-GOOD source first: run only against the broken version, a
    /// self-referencing scan reports the offender it was written for and looks correct.
    fn source() -> &'static str {
        const FULL: &str = include_str!("commands.rs");
        match FULL.find("\n#[cfg(test)]") {
            Some(i) => &FULL[..i],
            None => FULL,
        }
    }

    /// Every `#[tauri::command...]` in this file, as (attribute line, function name, body).
    fn commands() -> Vec<(String, String, String)> {
        let mut out = Vec::new();
        // Line-start only: a real attribute sits at column 0, a mention inside a doc comment
        // does not.
        let marker = "\n#[tauri::command";
        let mut rest = source();
        while let Some(i) = rest.find(marker) {
            rest = &rest[i..];
            // `rest` begins with the marker's own leading newline; skip it before looking for
            // the end of the attribute line, or the attribute comes out empty and every command
            // reads as "not async".
            let line = &rest[1..];
            let attr_end = line.find('\n').unwrap_or(line.len());
            let attr = line[..attr_end].to_string();
            let after = &line[attr_end..];
            let name = after
                .split_once("fn ")
                .and_then(|(_, t)| t.split(|c: char| !c.is_alphanumeric() && c != '_').next())
                .unwrap_or("<unknown>")
                .to_string();
            // Body: up to the next command attribute, or end of file.
            let body_end = after[1..]
                .find(marker)
                .map(|j| j + 1)
                .unwrap_or(after.len());
            out.push((attr, name, after[..body_end].to_string()));
            rest = &after[body_end..];
            if rest.is_empty() {
                break;
            }
        }
        out
    }

    #[test]
    fn a_command_that_blocks_on_the_platform_is_declared_async() {
        let offenders: Vec<String> = commands()
            .into_iter()
            .filter(|(_, _, body)| body.contains("blocking_"))
            .filter(|(attr, _, _)| !attr.contains("(async)"))
            .map(|(_, name, _)| name)
            .collect();

        assert!(
            offenders.is_empty(),
            "these commands call a blocking platform API from the main thread and will deadlock \
             the app: {offenders:?}. Declare them #[tauri::command(async)] so they run on the \
             threadpool. This is the folder-picker bug; it has no runtime test."
        );
    }

    #[test]
    fn the_check_can_actually_fail() {
        // Guards the guard. If `commands()` ever stops finding commands -- a parser change, a
        // reformat, a move to another file -- the test above passes by finding nothing, which is
        // the failure mode it exists to prevent. An empty list is not a clean bill of health.
        let found = commands();
        assert!(
            found.len() >= 5,
            "expected to find the command surface; found {} -- the scan is broken, and a broken \
             scan reports no offenders",
            found.len()
        );
        assert!(
            found.iter().any(|(_, name, _)| name == "workspace_pick"),
            "workspace_pick is the command this check exists for and it was not found: {:?}",
            found.iter().map(|(_, n, _)| n).collect::<Vec<_>>()
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
    fn workspace_info_fields_are_what_app_svelte_reads() {
        let json = serde_json::to_string(&WorkspaceInfo {
            root: PathBuf::from("/w"),
            name: "w".to_string(),
        })
        .unwrap();
        assert!(
            json.contains(r#""root":"#) && json.contains(r#""name":"#),
            "{json}"
        );
    }

    #[test]
    fn read_result_fields_are_what_app_svelte_reads() {
        let json = serde_json::to_string(&ReadResult {
            content: "x".to_string(),
            hash: crate::document::ContentHash::of(b"x"),
        })
        .unwrap();
        assert!(
            json.contains(r#""content":"x""#) && json.contains(r#""hash":"#),
            "{json}"
        );
    }
}

#[cfg(test)]
mod asset_scope_revocation {
    //! `forbid_directory` is not a release. `FsScope`'s four mutators all *push* onto one of two
    //! monotonic `HashSet<Pattern>`s -- verified against tauri-2.11.5's `src/scope/fs.rs`, whose
    //! only `remove` call is on event listeners -- and `is_allowed` checks `forbidden_patterns`
    //! first, returning `false` on a match **without consulting the allow list**.
    //!
    //! So a forbid is permanent and beats every later grant. `rescope_workspace` uses it as the
    //! counterpart to `allow_directory`, which makes revisiting a workspace break it for the rest
    //! of the process. Under D-4 and I-3 -- an editor that stays open for days -- switching back
    //! to a folder you had open earlier is ordinary use, not an edge case.
    use super::*;
    use tauri::Manager;

    #[test]
    fn a_forbid_is_permanent_and_outranks_every_later_grant() {
        let dir = tempfile::Builder::new()
            .prefix("medd-scope-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let image = root.join("img.png");
        std::fs::write(&image, b"not really a png").unwrap();

        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds");
        let scope = app.asset_protocol_scope();

        // Open the workspace: readable, as it must be.
        scope.allow_directory(&root, true).unwrap();
        assert!(
            scope.is_allowed(&image),
            "a freshly opened workspace's files must be readable -- if this fails the test proves \
             nothing about what follows"
        );

        // Switch away. This is what `rescope_workspace` does today.
        scope.forbid_directory(&root, true).unwrap();
        assert!(!scope.is_allowed(&image), "forbidding takes effect");

        // Re-grant the very same root, exactly as reopening the workspace would.
        scope.allow_directory(&root, true).unwrap();

        assert!(
            !scope.is_allowed(&image),
            "the re-grant is INEFFECTIVE: a forbid is permanent and outranks every later grant, \
             which is the library property that makes `forbid_directory` unusable as a release \
             and the reason `rescope_workspace` must never call it. If this ever starts passing, \
             FsScope has gained real revocation and the design built around its absence should be \
             revisited."
        );
    }

    #[test]
    fn switching_workspace_and_returning_leaves_the_first_one_readable() {
        // The behavioural claim, against the real function rather than against `FsScope`. The
        // test above establishes *why* this could fail; this one establishes that it does not.
        // Both are needed: the first would keep passing if `rescope_workspace` were reverted, and
        // the second would keep passing if it were never written.
        let a = tempfile::Builder::new()
            .prefix("medd-ws-a-")
            .tempdir()
            .unwrap();
        let c = tempfile::Builder::new()
            .prefix("medd-ws-c-")
            .tempdir()
            .unwrap();
        let root_a = a.path().canonicalize().unwrap();
        let root_c = c.path().canonicalize().unwrap();
        let image = root_a.join("img.png");
        std::fs::write(&image, b"not really a png").unwrap();

        let (fs_watcher, _rx) = FsWatcher::new().unwrap();
        let app = tauri::test::mock_builder()
            .manage(Mutex::new(fs_watcher))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds");
        let watcher = app.state::<Mutex<FsWatcher>>();

        rescope_workspace(app.handle(), &watcher, None, root_a.clone());
        assert!(
            scope_allows(&app, &image),
            "the first workspace opens readable"
        );

        rescope_workspace(app.handle(), &watcher, Some(root_a.clone()), root_c.clone());
        rescope_workspace(app.handle(), &watcher, Some(root_c), root_a.clone());

        assert!(
            scope_allows(&app, &image),
            "returning to a workspace must leave it readable; a permanent forbid on the way out \
             would make every image in it dead until restart"
        );
    }

    fn scope_allows<R: Runtime>(app: &tauri::App<R>, path: &std::path::Path) -> bool {
        app.asset_protocol_scope().is_allowed(path)
    }
}

#[cfg(test)]
mod watching_a_document_opened_without_a_workspace {
    //! Field report: a document edited externally did not reload. This asks the narrowest
    //! question that could explain it -- **is anything being watched at all?**
    //!
    //! `document_read` registers a watch only when `is_loose`, and `is_loose` is computed with
    //! `workspace.as_ref().is_some_and(..)`. With no workspace open that is `false` by the
    //! `Option` being `None`, not by any judgement about the path -- so the watch is skipped for
    //! a reason that never mentions the document. Opening a file straight from Finder, the CLI or
    //! Neovim, without opening a folder first, is medd's stated launch path (README) and reaches
    //! exactly this state.
    //!
    //! Asserted by writing to the file from outside and draining the watcher, rather than by
    //! reading the condition: the question is whether events arrive, and only events answer it.
    use super::*;
    use crate::watcher::decide;
    use std::time::Duration;
    use tauri::Manager;

    fn drain(
        rx: &std::sync::mpsc::Receiver<notify_debouncer_full::DebounceEventResult>,
    ) -> Vec<notify_debouncer_full::DebouncedEvent> {
        let mut all = Vec::new();
        while let Ok(Ok(events)) = rx.recv_timeout(Duration::from_millis(500)) {
            all.extend(events);
        }
        all
    }

    /// Builds a mock app whose workspace state is whatever the caller passes, reads `file`
    /// through the real command, then edits `file` externally and returns what the watcher saw.
    fn events_after_external_edit(
        workspace: Option<Workspace>,
        file: &std::path::Path,
    ) -> Vec<crate::watcher::WatcherEvent> {
        let (fs_watcher, rx) = FsWatcher::new().unwrap();
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![document_read])
            .manage(DocumentStore::new())
            .manage(Mutex::new(workspace))
            .manage(Mutex::new(fs_watcher))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds");

        document_read(
            app.handle().clone(),
            file.to_path_buf(),
            app.state::<DocumentStore>(),
            app.state::<Mutex<Option<Workspace>>>(),
            app.state::<Mutex<FsWatcher>>(),
        )
        .expect("the document reads");

        std::fs::write(file, "edited by another editor").unwrap();

        let store = app.state::<DocumentStore>();
        decide(&drain(&rx), None, &store)
    }

    #[test]
    fn a_document_opened_with_no_workspace_is_watched_and_reloads() {
        let dir = tempfile::Builder::new()
            .prefix("medd-loose-watch-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("note.md");
        std::fs::write(&file, "v1").unwrap();

        let events = events_after_external_edit(None, &file);

        // Re-aimed from the characterisation it started as. It asserted `events.is_empty()` --
        // the defect -- and went red the moment the condition changed from "is a workspace open
        // and is this outside it" to "is this outside whatever we already watch", which is the
        // fix landing rather than breakage. The old assertion and this one are the same test
        // pointed at opposite answers, which is the shape a characterisation test should have.
        assert!(
            events.contains(&crate::watcher::WatcherEvent::DocumentChanged {
                path: file.to_path_buf(),
                content: "edited by another editor".to_string(),
                hash: crate::document::ContentHash::of(b"edited by another editor"),
            }),
            "a document opened with no folder open must still be watched -- this is the reported \
             bug, and medd's stated launch path opens files exactly this way; got {events:?}"
        );
    }

    #[test]
    fn the_same_file_is_watched_when_a_workspace_is_open() {
        // The control, and the test above is worthless without it. `assert!(events.is_empty())`
        // passes just as happily when the drain is broken, the debounce never fires, or the mock
        // app never ran the command -- every way of observing nothing looks identical to nothing
        // happening. This arm changes exactly one input, the workspace, and requires the opposite
        // result from the same harness.
        let dir = tempfile::Builder::new()
            .prefix("medd-loose-watch-")
            .tempdir()
            .unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("note.md");
        std::fs::write(&file, "v1").unwrap();

        // A workspace rooted somewhere else, so `file` classifies as Loose and `document_read`
        // takes the branch that registers a watch on its directory.
        let other = tempfile::Builder::new()
            .prefix("medd-other-root-")
            .tempdir()
            .unwrap();
        let workspace = Workspace::open(other.path()).expect("the other root opens");

        let events = events_after_external_edit(Some(workspace), &file);

        assert!(
            events.contains(&crate::watcher::WatcherEvent::DocumentChanged {
                path: file.clone(),
                content: "edited by another editor".to_string(),
                hash: crate::document::ContentHash::of(b"edited by another editor"),
            }),
            "a loose document IS watched, and this harness can see the events the other test \
             asserts the absence of; got {events:?}"
        );
    }
}

#[cfg(test)]
mod ipc_probe {
    //! PROBE — what can a Rust test actually reach above the IPC boundary?
    use super::*;
    use tauri::Manager;

    #[test]
    fn a_command_runs_against_real_tauri_state_and_its_effect_is_observable() {
        let app = tauri::test::mock_builder()
            .invoke_handler(tauri::generate_handler![quit_ready])
            .manage(crate::quit::QuitCoordinator::new())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app builds");

        // A real `State<'_, QuitCoordinator>`, from a real app handle.
        let coordinator = app.state::<crate::quit::QuitCoordinator>();
        let rx = coordinator
            .decide()
            .start_flush
            .expect("first decision starts the flush");

        // Call the command itself, with the Tauri type it takes in production.
        quit_ready(app.state::<crate::quit::QuitCoordinator>());

        assert!(
            crate::quit::wait_for_quit_signal(&rx, std::time::Duration::from_secs(1)),
            "quit_ready should have signalled the coordinator it was given"
        );
    }
}

#[cfg(test)]
mod picker_reachability {
    //! Does a mock app give enough to reach `workspace_pick`? Reports capabilities; never calls
    //! `blocking_pick_folder`, which would hang if the answer is no.
    use tauri::Manager;

    #[test]
    fn what_a_mock_app_provides() {
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_dialog::init())
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .expect("mock app with the dialog plugin builds");

        let windows = app.webview_windows();
        println!("PROBE webview_windows = {}", windows.len());
        println!(
            "PROBE window names     = {:?}",
            windows.keys().collect::<Vec<_>>()
        );
        // Can we even obtain a dialog handle?
        let _ = &app;
        println!("PROBE dialog plugin initialised without panicking");

        // NOT attempted here: `blocking_pick_folder` itself. Done once, on a spawned thread
        // with a 4s bound, and it DID NOT RETURN — so a test that calls it hangs the suite rather
        // than reporting. Left out deliberately; the result is recorded in
        // verification-status.md §4 so nobody has to rediscover it by hanging their own run.
    }
}

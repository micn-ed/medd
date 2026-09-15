// Prevents an additional console window on Windows in release builds. medd targets macOS only
// (see README), but this is harmless to leave in place.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Mutex;

use tauri::menu::{AboutMetadata, Menu, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};

use routing::{paths_from_argv, paths_from_urls, route_open, Delivery, PendingOpens};

mod commands;
mod document;
mod error;
mod quickopen;
mod quit;
mod routing;
mod state;
mod watcher;
mod workspace;

/// Builds medd's menu bar: `tauri::menu::Menu::default` in every respect except two targeted
/// swaps, both routed to the plan's fifth blocker (docs/design-quit-flush.md).
///
/// **Cmd+W closes the active tab, not the window.** `Menu::default` binds it to
/// `PredefinedMenuItem::close_window`, which under I-2 ("closing the window quits the app") makes
/// Cmd+W quit medd — the opposite of what it means in every other tabbed editor. Cmd+Shift+W takes
/// over closing the window instead, via `Window::close()` below — still an exit path under I-2, so
/// it still flushes, but through `RunEvent::WindowEvent`'s `CloseRequested`, not `ExitRequested`:
/// an architect review found that `ExitRequested` on the window-close route fires only *after* the
/// window (and the webview the flush needs) is already destroyed, so `main`'s handler catches both
/// event shapes rather than assuming one covers every exit — see `QuitCoordinator::decide` and
/// `start_flush_thread`. This keeps the conventional keyboard route to closing the window from
/// disappearing as a side effect of freeing up Cmd+W.
///
/// **Quit is a custom item, not `PredefinedMenuItem::quit`.** This is not cosmetic: on macOS,
/// `PredefinedMenuItem::quit` maps to `[NSApp terminate:]`, whose only cancellation point
/// (`applicationShouldTerminate:`) `tao` does not implement — so Cmd+Q would bypass
/// `RunEvent::ExitRequested` entirely and the flush below would silently stop covering the most
/// common quit gesture on the platform, while still passing every test written against
/// `ExitRequested`. A custom item's handler calling `AppHandle::exit()` routes through
/// `Message::RequestExit` instead, which *is* `ExitRequested` and *is* preventable. Verified this
/// assumption by spot-testing a real `Cmd+Q` keystroke against a custom menu item before writing
/// any of the flush logic behind it. Two things worth recording so nobody re-discovers them the
/// slow way: a bare `MenuItem` appended directly to the top-level `Menu` is accepted without error
/// on macOS and simply never appears (only `Submenu`s may sit at the top level there), and macOS
/// overrides the *first* top-level submenu's displayed title to the running application's name
/// regardless of the string given to it.
fn build_menu(app: &tauri::AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let pkg_info = app.package_info();
    let config = app.config();
    let about_metadata = AboutMetadata {
        name: Some(pkg_info.name.clone()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config.bundle.publisher.clone().map(|p| vec![p]),
        ..Default::default()
    };

    let quit_item = MenuItemBuilder::with_id("quit", "Quit")
        .accelerator("CmdOrCtrl+Q")
        .build(app)?;
    let close_tab_item = MenuItemBuilder::with_id("close-tab", "Close Tab")
        .accelerator("CmdOrCtrl+W")
        .build(app)?;
    let close_window_item = MenuItemBuilder::with_id("close-window", "Close Window")
        .accelerator("CmdOrCtrl+Shift+W")
        .build(app)?;

    let app_menu = SubmenuBuilder::new(app, pkg_info.name.clone())
        .about(Some(about_metadata))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .separator()
        .item(&quit_item)
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&close_tab_item)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View").fullscreen().build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .item(&close_window_item)
        .build()?;

    Menu::with_items(
        app,
        &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu],
    )
}

fn main() {
    let app = tauri::Builder::default()
        // Registered first, per upstream's own recommendation: this plugin's `.setup()` is the
        // thing that decides, atomically and exactly once, whether this process becomes the
        // primary or forwards to one and exits — everything else should happen after that
        // question is settled. The callback here only ever fires for a *secondary* launch that
        // this process's own bind refused (architecture.md §9); the primary's own cold-launch
        // argv never reaches it; see the `.setup()` block below for that half.
        .plugin(tauri_plugin_single_instance::init(
            |app_handle, argv, _cwd| {
                // The shim resolves relative paths to absolute before handing them on
                // (architecture.md §9), so `_cwd` is deliberately unused here — the running
                // instance's own working directory is not the launching one's, and the plugin
                // forwards `_cwd` only because the wire format always carries it, not because medd
                // needs it. QA's criterion for this exact claim ("the shim is authoritative for
                // resolution") is tested through `scripts/medd` itself, not here.
                handle_launch(app_handle, paths_from_argv(&argv));
            },
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(document::DocumentStore::new())
        .manage(Mutex::new(None::<workspace::Workspace>))
        .manage(quit::QuitCoordinator::new())
        .manage(quickopen::FileIndex::new())
        .manage(document::TempSweeper::new())
        .manage(PendingOpens::new())
        .setup(|app| {
            let (fs_watcher, rx) =
                watcher::FsWatcher::new().expect("failed to start filesystem watcher");
            app.manage(Mutex::new(fs_watcher));

            // Its own thread (architecture.md §6): a filesystem event storm drains here, never
            // on a thread command handling depends on.
            let handle = app.handle().clone();
            std::thread::spawn(move || watcher::run_event_loop(rx, handle));

            let menu = build_menu(app.handle())?;
            app.set_menu(menu)?;

            // The primary's OWN cold-launch arguments — `medd fixture.md` when nothing was
            // running yet. The single-instance plugin's callback above never sees these: it only
            // fires when a *later* launch finds this process already bound. Routed through the
            // exact same `handle_launch` as every other entry point, so this is one more caller
            // rather than a fourth thing to keep in sync with the other three.
            let argv: Vec<String> = std::env::args().collect();
            handle_launch(app.handle(), paths_from_argv(&argv));

            app.on_menu_event(|app_handle, event| match event.id().as_ref() {
                "quit" => app_handle.exit(0),
                // Just asks the window to close, exactly as if the user had clicked the traffic
                // light — `RunEvent::WindowEvent`'s `CloseRequested` arm below is what actually
                // intercepts that (to flush) before it's allowed to proceed. Only the *keyboard
                // route* to this action needed to move off Cmd+W.
                "close-window" => {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        let _ = window.close();
                    }
                }
                // A tab close, unlike a window close, is entirely the frontend's business — tabs
                // don't exist in Rust. `closeTab` already flushes any pending autosave for the
                // tab it removes (plan-v0.1.md's close-flush work), so routing Cmd+W through the
                // same function the tab strip's own close button uses costs nothing extra here.
                // Closing medd's very last tab leaves the window open showing the "click a
                // Markdown file" hint, same as VS Code — chosen, not accidental: I-2 governs the
                // *window*, and a tab isn't one.
                "close-tab" => {
                    let _ = app_handle.emit("menu:close-tab", ());
                }
                _ => {}
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::workspace_pick,
            commands::workspace_open,
            commands::dir_list,
            commands::document_read,
            commands::document_write,
            commands::quit_ready,
            commands::quick_open_files,
            commands::open_external,
            commands::frontend_ready,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // Two event *shapes* reach here needing the same treatment, and an architect review is
        // why both are handled rather than just the first: `RunEvent::ExitRequested` (Cmd+Q's
        // custom item, or `AppHandle::exit()` called programmatically) fires early enough that the
        // webview is still alive to flush. `WindowEvent::CloseRequested` (the traffic light,
        // Cmd+Shift+W) does NOT reach `ExitRequested` until the window — and the webview inside
        // it — has already been destroyed, so a version that only hooked `ExitRequested` silently
        // flushed nothing on every window-close route.
        //
        // Neither arm decides anything itself — architecture.md §2, "a shell contains no
        // decisions" — it only asks `QuitCoordinator::decide` and mechanically acts on the answer:
        // call the event's own prevent function or don't, start the flush thread or don't. The
        // decision logic (is this the first request, has the flush finished, should a repeat
        // press be let through) lives entirely in quit.rs, where it can be — and is — tested
        // without a Tauri type in sight.
        //
        // A future Dock "Quit" or `SIGTERM`/`SIGKILL` still terminate via AppKit/the OS with no
        // hook at all reachable from here — nothing can flush those; see quit.rs's own doc comment.
        match event {
            RunEvent::ExitRequested { api, .. } => {
                let decision = app_handle.state::<quit::QuitCoordinator>().decide();
                if decision.prevent {
                    api.prevent_exit();
                }
                if let Some(rx) = decision.start_flush {
                    start_flush_thread(app_handle.clone(), rx);
                }
            }
            RunEvent::WindowEvent {
                event: WindowEvent::CloseRequested { api, .. },
                ..
            } => {
                let decision = app_handle.state::<quit::QuitCoordinator>().decide();
                if decision.prevent {
                    api.prevent_close();
                }
                if let Some(rx) = decision.start_flush {
                    start_flush_thread(app_handle.clone(), rx);
                }
            }
            // Finder double-click / "Open With" (ADR-003): the other listener that carries
            // paths, routed through the exact same `handle_launch` as the CLI's two call sites.
            RunEvent::Opened { urls } => {
                handle_launch(app_handle, paths_from_urls(&urls));
            }
            // Dock click, or `open -a` on an already-running instance — carries no paths at all
            // (ADR-003: "routing and activation are separate", and this listener is the reason
            // that became visible), so it calls `activate` alone rather than `handle_launch`,
            // which would have nothing to route and nowhere to send an empty batch.
            RunEvent::Reopen { .. } => {
                activate_main_window(app_handle);
            }
            _ => {}
        }
    });
}

/// Routes `paths` (already argv[0]-stripped or URL-filtered by the caller) and activates the
/// main window — the two actions ADR-003 says every path-carrying listener performs, in that
/// order. Contains no decision of its own: `route_open` has already decided what each path is,
/// and `PendingOpens::offer` has already decided whether to deliver live or hold, so this only
/// ever matches on their answers and performs the mechanical action each one names.
fn handle_launch(app_handle: &AppHandle, paths: Vec<PathBuf>) {
    let mut targets = Vec::new();
    let mut errors = Vec::new();
    for result in route_open(paths) {
        match result {
            Ok(target) => targets.push(target),
            Err(e) => errors.push(e),
        }
    }

    if !errors.is_empty() {
        // Not a surface that replaces the tab UI (architecture.md §9) — the frontend's own
        // `open:error` handler is responsible for that, same posture as `document_read`'s errors.
        let _ = app_handle.emit("open:error", errors);
    }

    if !targets.is_empty() {
        if let Delivery::Live(targets) = app_handle.state::<PendingOpens>().offer(targets) {
            let _ = app_handle.emit("open:request", targets);
        }
    }

    activate_main_window(app_handle);
}

fn activate_main_window(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        routing::activate(&window);
    }
}

/// Asks the frontend to flush (doc/doc.ts's `flushAll`) and waits, bounded, for it to reach
/// quiescence before actually exiting. Only ever called with the receiver from a `decide()` call
/// that returned `start_flush: Some(_)` — the first exit-shaped event, never a repeat.
fn start_flush_thread(app_handle: AppHandle, rx: mpsc::Receiver<()>) {
    // The frontend's signal (`quit_ready`) can only make the exit below happen sooner than
    // `QUIT_FLUSH_CEILING`, never later — see quit.rs's own doc comment for why that direction
    // only matters.
    let _ = app_handle.emit("app:before-quit", ());

    std::thread::spawn(move || {
        if !quit::wait_for_quit_signal(&rx, quit::QUIT_FLUSH_CEILING) {
            eprintln!(
                "medd: quit flush did not complete within {:?}; exiting anyway",
                quit::QUIT_FLUSH_CEILING
            );
        }
        app_handle
            .state::<quit::QuitCoordinator>()
            .mark_ready_to_exit();
        app_handle.exit(0);
    });
}

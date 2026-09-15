// Prevents an additional console window on Windows in release builds. medd targets macOS only
// (see README), but this is harmless to leave in place.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use tauri::menu::{AboutMetadata, Menu, MenuItemBuilder, SubmenuBuilder};
use tauri::{Emitter, Manager, RunEvent};

mod commands;
mod document;
mod error;
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
/// over closing the window (still an exit path under I-2, so it still flushes; see `main`'s
/// `RunEvent::ExitRequested` handling), so the conventional keyboard route to that action doesn't
/// disappear as a side effect of freeing up Cmd+W.
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
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(document::DocumentStore::new())
        .manage(Mutex::new(None::<workspace::Workspace>))
        .manage(quit::QuitCoordinator::new())
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

            app.on_menu_event(|app_handle, event| match event.id().as_ref() {
                "quit" => app_handle.exit(0),
                // `WindowEvent::CloseRequested` is deliberately left unintercepted: closing the
                // window is meant to proceed exactly as it always has (and, under I-2, exit the
                // app once it's the last one), the same as if the user had clicked the traffic
                // light. Only the *keyboard route* to that action needed to move off Cmd+W.
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
            commands::open_external,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        // Every path that ends the process — Cmd+Q's custom item above, the window closing as
        // I-2's last one, a future Dock "Quit" (which also terminates via AppKit and is one of
        // the routes nothing here can flush; see quit.rs's own doc comment) landing here as a
        // programmatic exit instead — funnels through this one event rather than being hooked at
        // each origin, so there is exactly one place that ever decides whether an exit is allowed
        // to proceed yet.
        if let RunEvent::ExitRequested { api, .. } = event {
            let coordinator = app_handle.state::<quit::QuitCoordinator>();
            let Some(rx) = coordinator.begin_shutdown() else {
                // Not the first request: either this *is* our own completion signal calling
                // `exit()` again, or the user pressed the quit gesture again while still waiting.
                // Either way, per the leader's ruling, a repeat means "yes, I mean it" — let this
                // one proceed rather than restarting (or re-preventing) the bound below.
                return;
            };
            api.prevent_exit();

            // Ask the frontend to flush (doc/doc.ts's `flushAll`) and wait for every path to
            // reach quiescence, then report back — but the bound below is what actually decides
            // when medd exits. The frontend's signal can only make that happen sooner.
            let _ = app_handle.emit("app:before-quit", ());

            let handle = app_handle.clone();
            std::thread::spawn(move || {
                if !quit::wait_for_quit_signal(&rx, quit::QUIT_FLUSH_CEILING) {
                    eprintln!(
                        "medd: quit flush did not complete within {:?}; exiting anyway",
                        quit::QUIT_FLUSH_CEILING
                    );
                }
                handle.exit(0);
            });
        }
    });
}

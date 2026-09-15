// Prevents an additional console window on Windows in release builds. medd targets macOS only
// (see README), but this is harmless to leave in place.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;

use tauri::Manager;

mod commands;
mod document;
mod error;
mod routing;
mod state;
mod watcher;
mod workspace;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(document::DocumentStore::new())
        .manage(Mutex::new(None::<workspace::Workspace>))
        .setup(|app| {
            let (fs_watcher, rx) =
                watcher::FsWatcher::new().expect("failed to start filesystem watcher");
            app.manage(Mutex::new(fs_watcher));

            // Its own thread (architecture.md §6): a filesystem event storm drains here, never
            // on a thread command handling depends on.
            let handle = app.handle().clone();
            std::thread::spawn(move || watcher::run_event_loop(rx, handle));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::workspace_pick,
            commands::workspace_open,
            commands::dir_list,
            commands::document_read,
            commands::document_write,
            commands::open_external,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

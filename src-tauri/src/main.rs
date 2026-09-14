// Prevents an additional console window on Windows in release builds. medd targets macOS only
// (see README), but this is harmless to leave in place.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod document;
mod error;
mod routing;
mod state;
mod watcher;
mod workspace;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::ping])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

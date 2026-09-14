//! The `#[tauri::command]` surface (architecture.md §4). This is the only module in the
//! workspace that knows Tauri's command macros exist — everything else stays plain Rust.

/// Skeleton smoke test: proves a value can cross the IPC bridge in both directions.
/// Not part of the real command surface in §4 — removed once increment 2 lands `document_read`.
#[tauri::command]
pub fn ping(message: String) -> String {
    format!("pong: {message} (medd v{})", env!("CARGO_PKG_VERSION"))
}

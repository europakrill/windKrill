//! Tauri application shell.

mod commands;

use commands::BuilderExt;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .register_krill_commands()
        .run(tauri::generate_context!())
        .expect("failed to run windKrill Tauri application");
}

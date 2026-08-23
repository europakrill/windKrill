//! krill-app: Tauri command layer. Compiled only when building the full
//! desktop app (requires tauri CLI + platform webview deps).

fn main() {
    krill_app_lib::run();
}

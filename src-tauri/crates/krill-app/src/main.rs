//! windKrill desktop binary entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    krill_app_lib::run();
}

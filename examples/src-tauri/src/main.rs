// Thin desktop entry. The app lives in lib.rs so mobile builds can
// load it as a library (tauri's mobile entry point); desktop just
// calls through.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    faders_lib::run();
}

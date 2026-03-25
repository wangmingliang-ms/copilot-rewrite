// Copilot Rewrite - Main entry point
// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    copilot_rewrite_lib::run()
}

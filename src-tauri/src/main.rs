// Copilot Rewrite - Main entry point
// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::OpenOptions;
use std::os::windows::fs::OpenOptionsExt;

fn main() {
    // Single-instance check using a lock file with no sharing
    let lock_path = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("copilot-rewrite")
        .join(".lock");

    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // FILE_SHARE_NONE = 0 means no other process can open this file
    let _lock_file = match OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0) // No sharing — fails if another instance has it open
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(_) => {
            eprintln!("Copilot Rewrite is already running.");
            std::process::exit(0);
        }
    };

    // _lock_file stays open for program lifetime, preventing second instances
    copilot_rewrite_lib::run()
}

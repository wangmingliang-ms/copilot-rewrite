// Clipboard management module
// Handles reading/writing clipboard and save/restore operations

pub mod manager;

pub use manager::{get_text, set_text, ClipboardGuard};

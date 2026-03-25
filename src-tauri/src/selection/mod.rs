// Selection engine module
// Combines UIA polling and clipboard fallback to detect text selection system-wide

pub mod uia;
pub mod monitor;

pub use monitor::start_selection_engine;

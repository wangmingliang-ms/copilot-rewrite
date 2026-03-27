// Selection engine module
// Combines UIA polling and clipboard fallback to detect text selection system-wide

pub mod monitor;
pub mod uia;

pub use monitor::start_selection_engine;

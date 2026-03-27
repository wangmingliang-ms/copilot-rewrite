// Clipboard manager using clipboard-win crate
// Provides save/restore functionality to avoid losing user's clipboard content

use anyhow::Result;
use clipboard_win::{formats, get_clipboard, set_clipboard};
use log::{debug, warn};

/// Get text content from the system clipboard
pub fn get_text() -> Result<String> {
    let text: String = get_clipboard(formats::Unicode)
        .map_err(|e| anyhow::anyhow!("Failed to read clipboard text: {}", e))?;
    Ok(text)
}

/// Set text content to the system clipboard
pub fn set_text(text: &str) -> Result<()> {
    set_clipboard(formats::Unicode, text)
        .map_err(|e| anyhow::anyhow!("Failed to write to clipboard: {}", e))?;
    Ok(())
}

/// RAII guard that saves clipboard content on creation and restores it on drop
/// Used during text replacement to preserve the user's clipboard
pub struct ClipboardGuard {
    saved_text: Option<String>,
}

impl ClipboardGuard {
    /// Create a new guard, saving the current clipboard content
    pub fn new() -> Self {
        let saved_text = match get_text() {
            Ok(text) => {
                debug!("Saved clipboard content: {} chars", text.len());
                Some(text)
            }
            Err(e) => {
                warn!("Could not save clipboard content: {}", e);
                None
            }
        };

        Self { saved_text }
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        if let Some(ref text) = self.saved_text {
            // Small delay to ensure the Ctrl+V paste has completed
            std::thread::sleep(std::time::Duration::from_millis(100));

            match set_text(text) {
                Ok(_) => debug!("Restored clipboard content"),
                Err(e) => warn!("Failed to restore clipboard content: {}", e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_roundtrip() {
        let test_text = "Copilot Rewrite test string 🧪";
        set_text(test_text).expect("Failed to set clipboard");
        let result = get_text().expect("Failed to get clipboard");
        assert_eq!(result, test_text);
    }
}

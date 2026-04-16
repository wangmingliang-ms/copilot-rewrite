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

/// Set both HTML and plain text content to the system clipboard.
/// Apps that support rich paste (Teams, Outlook) will use the HTML format;
/// apps that only support plain text will use the Unicode fallback.
pub fn set_html(html: &str, plain_text: &str) -> Result<()> {
    use clipboard_win::raw;

    // CF_HTML format requires a specific header
    let cf_html = build_cf_html(html);

    // Register CF_HTML format
    let html_format = unsafe {
        windows::Win32::System::DataExchange::RegisterClipboardFormatW(
            windows::core::w!("HTML Format"),
        )
    };

    // Open clipboard and set both formats
    raw::open().map_err(|e| anyhow::anyhow!("Failed to open clipboard: {}", e))?;
    let _ = raw::empty();

    // Set CF_UNICODETEXT (plain text fallback)
    // Use set_without_clear because we already called raw::empty() above.
    // raw::set() would call empty() again, which is fine for the first format
    // but would wipe previously-set formats if used for subsequent calls.
    let wide: Vec<u16> = plain_text.encode_utf16().chain(std::iter::once(0)).collect();
    let text_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2)
    };
    if let Err(e) = raw::set_without_clear(formats::CF_UNICODETEXT, text_bytes) {
        let _ = raw::close();
        anyhow::bail!("Failed to set clipboard text: {}", e);
    }

    // Set CF_HTML (rich text)
    // MUST use set_without_clear — raw::set() empties the clipboard first,
    // which would wipe the CF_UNICODETEXT we just set above.
    if let Err(e) = raw::set_without_clear(html_format, cf_html.as_bytes()) {
        debug!("Failed to set CF_HTML (non-fatal): {}", e);
    }

    raw::close().map_err(|e| anyhow::anyhow!("Failed to close clipboard: {}", e))?;
    Ok(())
}

/// Build the CF_HTML clipboard format string with required headers.
///
/// SAFETY INVARIANT: The placeholder strings (SSSSSSSSSS, etc.) are each exactly
/// 10 characters, matching the `{:010}` format width. If the placeholder length
/// changes, the offset calculations will be wrong (header.len() would change).
fn build_cf_html(html_fragment: &str) -> String {
    // CF_HTML format: https://docs.microsoft.com/en-us/windows/win32/dataxchg/html-clipboard-format
    let header = "Version:0.9\r\nStartHTML:SSSSSSSSSS\r\nEndHTML:EEEEEEEEEE\r\nStartFragment:FFFFFFFFFF\r\nEndFragment:GGGGGGGGGG\r\n";
    let prefix = "<html><body>\r\n<!--StartFragment-->";
    let suffix = "<!--EndFragment-->\r\n</body></html>";

    let start_html = header.len();
    let start_fragment = start_html + prefix.len();
    let end_fragment = start_fragment + html_fragment.len();
    let end_html = end_fragment + suffix.len();

    let mut result = header.to_string();
    result = result.replace("SSSSSSSSSS", &format!("{:010}", start_html));
    result = result.replace("EEEEEEEEEE", &format!("{:010}", end_html));
    result = result.replace("FFFFFFFFFF", &format!("{:010}", start_fragment));
    result = result.replace("GGGGGGGGGG", &format!("{:010}", end_fragment));
    result.push_str(prefix);
    result.push_str(html_fragment);
    result.push_str(suffix);
    result
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
            std::thread::sleep(std::time::Duration::from_millis(150));

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

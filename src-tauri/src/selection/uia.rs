// UI Automation integration for detecting text selection
// Uses IUIAutomation COM interfaces via the `windows` crate

use anyhow::{Context, Result};
use log::{debug, trace};
use windows::core::{Interface, BSTR};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    UIA_TextPatternId,
};

/// Wrapper around UI Automation COM interfaces
pub struct UiaEngine {
    automation: IUIAutomation,
}

impl UiaEngine {
    /// Initialize COM and create the UIAutomation object
    pub fn new() -> Result<Self> {
        unsafe {
            // Initialize COM for this thread
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM")?;

            // Create the IUIAutomation instance
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL)
                    .context("Failed to create IUIAutomation instance")?;

            Ok(Self { automation })
        }
    }

    /// Get the currently focused element in the UI
    pub fn get_focused_element(&self) -> Result<IUIAutomationElement> {
        unsafe {
            self.automation
                .GetFocusedElement()
                .context("Failed to get focused element")
        }
    }

    /// Try to get selected text from the focused element using TextPattern
    pub fn get_selected_text(&self) -> Result<Option<String>> {
        let element = match self.get_focused_element() {
            Ok(el) => el,
            Err(e) => {
                trace!("No focused element: {}", e);
                return Ok(None);
            }
        };

        self.get_text_from_element(&element)
    }

    /// Extract selected text from a UI Automation element using TextPattern
    fn get_text_from_element(&self, element: &IUIAutomationElement) -> Result<Option<String>> {
        unsafe {
            // Try to get TextPattern from the element
            let pattern_obj = match element
                .GetCurrentPattern(UIA_TextPatternId)
            {
                Ok(p) => p,
                Err(_) => {
                    trace!("Element does not support TextPattern");
                    return Ok(None);
                }
            };

            // Cast to IUIAutomationTextPattern
            let text_pattern: IUIAutomationTextPattern = match pattern_obj.cast() {
                Ok(tp) => tp,
                Err(_) => {
                    trace!("Failed to cast to IUIAutomationTextPattern");
                    return Ok(None);
                }
            };

            // Get the selection ranges
            let selection = match text_pattern.GetSelection() {
                Ok(s) => s,
                Err(e) => {
                    trace!("GetSelection failed: {}", e);
                    return Ok(None);
                }
            };

            // Check if there are any selection ranges
            let length = selection.Length().unwrap_or(0);
            if length == 0 {
                return Ok(None);
            }

            // Get the first selection range
            let range = match selection.GetElement(0) {
                Ok(r) => r,
                Err(e) => {
                    debug!("Failed to get selection range element: {}", e);
                    return Ok(None);
                }
            };

            // Extract text from the range (limit to 10000 chars)
            let text: BSTR = match range.GetText(10000) {
                Ok(t) => t,
                Err(e) => {
                    debug!("Failed to get text from range: {}", e);
                    return Ok(None);
                }
            };

            let text_str = text.to_string();

            if text_str.is_empty() {
                Ok(None)
            } else {
                debug!("UIA selected text: {} chars", text_str.len());
                Ok(Some(text_str))
            }
        }
    }
}

// COM objects are thread-safe through COM marshaling
unsafe impl Send for UiaEngine {}
unsafe impl Sync for UiaEngine {}

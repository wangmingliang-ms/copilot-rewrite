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
    UIA_TextPatternId, UIA_ValuePatternId, UIA_EditControlTypeId, UIA_DocumentControlTypeId,
};

/// Bounding rectangle of the focused element (physical pixels)
#[derive(Debug, Clone, Copy, Default)]
pub struct ElementRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Wrapper around UI Automation COM interfaces
pub struct UiaEngine {
    automation: IUIAutomation,
}

impl UiaEngine {
    /// Initialize COM and create the UIAutomation object
    pub fn new() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .context("Failed to initialize COM")?;

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

    /// Get the bounding rectangle of the focused element
    pub fn get_focused_element_rect(&self) -> Option<ElementRect> {
        let element = self.get_focused_element().ok()?;
        unsafe {
            let rect = element.CurrentBoundingRectangle().ok()?;
            let r = ElementRect {
                x: rect.left,
                y: rect.top,
                width: rect.right - rect.left,
                height: rect.bottom - rect.top,
            };
            // Sanity check — ignore degenerate rects
            if r.width > 10 && r.height > 10 {
                Some(r)
            } else {
                None
            }
        }
    }

    /// Try to get selected text from the focused element using TextPattern
    /// Only returns text if the focused element is an editable control (input/textarea/contenteditable)
    pub fn get_selected_text(&self) -> Result<Option<String>> {
        let element = match self.get_focused_element() {
            Ok(el) => el,
            Err(e) => {
                trace!("No focused element: {}", e);
                return Ok(None);
            }
        };

        // Only trigger popup for editable elements (input fields, text areas, contenteditable)
        if !self.is_editable_element(&element) {
            trace!("Focused element is not editable — skipping");
            return Ok(None);
        }

        self.get_text_from_element(&element)
    }

    /// Check if a UIA element is an editable control
    /// Returns true for: Edit controls, Document controls (contenteditable),
    /// and any element that supports ValuePattern (general editability indicator)
    fn is_editable_element(&self, element: &IUIAutomationElement) -> bool {
        unsafe {
            // Check ControlType — Edit and Document are always editable
            if let Ok(control_type) = element.CurrentControlType() {
                if control_type == UIA_EditControlTypeId || control_type == UIA_DocumentControlTypeId {
                    return true;
                }
            }

            // Check if the element supports ValuePattern (indicates editability)
            if element.GetCurrentPattern(UIA_ValuePatternId).is_ok() {
                return true;
            }

            false
        }
    }

    /// Extract selected text from a UI Automation element using TextPattern
    fn get_text_from_element(&self, element: &IUIAutomationElement) -> Result<Option<String>> {
        unsafe {
            let pattern_obj = match element.GetCurrentPattern(UIA_TextPatternId) {
                Ok(p) => p,
                Err(_) => {
                    trace!("Element does not support TextPattern");
                    return Ok(None);
                }
            };

            let text_pattern: IUIAutomationTextPattern = match pattern_obj.cast() {
                Ok(tp) => tp,
                Err(_) => {
                    trace!("Failed to cast to IUIAutomationTextPattern");
                    return Ok(None);
                }
            };

            let selection = match text_pattern.GetSelection() {
                Ok(s) => s,
                Err(e) => {
                    trace!("GetSelection failed: {}", e);
                    return Ok(None);
                }
            };

            let length = selection.Length().unwrap_or(0);
            if length == 0 {
                return Ok(None);
            }

            let range = match selection.GetElement(0) {
                Ok(r) => r,
                Err(e) => {
                    debug!("Failed to get selection range element: {}", e);
                    return Ok(None);
                }
            };

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
                trace!("UIA selected text: {} chars", text_str.len());
                Ok(Some(text_str))
            }
        }
    }
}

unsafe impl Send for UiaEngine {}
unsafe impl Sync for UiaEngine {}

// UI Automation integration for detecting text selection
// Uses IUIAutomation COM interfaces via the `windows` crate
//
// Detection strategies (in priority order):
// 1. GetFocusedElement — fastest, works for most apps (browser input, Write Mode)
// 2. Cached element — re-check a previously found element (avoids flicker)
// 3. Event element — from TextSelectionChanged event (covers Teams, touch, etc.)
// 4. ElementFromPoint — mouse cursor fallback (works during active selection)
// 5. TreeWalker — traverse foreground window's UIA subtree (last resort)
//
// The TextSelectionChanged event handler is implemented via manual COM vtable
// (not #[implement] macro) to avoid windows-core version conflicts with Tauri.

use anyhow::{Context, Result};
use log::{debug, info, trace};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use windows::core::{Interface, BSTR, GUID};
use windows::Win32::Foundation::{HWND, POINT, S_OK};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationEventHandler,
    IUIAutomationTextPattern, IUIAutomationTreeWalker, TreeScope_Subtree, UIA_EditControlTypeId,
    UIA_Text_TextSelectionChangedEventId, UIA_TextPatternId, UIA_ValuePatternId,
};
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

/// Bounding rectangle of the focused element (physical pixels)
#[derive(Debug, Clone, Copy, Default)]
pub struct ElementRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Maximum depth for TreeWalker traversal to limit performance impact
const TREE_WALKER_MAX_DEPTH: u32 = 15;

// ═══════════════════════════════════════════════════════════════════════
// Manual COM implementation for IUIAutomationEventHandler
// We can't use #[implement] because Tauri pulls in a different windows-core
// version, causing trait mismatch. Manual vtable avoids the macro entirely.
// ═══════════════════════════════════════════════════════════════════════

/// IUnknown GUID
const IID_IUNKNOWN: GUID = GUID::from_values(
    0x00000000,
    0x0000,
    0x0000,
    [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
);

/// IUIAutomationEventHandler GUID
/// {146C3C17-F12E-4E22-8C27-F894B9B79C69}
const IID_IUIAUTOMATION_EVENT_HANDLER: GUID = GUID::from_values(
    0x146C3C17,
    0xF12E,
    0x4E22,
    [0x8C, 0x27, 0xF8, 0x94, 0xB9, 0xB7, 0x9C, 0x69],
);

/// Shared state between the event handler and the polling thread.
/// The event handler sets `has_event=true` and stores the element,
/// and the polling loop reads + clears it.
pub struct EventState {
    pub has_event: AtomicBool,
    pub event_element: parking_lot::Mutex<Option<IUIAutomationElement>>,
    /// Set to true when a TextSelectionChanged event fires but the element
    /// has no selection. This signals the polling thread to clear cached state
    /// and hide the popup.
    pub selection_cleared: AtomicBool,
}

// Safety: IUIAutomationElement is a COM pointer (ref-counted, safe to send).
// parking_lot::Mutex provides Sync.
unsafe impl Send for EventState {}
unsafe impl Sync for EventState {}

/// The raw COM object layout for our event handler.
/// Must start with a vtable pointer (COM convention).
#[repr(C)]
struct RawEventHandler {
    vtable: *const EventHandlerVtbl,
    ref_count: AtomicU32,
    shared: Arc<EventState>,
    our_pid: u32,
}

/// COM vtable for IUIAutomationEventHandler (inherits from IUnknown)
#[repr(C)]
struct EventHandlerVtbl {
    // IUnknown methods
    query_interface: unsafe extern "system" fn(
        this: *mut RawEventHandler,
        riid: *const GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> i32,
    add_ref: unsafe extern "system" fn(this: *mut RawEventHandler) -> u32,
    release: unsafe extern "system" fn(this: *mut RawEventHandler) -> u32,
    // IUIAutomationEventHandler method
    handle_automation_event: unsafe extern "system" fn(
        this: *mut RawEventHandler,
        sender: *mut std::ffi::c_void, // IUIAutomationElement
        event_id: i32,                 // UIA_EVENT_ID
    ) -> i32,
}

// Static vtable — one instance shared by all handler objects
static EVENT_HANDLER_VTBL: EventHandlerVtbl = EventHandlerVtbl {
    query_interface: raw_query_interface,
    add_ref: raw_add_ref,
    release: raw_release,
    handle_automation_event: raw_handle_automation_event,
};

unsafe extern "system" fn raw_query_interface(
    this: *mut RawEventHandler,
    riid: *const GUID,
    ppv: *mut *mut std::ffi::c_void,
) -> i32 {
    let riid = &*riid;
    if *riid == IID_IUNKNOWN || *riid == IID_IUIAUTOMATION_EVENT_HANDLER {
        raw_add_ref(this);
        *ppv = this as *mut std::ffi::c_void;
        S_OK.0
    } else {
        *ppv = std::ptr::null_mut();
        // E_NOINTERFACE
        0x80004002_u32 as i32
    }
}

unsafe extern "system" fn raw_add_ref(this: *mut RawEventHandler) -> u32 {
    let handler = &*this;
    handler.ref_count.fetch_add(1, Ordering::Relaxed) + 1
}

unsafe extern "system" fn raw_release(this: *mut RawEventHandler) -> u32 {
    let handler = &*this;
    let prev = handler.ref_count.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        std::sync::atomic::fence(Ordering::Acquire);
        // Drop the Box — ref count reached zero
        drop(Box::from_raw(this));
    }
    prev - 1
}

unsafe extern "system" fn raw_handle_automation_event(
    this: *mut RawEventHandler,
    sender: *mut std::ffi::c_void,
    event_id: i32,
) -> i32 {
    let handler = &*this;

    if event_id == UIA_Text_TextSelectionChangedEventId.0 {
        if !sender.is_null() {
            // Safety: `sender` is a borrowed COM pointer from the UIA callback.
            // `from_raw` takes ownership without AddRef, so we wrap in ManuallyDrop
            // to prevent Release when it goes out of scope (COM still owns this ref).
            // We then clone() to get our own AddRef'd reference for caching.
            let borrowed = std::mem::ManuallyDrop::new(
                IUIAutomationElement::from_raw(sender),
            );

            let sender_pid = borrowed.CurrentProcessId().unwrap_or(0) as u32;
            if sender_pid == handler.our_pid {
                trace!("TextSelectionChanged from own process — ignoring");
                return S_OK.0;
            }

            debug!(
                "TextSelectionChanged event from pid={}",
                sender_pid
            );

            // clone() does AddRef — this is our owned reference to cache
            let cached = (*borrowed).clone();
            *handler.shared.event_element.lock() = Some(cached);
            handler.shared.has_event.store(true, Ordering::Release);
        }
    }

    S_OK.0
}

/// Create a manual COM event handler and return it as IUIAutomationEventHandler.
fn create_event_handler(shared: Arc<EventState>, our_pid: u32) -> IUIAutomationEventHandler {
    let handler = Box::new(RawEventHandler {
        vtable: &EVENT_HANDLER_VTBL,
        ref_count: AtomicU32::new(1), // start with ref_count = 1
        shared,
        our_pid,
    });
    let raw_ptr = Box::into_raw(handler);
    // Safety: our vtable layout matches IUIAutomationEventHandler's COM interface.
    // The first field is the vtable pointer, which is what COM expects.
    unsafe { IUIAutomationEventHandler::from_raw(raw_ptr as *mut std::ffi::c_void) }
}

// ═══════════════════════════════════════════════════════════════════════
// UiaEngine — main API
// ═══════════════════════════════════════════════════════════════════════

/// Wrapper around UI Automation COM interfaces
pub struct UiaEngine {
    automation: IUIAutomation,
    /// TreeWalker for traversing the UIA element tree
    tree_walker: IUIAutomationTreeWalker,
    /// Cached element that previously had a text selection.
    /// Re-checked on subsequent polls to avoid flickering when mouse moves away.
    cached_element: std::cell::RefCell<Option<IUIAutomationElement>>,
    /// Our process ID — used to skip our own popup overlay
    our_pid: u32,
    /// Shared state with the TextSelectionChanged event handler
    event_state: Arc<EventState>,
    /// Whether the event handler was successfully registered
    event_handler_active: bool,
}

impl UiaEngine {
    /// Initialize COM and create the UIAutomation object + TreeWalker.
    /// Also registers a TextSelectionChanged event handler on the desktop root.
    pub fn new() -> Result<Self> {
        unsafe {
            // STA (Single-Threaded Apartment) is required for UIA:
            // UIA interacts with UI elements across processes via COM proxies,
            // which need a message pump on the calling thread. STA provides this.
            // The selection monitor runs on a dedicated OS thread, satisfying STA.
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .context("Failed to initialize COM")?;

            let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL)
                .context("Failed to create IUIAutomation instance")?;

            let tree_walker = automation
                .ControlViewWalker()
                .context("Failed to get ControlViewWalker")?;

            let our_pid = GetCurrentProcessId();

            let event_state = Arc::new(EventState {
                has_event: AtomicBool::new(false),
                event_element: parking_lot::Mutex::new(None),
                selection_cleared: AtomicBool::new(false),
            });

            // Register TextSelectionChanged event handler on the desktop root element
            let event_handler_active =
                match Self::register_event_handler(&automation, &event_state, our_pid) {
                    Ok(()) => {
                        info!("TextSelectionChanged event handler registered successfully");
                        true
                    }
                    Err(e) => {
                        info!(
                            "TextSelectionChanged event handler registration failed: {}. \
                             Relying on polling strategies only.",
                            e
                        );
                        false
                    }
                };

            Ok(Self {
                automation,
                tree_walker,
                cached_element: std::cell::RefCell::new(None),
                our_pid,
                event_state,
                event_handler_active,
            })
        }
    }

    /// Register the TextSelectionChanged event handler on the desktop root element
    unsafe fn register_event_handler(
        automation: &IUIAutomation,
        event_state: &Arc<EventState>,
        our_pid: u32,
    ) -> Result<()> {
        let root = automation
            .GetRootElement()
            .context("Failed to get root element")?;

        let handler = create_event_handler(Arc::clone(event_state), our_pid);

        automation
            .AddAutomationEventHandler(
                UIA_Text_TextSelectionChangedEventId,
                &root,
                TreeScope_Subtree,
                None, // no cache request
                &handler,
            )
            .context("AddAutomationEventHandler failed")?;

        Ok(())
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
            if r.width > 10 && r.height > 10 {
                Some(r)
            } else {
                None
            }
        }
    }

    /// Clear the cached UIA element (e.g., when foreground window changes).
    pub fn clear_cache(&self) {
        *self.cached_element.borrow_mut() = None;
    }

    /// Get the bounding rectangle of the currently selected text range.
    /// Returns the union of all bounding rectangles (multi-line selections produce multiple rects).
    /// Coordinates are in physical pixels.
    pub fn get_selection_rect(&self) -> Option<ElementRect> {
        // Try cached element first, then focused element
        let element = self.cached_element.borrow().clone()
            .or_else(|| self.get_focused_element().ok());
        let element = element?;
        unsafe {
            let pattern_obj = element.GetCurrentPattern(UIA_TextPatternId).ok()?;
            let text_pattern: IUIAutomationTextPattern = pattern_obj.cast().ok()?;
            let selection = text_pattern.GetSelection().ok()?;
            if selection.Length().unwrap_or(0) == 0 {
                return None;
            }
            let range = selection.GetElement(0).ok()?;
            let rects_sa = range.GetBoundingRectangles().ok()?;

            // SAFEARRAY of doubles: each rect is [x, y, width, height] (4 doubles per rect)
            let mut data_ptr: *mut core::ffi::c_void = core::ptr::null_mut();
            windows::Win32::System::Ole::SafeArrayAccessData(rects_sa, &mut data_ptr).ok()?;

            // RAII guard: ensure SafeArrayUnaccessData is always called
            struct SafeArrayGuard(*const windows::Win32::System::Com::SAFEARRAY);
            impl Drop for SafeArrayGuard {
                fn drop(&mut self) {
                    unsafe {
                        let _ = windows::Win32::System::Ole::SafeArrayUnaccessData(self.0);
                    }
                }
            }
            let _sa_guard = SafeArrayGuard(rects_sa);

            let num_elements = {
                let lower = windows::Win32::System::Ole::SafeArrayGetLBound(rects_sa, 1).unwrap_or(0);
                let upper = windows::Win32::System::Ole::SafeArrayGetUBound(rects_sa, 1).unwrap_or(-1);
                if upper < lower {
                    return None; // _sa_guard drops, calling SafeArrayUnaccessData
                }
                (upper - lower + 1) as usize
            };
            if num_elements == 0 || data_ptr.is_null() {
                return None;
            }
            let doubles = core::slice::from_raw_parts(data_ptr as *const f64, num_elements);

            // Compute the union bounding box of all rectangles
            let mut min_x = f64::MAX;
            let mut min_y = f64::MAX;
            let mut max_x = f64::MIN;
            let mut max_y = f64::MIN;
            for chunk in doubles.chunks(4) {
                if chunk.len() == 4 {
                    let (rx, ry, rw, rh): (f64, f64, f64, f64) = (chunk[0], chunk[1], chunk[2], chunk[3]);
                    if rw > 0.0 && rh > 0.0 {
                        min_x = min_x.min(rx);
                        min_y = min_y.min(ry);
                        max_x = max_x.max(rx + rw);
                        max_y = max_y.max(ry + rh);
                    }
                }
            }

            // _sa_guard drops here, calling SafeArrayUnaccessData

            if max_x > min_x && max_y > min_y {
                Some(ElementRect {
                    x: min_x as i32,
                    y: min_y as i32,
                    width: (max_x - min_x) as i32,
                    height: (max_y - min_y) as i32,
                })
            } else {
                None
            }
        }
    }

    /// Try to get selected text from the focused element using TextPattern
    /// Only returns text if the focused element is an editable control
    pub fn get_selected_text(&self) -> Result<Option<String>> {
        let element = match self.get_focused_element() {
            Ok(el) => el,
            Err(e) => {
                trace!("No focused element: {}", e);
                return Ok(None);
            }
        };

        if !self.is_editable_element(&element) {
            trace!("Focused element is not editable — skipping");
            return Ok(None);
        }

        self.get_text_from_element(&element)
    }

    /// Try to get selected text from ANY element (input or non-input).
    /// Returns (Option<text>, is_input_element).
    ///
    /// Two-phase detection:
    ///
    /// **Phase 1 - Write Mode (v0.6.0 golden path):**
    /// GetFocusedElement -> is editable? -> has selection? -> Write Mode.
    /// This is the most reliable: if cursor is in an input box, the focused
    /// element IS the input box.
    ///
    /// **Phase 2 - Read Mode (only when focused element is NOT editable):**
    /// Cached -> Event -> Focused(non-editable) -> ElementFromPoint -> TreeWalker.
    /// Everything found here is is_input=false (Read Mode).
    pub fn get_selected_text_any(&self) -> Result<(Option<String>, bool)> {
        // Get focused element ONCE — this is a cross-process COM call (1-10ms).
        // Reused across Phase 1, cached element validation, and Phase 2 fallback.
        let focused = self.get_focused_element().ok();

        // =================================================================
        // Phase 1: Write Mode -- check focused element first
        // If focus is on an editable element with selected text -> Write Mode
        // =================================================================
        if let Some(ref focused_el) = focused {
            if self.is_editable_element(focused_el) {
                if let Ok(Some(text)) = self.get_text_from_element(focused_el) {
                    self.log_element_info(focused_el, true, "focused-write");
                    *self.cached_element.borrow_mut() = Some(focused_el.clone());
                    // Consume any pending event to avoid stale detection
                    if self.event_handler_active {
                        self.event_state.has_event.swap(false, Ordering::AcqRel);
                        self.event_state.event_element.lock().take();
                    }
                    return Ok((Some(text), true));
                }
            } else {
                // Phase 1 miss: focused element is not editable — log for debugging
                unsafe {
                    let ct = focused_el.CurrentControlType().unwrap_or_default();
                    let name = focused_el.CurrentName().unwrap_or_default();
                    let class = focused_el.CurrentClassName().unwrap_or_default();
                    debug!("Phase 1 skip: focused not editable — type={}, name={:?}, class={:?}", ct.0, name.to_string(), class.to_string());
                }
            }
        }

        // =================================================================
        // Phase 2: focused element is not editable (or no focus)
        // Still check is_editable_element on whatever we find
        // =================================================================

        // Check TextSelectionChanged event
        if self.event_handler_active
            && self.event_state.has_event.swap(false, Ordering::AcqRel)
        {
            let element = self.event_state.event_element.lock().take();
            if let Some(element) = element {
                match self.get_text_from_element(&element) {
                    Ok(Some(_text)) => {
                        debug!("TextSelectionChanged event: element has selection, caching");
                        *self.cached_element.borrow_mut() = Some(element);
                    }
                    _ => {
                        debug!("TextSelectionChanged event: no text, falling through");
                        *self.cached_element.borrow_mut() = None;
                    }
                }
            }
        }

        // Re-check cached element (hot path for sustained selection)
        // But only if the cached element is still relevant — for editable elements,
        // verify it's still focused. Browsers keep selection state on unfocused inputs,
        // which would cause stale popup if we don't check.
        {
            let cached = self.cached_element.borrow();
            if let Some(ref element) = *cached {
                let is_input = self.is_editable_element(element);
                if is_input {
                    // Editable cached element: only trust it if it matches the current focused element
                    if let Some(ref focused_el) = focused {
                        let same = unsafe {
                            self.automation.CompareElements(element, focused_el).unwrap_or_default().as_bool()
                        };
                        if same {
                            if let Ok(Some(text)) = self.get_text_from_element(element) {
                                self.log_element_info(element, true, "cached-focused");
                                return Ok((Some(text), true));
                            }
                        } else {
                            debug!("Cached editable element lost focus — clearing cache");
                        }
                    }
                } else {
                    // Non-editable cached element: trust as-is (Read Mode)
                    if let Ok(Some(text)) = self.get_text_from_element(element) {
                        self.log_element_info(element, false, "cached");
                        return Ok((Some(text), false));
                    }
                }
            }
        }
        *self.cached_element.borrow_mut() = None;

        // Reuse focused element for non-editable selection check
        if let Some(focused_el) = focused {
            if let Ok(Some(text)) = self.get_text_from_element(&focused_el) {
                let is_input = self.is_editable_element(&focused_el);
                self.log_element_info(&focused_el, is_input, "focused");
                *self.cached_element.borrow_mut() = Some(focused_el);
                return Ok((Some(text), is_input));
            }
        }

        // ElementFromPoint at mouse cursor
        if let Some(element) = self.get_element_at_cursor() {
            if let Ok(Some(text)) = self.get_text_from_element(&element) {
                let is_input = self.is_editable_element(&element);
                self.log_element_info(&element, is_input, "point");
                *self.cached_element.borrow_mut() = Some(element);
                return Ok((Some(text), is_input));
            }
        }

        // TreeWalker -- traverse foreground window UIA subtree (last resort)
        if let Some((element, text)) = self.find_selection_in_foreground_window() {
            let is_input = self.is_editable_element(&element);
            self.log_element_info(&element, is_input, "tree");
            *self.cached_element.borrow_mut() = Some(element);
            return Ok((Some(text), is_input));
        }

        Ok((None, false))
    }
    /// Traverse the foreground window's UIA subtree to find an element with
    /// a TextPattern that has an active text selection.
    fn find_selection_in_foreground_window(&self) -> Option<(IUIAutomationElement, String)> {
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.0.is_null() {
                return None;
            }

            let root = self.automation.ElementFromHandle(hwnd).ok()?;

            let root_pid = root.CurrentProcessId().unwrap_or(0) as u32;
            if root_pid == self.our_pid {
                return None;
            }

            self.walk_tree_for_selection(&root, 0)
        }
    }

    /// Recursively walk the UIA tree looking for an element with TextPattern + selection.
    /// Prefers editable elements (Write Mode) over non-editable (Read Mode).
    /// If a non-editable element has selection, remember it but keep looking
    /// for an editable child that also has selection.
    fn walk_tree_for_selection(
        &self,
        element: &IUIAutomationElement,
        depth: u32,
    ) -> Option<(IUIAutomationElement, String)> {
        if depth > TREE_WALKER_MAX_DEPTH {
            return None;
        }

        // Check current element for selection
        let mut non_editable_result: Option<(IUIAutomationElement, String)> = None;
        if let Ok(Some(text)) = self.get_text_from_element(element) {
            if self.is_editable_element(element) {
                // Found an editable element with selection — best case, return immediately
                return Some((element.clone(), text));
            }
            // Non-editable with selection — remember but keep searching children
            non_editable_result = Some((element.clone(), text));
        }

        // Search children for a better (editable) match
        unsafe {
            let child = match self.tree_walker.GetFirstChildElement(element) {
                Ok(c) => c,
                Err(_) => return non_editable_result,
            };

            let mut current = child;
            loop {
                if let Some(result) = self.walk_tree_for_selection(&current, depth + 1) {
                    if self.is_editable_element(&result.0) {
                        // Found editable child — prefer it
                        return Some(result);
                    }
                    // Non-editable child result — only use if we don't have one yet
                    if non_editable_result.is_none() {
                        non_editable_result = Some(result);
                    }
                }

                match self.tree_walker.GetNextSiblingElement(&current) {
                    Ok(next) => current = next,
                    Err(_) => break,
                }
            }
        }

        non_editable_result
    }

    /// Get UIA element at the current mouse cursor position.
    fn get_element_at_cursor(&self) -> Option<IUIAutomationElement> {
        unsafe {
            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_err() {
                return None;
            }
            let element = self.automation.ElementFromPoint(point).ok()?;

            let element_pid = element.CurrentProcessId().unwrap_or(0) as u32;
            if element_pid == self.our_pid {
                trace!("ElementFromPoint hit our own process — skipping");
                return None;
            }

            Some(element)
        }
    }

    /// Log element info (only called when text selection is found)
    fn log_element_info(&self, element: &IUIAutomationElement, is_input: bool, source: &str) {
        unsafe {
            let control_type = element.CurrentControlType().unwrap_or_default();
            let class_name = element.CurrentClassName().unwrap_or_default();
            debug!(
                "UIA selection [{}]: type={}, class='{}', is_input={}",
                source, control_type.0, class_name, is_input
            );
        }
    }

    /// Check if a UIA element is an editable control
    fn is_editable_element(&self, element: &IUIAutomationElement) -> bool {
        unsafe {
            let control_type = element.CurrentControlType().unwrap_or_default();

            // Edit controls are always editable (input fields, textareas)
            if control_type == UIA_EditControlTypeId {
                return true;
            }

            // For other controls: check ValuePattern + IsReadOnly
            // A contenteditable div = Document + ValuePattern(IsReadOnly=false)
            // A plain webpage = Document + ValuePattern(IsReadOnly=true) OR no ValuePattern
            if let Ok(pattern_obj) = element.GetCurrentPattern(UIA_ValuePatternId) {
                if let Ok(value_pattern) = pattern_obj
                    .cast::<windows::Win32::UI::Accessibility::IUIAutomationValuePattern>()
                {
                    if let Ok(is_readonly) = value_pattern.CurrentIsReadOnly() {
                        if is_readonly.as_bool() {
                            return false;
                        }
                        return true;
                    }
                }
                // ValuePattern exists but couldn't check IsReadOnly — assume editable
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
                Err(_) => return Ok(None),
            };

            let text_pattern: IUIAutomationTextPattern = match pattern_obj.cast() {
                Ok(tp) => tp,
                Err(_) => return Ok(None),
            };

            let selection = match text_pattern.GetSelection() {
                Ok(s) => s,
                Err(_) => return Ok(None),
            };

            let length = selection.Length().unwrap_or(0);
            if length == 0 {
                return Ok(None);
            }

            let range = match selection.GetElement(0) {
                Ok(r) => r,
                Err(e) => {
                    debug!("Failed to get selection range: {}", e);
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

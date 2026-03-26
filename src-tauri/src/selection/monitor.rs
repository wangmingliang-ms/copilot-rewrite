// Selection monitor - orchestrates UIA polling and clipboard fallback
// Runs in a background thread, emitting selection events to the frontend

use crate::{AppState, SelectionInfo, SelectionSource};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION};

use super::uia::UiaEngine;
use crate::overlay;

/// Get the process name and window title for a given HWND
fn get_window_context(hwnd: isize) -> (String, String) {
    let h = windows::Win32::Foundation::HWND(hwnd as *mut _);

    // Window title
    let window_title = {
        let mut buf = [0u16; 512];
        let len = unsafe { GetWindowTextW(h, &mut buf) } as usize;
        String::from_utf16_lossy(&buf[..len])
    };

    // Process name
    let app_name = {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if pid > 0 {
            if let Ok(process) = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
                let mut buf = [0u16; 512];
                let mut size = buf.len() as u32;
                if unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_FORMAT(0), windows::core::PWSTR(buf.as_mut_ptr()), &mut size) }.is_ok() {
                    let path = String::from_utf16_lossy(&buf[..size as usize]);
                    // Extract just the filename without extension
                    path.rsplit('\\').next().unwrap_or(&path)
                        .strip_suffix(".exe").unwrap_or(&path)
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        }
    };

    (app_name, window_title)
}

/// Minimum time between showing the popup (debounce)
const DEBOUNCE_MS: u64 = 200;
/// Minimum text length to trigger popup
const MIN_TEXT_LENGTH: usize = 1;
/// Maximum text length to process (avoid huge payloads)
const MAX_TEXT_LENGTH: usize = 5000;

/// Start the selection monitoring engine
/// This runs in a dedicated background thread and polls for text selections
pub fn start_selection_engine(app_handle: AppHandle, state: Arc<AppState>) {
    info!("Selection engine starting...");

    // Initialize UIA on this thread (COM must be initialized per-thread)
    let uia = match UiaEngine::new() {
        Ok(engine) => {
            info!("UIA engine initialized successfully");
            Some(engine)
        }
        Err(e) => {
            error!("Failed to initialize UIA engine: {}. Using clipboard fallback only.", e);
            None
        }
    };

    let mut last_selection: Option<String> = None;
    let mut last_change_time = Instant::now();
    let mut debounce_text: Option<String> = None;
    let mut seen_generation = state.selection_generation.load(std::sync::atomic::Ordering::Relaxed);

    loop {
        // Check if monitoring is enabled
        if !*state.enabled.lock() {
            // Clear state so same text can re-trigger after re-enable
            last_selection = None;
            debounce_text = None;
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        // Check if dismiss happened (generation bumped)
        let current_gen = state.selection_generation.load(std::sync::atomic::Ordering::Relaxed);
        if current_gen != seen_generation {
            info!("Generation changed ({} → {}) — resetting monitor state (popup was dismissed)", seen_generation, current_gen);
            last_selection = None;
            debounce_text = None;
            seen_generation = current_gen;
        }

        let mut preview_is_visible = *state.preview_visible.lock();
        let poll_interval = state.settings.lock().poll_interval_ms;

        // Safety check: if preview_visible flag is stuck but popup is actually hidden, auto-reset
        if preview_is_visible {
            if let Some(popup) = app_handle.get_webview_window("popup") {
                if let Ok(visible) = popup.is_visible() {
                    if !visible {
                        warn!("preview_visible was true but popup is hidden — auto-resetting");
                        *state.preview_visible.lock() = false;
                        preview_is_visible = false;
                    }
                }
            }
        }

        // Try to get selected text via UIA
        // Skip if the foreground window is our own popup (e.g., user selecting text in preview)
        let is_own_window = {
            let fg = unsafe { GetForegroundWindow().0 as isize };
            if let Some(popup) = app_handle.get_webview_window("popup") {
                popup.hwnd().map(|h| h.0 as isize == fg).unwrap_or(false)
            } else {
                false
            }
        };

        let selected_text = if is_own_window {
            // Don't poll UIA when our own popup is focused — preserve current state
            trace!("Skipping UIA poll — own popup is foreground");
            std::thread::sleep(Duration::from_millis(poll_interval));
            continue;
        } else if let Some(ref uia_engine) = uia {
            match uia_engine.get_selected_text() {
                Ok(text) => text,
                Err(e) => {
                    debug!("UIA selection check error: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Process the selection
        if let Some(text) = selected_text {
            // Validate text
            if text.len() >= MIN_TEXT_LENGTH && text.len() <= MAX_TEXT_LENGTH {
                let text_trimmed = text.trim().to_string();

                if !text_trimmed.is_empty() {
                    if preview_is_visible {
                        // Popup is expanded — don't trigger new popups
                        last_selection = Some(text_trimmed);
                    } else {
                        // Normal mode: detect changes and show popup icon
                        let text_changed = last_selection.as_ref() != Some(&text_trimmed);

                        if text_changed {
                            info!("Selection changed: {} chars (was {} chars)",
                                text_trimmed.len(),
                                last_selection.as_ref().map(|s| s.len()).unwrap_or(0));
                            debounce_text = Some(text_trimmed.clone());
                            last_change_time = Instant::now();
                            last_selection = Some(text_trimmed);
                        } else if let Some(ref debounced) = debounce_text {
                            if last_change_time.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                                // Debounce complete — show popup icon
                                let mouse_pos = get_cursor_position();
                                let source_hwnd = unsafe { GetForegroundWindow().0 as isize };
                                let (app_name, window_title) = get_window_context(source_hwnd);

                                // Get bounding rect of the focused input element
                                let input_rect = if let Some(ref uia_engine) = uia {
                                    uia_engine.get_focused_element_rect()
                                        .map(|r| (r.x, r.y, r.width, r.height))
                                } else {
                                    None
                                };

                                let selection_info = SelectionInfo {
                                    text: debounced.clone(),
                                    mouse_x: mouse_pos.0,
                                    mouse_y: mouse_pos.1,
                                    source: SelectionSource::UIA,
                                    source_hwnd: Some(source_hwnd),
                                    input_rect,
                                    app_name,
                                    window_title,
                                };

                                show_popup(app_handle.clone(), &state, selection_info);
                                debounce_text = None;
                            }
                        }
                    }
                }
            }
        } else {
            // No selection detected
            if last_selection.is_some() && !preview_is_visible {
                info!("Selection cleared — hiding popup (was {} chars)",
                    last_selection.as_ref().map(|s| s.len()).unwrap_or(0));
                last_selection = None;
                debounce_text = None;
                overlay::hide_popup(&app_handle);
                *state.current_selection.lock() = None;
            }
            // If preview_is_visible: do nothing — let dismiss_popup handle cleanup
        }

        std::thread::sleep(Duration::from_millis(poll_interval));
    }
}

/// Get the current cursor position using Win32 API
fn get_cursor_position() -> (i32, i32) {
    unsafe {
        let mut point = POINT::default();
        if GetCursorPos(&mut point).is_ok() {
            (point.x, point.y)
        } else {
            (0, 0)
        }
    }
}

/// Show the popup icon near the mouse cursor
fn show_popup(app_handle: AppHandle, state: &Arc<AppState>, info: SelectionInfo) {
    let preview: String = info.text.chars().take(50).collect();
    info!(
        "Showing popup icon at ({}, {}) for text: {}...",
        info.mouse_x,
        info.mouse_y,
        preview
    );

    // Store the current selection in state
    *state.current_selection.lock() = Some(info.clone());

    // Show popup icon at cursor position (with input rect for smart positioning)
    overlay::show_popup_icon(&app_handle, info.mouse_x, info.mouse_y, info.input_rect);

    // Emit event to frontend with the selection info
    if let Err(e) = app_handle.emit("selection-detected", &info) {
        warn!("Failed to emit selection event: {}", e);
    }
}

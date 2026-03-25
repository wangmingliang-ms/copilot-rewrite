// Selection monitor - orchestrates UIA polling and clipboard fallback
// Runs in a background thread, emitting selection events to the frontend

use crate::{AppState, SelectionInfo, SelectionSource};
use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tauri::AppHandle;
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

use super::uia::UiaEngine;
use crate::overlay;

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

    loop {
        // Check if monitoring is enabled
        if !*state.enabled.lock() {
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        let preview_is_visible = *state.preview_visible.lock();
        let poll_interval = state.settings.lock().poll_interval_ms;

        // Try to get selected text via UIA
        let selected_text = if let Some(ref uia_engine) = uia {
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
                            debounce_text = Some(text_trimmed.clone());
                            last_change_time = Instant::now();
                            last_selection = Some(text_trimmed);
                        } else if let Some(ref debounced) = debounce_text {
                            if last_change_time.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                                // Debounce complete — show popup icon
                                let mouse_pos = get_cursor_position();
                                let source_hwnd = unsafe { GetForegroundWindow().0 as isize };
                                let selection_info = SelectionInfo {
                                    text: debounced.clone(),
                                    mouse_x: mouse_pos.0,
                                    mouse_y: mouse_pos.1,
                                    source: SelectionSource::UIA,
                                    source_hwnd: Some(source_hwnd),
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
                // Only hide if popup is NOT in expanded/spinning state.
                // When user clicks the expanded popup, the source app loses focus
                // and UIA returns no selection — but we must keep the popup alive.
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
    debug!(
        "Showing popup icon at ({}, {}) for text: {}...",
        info.mouse_x,
        info.mouse_y,
        &info.text[..info.text.len().min(50)]
    );

    // Store the current selection in state
    *state.current_selection.lock() = Some(info.clone());

    // Show popup icon at cursor position
    overlay::show_popup_icon(&app_handle, info.mouse_x, info.mouse_y);

    // Emit event to frontend with the selection info
    if let Err(e) = app_handle.emit("selection-detected", &info) {
        warn!("Failed to emit selection event: {}", e);
    }
}

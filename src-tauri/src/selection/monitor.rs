// Selection monitor - orchestrates UIA polling and clipboard fallback
// Runs in a background thread, emitting selection events to the frontend

use crate::{AppState, SelectionInfo, SelectionSource};
use log::{debug, error, info, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::POINT;
use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

use super::uia::UiaEngine;
use crate::overlay;

/// Minimum time between showing the toolbar (debounce)
const DEBOUNCE_MS: u64 = 200;
/// Minimum text length to trigger toolbar
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

        // Skip monitoring while preview is visible (it would steal focus and
        // cause UIA to think the selection disappeared)
        if *state.preview_visible.lock() {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }

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
                    // Check if text has changed
                    let text_changed = last_selection.as_ref() != Some(&text_trimmed);

                    if text_changed {
                        // Text changed - start debounce timer
                        debounce_text = Some(text_trimmed.clone());
                        last_change_time = Instant::now();
                        last_selection = Some(text_trimmed);
                        // Selection changed — hide preview from previous translation
                        overlay::hide_preview(&app_handle);
                    } else if let Some(ref debounced) = debounce_text {
                        // Text stabilized - check debounce timer
                        if last_change_time.elapsed() >= Duration::from_millis(DEBOUNCE_MS) {
                            // Debounce complete - show toolbar!
                            let mouse_pos = get_cursor_position();
                            let source_hwnd = unsafe { GetForegroundWindow().0 as isize };
                            let selection_info = SelectionInfo {
                                text: debounced.clone(),
                                mouse_x: mouse_pos.0,
                                mouse_y: mouse_pos.1,
                                source: SelectionSource::UIA,
                                source_hwnd: Some(source_hwnd),
                            };

                            show_toolbar(&app_handle, &state, selection_info);
                            debounce_text = None;
                        }
                    }
                }
            }
        } else {
            // No selection detected - clear state
            if last_selection.is_some() {
                last_selection = None;
                debounce_text = None;
                // Hide toolbar AND preview when selection is cleared
                overlay::hide_all(&app_handle);
                *state.current_selection.lock() = None;
            }
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

/// Show the floating toolbar near the mouse cursor
fn show_toolbar(app_handle: &AppHandle, state: &Arc<AppState>, info: SelectionInfo) {
    debug!(
        "Showing toolbar at ({}, {}) for text: {}...",
        info.mouse_x,
        info.mouse_y,
        &info.text[..info.text.len().min(50)]
    );

    // Store the current selection in state
    *state.current_selection.lock() = Some(info.clone());

    // Use overlay module for proper positioning with DPI-aware coordinates
    overlay::show_toolbar_at(app_handle, info.mouse_x, info.mouse_y);

    // Emit event to frontend with the selection info
    if let Err(e) = app_handle.emit("selection-detected", &info) {
        warn!("Failed to emit selection event: {}", e);
    }
}

/// Hide the floating toolbar
fn hide_toolbar(app_handle: &AppHandle) {
    overlay::hide_toolbar(app_handle);
}

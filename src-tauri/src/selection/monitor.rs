// Selection monitor - orchestrates UIA polling and clipboard fallback
// Runs in a background thread, emitting selection events to the frontend

use crate::{AppState, SelectionInfo, SelectionSource};
use log::{debug, error, info, trace, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::POINT;
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
};

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
            if let Ok(process) =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
            {
                let mut buf = [0u16; 512];
                let mut size = buf.len() as u32;
                if unsafe {
                    QueryFullProcessImageNameW(
                        process,
                        PROCESS_NAME_FORMAT(0),
                        windows::core::PWSTR(buf.as_mut_ptr()),
                        &mut size,
                    )
                }
                .is_ok()
                {
                    let path = String::from_utf16_lossy(&buf[..size as usize]);
                    path.rsplit('\\')
                        .next()
                        .unwrap_or(&path)
                        .strip_suffix(".exe")
                        .unwrap_or(&path)
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

/// Debounce time for keyboard selections (mouse bypasses this)
const DEBOUNCE_MS: u64 = 100;
/// Minimum text length to trigger popup
const MIN_TEXT_LENGTH: usize = 1;
/// Maximum text length to process
const MAX_TEXT_LENGTH: usize = 5000;

/// Start the selection monitoring engine
pub fn start_selection_engine(app_handle: AppHandle, state: Arc<AppState>) {
    info!("Selection engine starting...");

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
    let mut last_is_input = true;
    let mut selection_source_hwnd: isize = 0;
    let mut popup_icon_visible = false;
    let mut mouse_idle_after_popup = false; // For Read Mode dismiss
    let mut mouse_selecting = false;        // Track drag state
    let mut mouseup_pending = false;        // Persists across iterations until consumed

    let mut seen_generation = state
        .selection_generation
        .load(std::sync::atomic::Ordering::Relaxed);
    let mut last_health_check = Instant::now();
    let health_check_interval = Duration::from_secs(300);

    loop {
        // ── Health check (every 5 min) ──────────────────────────────────
        if last_health_check.elapsed() >= health_check_interval {
            last_health_check = Instant::now();
            if let Some(popup) = app_handle.get_webview_window("popup") {
                match popup.hwnd() {
                    Ok(hwnd) => {
                        let hwnd_win = windows::Win32::Foundation::HWND(hwnd.0 as *mut _);
                        let valid = unsafe {
                            windows::Win32::UI::WindowsAndMessaging::IsWindow(hwnd_win).as_bool()
                        };
                        if !valid {
                            error!("Health check: popup HWND is INVALID");
                        } else {
                            info!("Health check: popup window OK (HWND=0x{:X})", hwnd.0 as isize);
                        }
                    }
                    Err(e) => error!("Health check: cannot get popup HWND: {}", e),
                }
                match popup.eval("window.__healthcheck = Date.now()") {
                    Ok(_) => info!("Health check: WebView2 renderer responsive"),
                    Err(e) => error!("Health check: WebView2 eval FAILED: {}", e),
                }
            } else {
                error!("Health check: popup webview window NOT FOUND");
            }
        }

        // ── Monitoring enabled? ─────────────────────────────────────────
        if !*state.enabled.lock() {
            last_selection = None;
            debounce_text = None;
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        // ── Generation check (external dismiss) ────────────────────────
        let current_gen = state
            .selection_generation
            .load(std::sync::atomic::Ordering::Relaxed);
        if current_gen != seen_generation {
            info!("Generation changed ({} → {}) — resetting monitor state", seen_generation, current_gen);
            last_selection = None;
            debounce_text = None;
            seen_generation = current_gen;
        }

        let mut preview_is_visible = *state.preview_visible.lock();
        let poll_interval = state.settings.lock().poll_interval_ms;

        // ── Safety: preview_visible stuck but popup hidden ──────────────
        if preview_is_visible {
            if let Some(popup) = app_handle.get_webview_window("popup") {
                if let Ok(visible) = popup.is_visible() {
                    if !visible {
                        warn!("preview_visible stuck — auto-resetting");
                        *state.preview_visible.lock() = false;
                        preview_is_visible = false;
                    }
                }
            }
        }

        // ── Mouse state ─────────────────────────────────────────────────
        let lbutton_now = unsafe { GetAsyncKeyState(0x01) } & (0x8000u16 as i16) != 0;

        // ── Read Mode mouse-click dismiss ───────────────────────────────
        // When popup icon is visible for non-input (Read Mode), detect click to dismiss.
        // UIA retains stale selection in Read Mode, so mouse is the dismiss signal.
        if popup_icon_visible && !preview_is_visible && !last_is_input {
            if !mouse_idle_after_popup {
                if !lbutton_now {
                    mouse_idle_after_popup = true;
                    debug!("Read Mode dismiss: mouse idle, watching for next click");
                }
            } else if lbutton_now {
                let click_on_popup = {
                    if let Some(popup) = app_handle.get_webview_window("popup") {
                        if let (Ok(pos), Ok(size)) = (popup.outer_position(), popup.outer_size()) {
                            let mut cursor = POINT::default();
                            let _ = unsafe { GetCursorPos(&mut cursor) };
                            cursor.x >= pos.x && cursor.x <= pos.x + size.width as i32
                                && cursor.y >= pos.y && cursor.y <= pos.y + size.height as i32
                        } else { false }
                    } else { false }
                };
                if !click_on_popup {
                    info!("Read Mode: mousedown — dismissing popup icon");
                    last_selection = None;
                    debounce_text = None;
                    popup_icon_visible = false;
                    mouse_idle_after_popup = false;
                    selection_source_hwnd = 0;
                    overlay::hide_popup(&app_handle);
                    *state.current_selection.lock() = None;
                    if let Some(ref uia_engine) = uia {
                        uia_engine.clear_cache();
                    }
                    std::thread::sleep(Duration::from_millis(poll_interval));
                    continue;
                }
            }
        } else {
            mouse_idle_after_popup = false;
        }

        // ── Mouseup detection (persistent flag) ─────────────────────────
        // mouseup_pending stays true until consumed by a show_popup call.
        // This handles the case where UIA takes 1-2 loop iterations to
        // detect the selection after mouseup.
        if !popup_icon_visible && !preview_is_visible {
            if lbutton_now && !mouse_selecting {
                mouse_selecting = true;
            } else if !lbutton_now && mouse_selecting {
                mouse_selecting = false;
                mouseup_pending = true;
                info!("Mouseup detected — pending instant show");
            }
        } else if !lbutton_now {
            mouse_selecting = false;
        }

        // ── Foreground window check ─────────────────────────────────────
        let current_fg = unsafe { GetForegroundWindow().0 as isize };
        let is_own_window = {
            let is_popup = if let Some(popup) = app_handle.get_webview_window("popup") {
                popup.hwnd().map(|h| h.0 as isize == current_fg).unwrap_or(false)
            } else { false };
            let is_settings = if let Some(settings) = app_handle.get_webview_window("settings") {
                settings.hwnd().map(|h| h.0 as isize == current_fg).unwrap_or(false)
            } else { false };
            is_popup || is_settings
        };

        // Foreground changed → dismiss
        let fg_pid = unsafe {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(windows::Win32::Foundation::HWND(current_fg as *mut _), Some(&mut pid));
            pid
        };
        let source_pid = if selection_source_hwnd != 0 {
            unsafe {
                let mut pid: u32 = 0;
                GetWindowThreadProcessId(windows::Win32::Foundation::HWND(selection_source_hwnd as *mut _), Some(&mut pid));
                pid
            }
        } else { 0 };
        let fg_changed = selection_source_hwnd != 0
            && (current_fg != selection_source_hwnd || (source_pid != 0 && fg_pid != source_pid));

        if (popup_icon_visible || preview_is_visible) && !is_own_window && fg_changed {
            info!(
                "Foreground changed: source=0x{:X}(pid={}) current=0x{:X}(pid={}) — hiding popup",
                selection_source_hwnd, source_pid, current_fg, fg_pid
            );
            last_selection = None;
            debounce_text = None;
            popup_icon_visible = false;
            selection_source_hwnd = 0;
            mouseup_pending = false;
            overlay::hide_popup(&app_handle);
            *state.current_selection.lock() = None;
            *state.preview_visible.lock() = false;
            state.cancel_token.lock().cancel();
            if let Some(ref uia_engine) = uia {
                uia_engine.clear_cache();
            }
            std::thread::sleep(Duration::from_millis(poll_interval));
            continue;
        }

        // ── Own window foreground → skip UIA ────────────────────────────
        if is_own_window {
            trace!("Own popup is foreground — preserving current state");
            std::thread::sleep(Duration::from_millis(poll_interval));
            continue;
        }

        // ── UIA selection check ─────────────────────────────────────────
        let selected_text = if let Some(ref uia_engine) = uia {
            let read_mode_enabled = state.settings.lock().read_mode_enabled;
            match uia_engine.get_selected_text_any() {
                Ok((Some(text), is_input)) => {
                    if is_input {
                        Some((text, true))
                    } else if read_mode_enabled {
                        Some((text, false))
                    } else {
                        None
                    }
                }
                Ok((None, _)) => None,
                Err(e) => {
                    debug!("UIA selection check error: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // ── Process selection ───────────────────────────────────────────
        if let Some((text, is_input)) = selected_text {
            if text.len() >= MIN_TEXT_LENGTH && text.len() <= MAX_TEXT_LENGTH {
                let text_trimmed = text.trim().to_string();

                if !text_trimmed.is_empty() {
                    if preview_is_visible {
                        last_selection = Some(text_trimmed);
                    } else {
                        let text_changed = last_selection.as_ref() != Some(&text_trimmed);

                        if text_changed {
                            info!(
                                "Selection changed: {} chars (was {} chars), is_input={}",
                                text_trimmed.len(),
                                last_selection.as_ref().map(|s| s.len()).unwrap_or(0),
                                is_input
                            );
                            last_selection = Some(text_trimmed.clone());
                            last_is_input = is_input;

                            if !mouseup_pending {
                                // Keyboard or other — start debounce timer
                                debounce_text = Some(text_trimmed.clone());
                                last_change_time = Instant::now();
                            }
                            // If mouseup_pending, skip debounce — will show below
                        } else if is_input != last_is_input {
                            info!(
                                "Mode changed: is_input {} → {} ({} chars)",
                                last_is_input, is_input, text_trimmed.len()
                            );
                            last_is_input = is_input;
                            if !mouseup_pending {
                                debounce_text = Some(text_trimmed.clone());
                                last_change_time = Instant::now();
                            }
                        }

                        // ── Show popup? ─────────────────────────────────
                        let instant = mouseup_pending && last_selection.is_some();
                        let debounced = debounce_text.is_some()
                            && last_change_time.elapsed() >= Duration::from_millis(DEBOUNCE_MS);

                        if instant || debounced {
                            let show_text = last_selection.as_ref().unwrap();
                            if instant {
                                info!("Instant popup via mouseup (skip debounce)");
                            }
                            let mouse_pos = get_cursor_position();
                            let source_hwnd = unsafe { GetForegroundWindow().0 as isize };
                            let (app_name, window_title) = get_window_context(source_hwnd);

                            let input_rect = if let Some(ref uia_engine) = uia {
                                uia_engine
                                    .get_selection_rect()
                                    .map(|r| (r.x, r.y, r.width, r.height))
                            } else {
                                None
                            };

                            let selection_info = SelectionInfo {
                                text: show_text.clone(),
                                mouse_x: mouse_pos.0,
                                mouse_y: mouse_pos.1,
                                source: SelectionSource::UIA,
                                source_hwnd: Some(source_hwnd),
                                input_rect,
                                app_name,
                                window_title,
                                is_input_element: last_is_input,
                            };

                            show_popup(app_handle.clone(), &state, selection_info);
                            selection_source_hwnd = source_hwnd;
                            popup_icon_visible = true;
                            mouse_idle_after_popup = false;
                            mouseup_pending = false;
                            debounce_text = None;
                        }
                    }
                }
            }
        } else {
            // ── No selection — unconditional dismiss ────────────────────
            // Selection gone = Popup gone, regardless of state (SPEC §3.3)
            if last_selection.is_some() {
                info!(
                    "Selection cleared — hiding popup (was {} chars, preview_was={})",
                    last_selection.as_ref().map(|s| s.len()).unwrap_or(0),
                    preview_is_visible
                );
                state.cancel_token.lock().cancel();
                last_selection = None;
                debounce_text = None;
                popup_icon_visible = false;
                selection_source_hwnd = 0;
                mouseup_pending = false;
                overlay::hide_popup(&app_handle);
                *state.current_selection.lock() = None;
                *state.preview_visible.lock() = false;
                state.selection_generation.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }

        std::thread::sleep(Duration::from_millis(poll_interval));
    }
}

/// Get the current cursor position
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
        info.mouse_x, info.mouse_y, preview
    );

    *state.current_selection.lock() = Some(info.clone());

    let icon_position = state.settings.lock().popup_icon_position.clone();
    overlay::show_popup_icon(&app_handle, info.mouse_x, info.mouse_y, info.input_rect, &icon_position);

    if let Err(e) = app_handle.emit("selection-detected", &info) {
        warn!("Failed to emit selection event: {}", e);
    }
}

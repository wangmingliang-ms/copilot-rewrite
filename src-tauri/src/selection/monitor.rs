// Selection monitor - orchestrates UIA polling and clipboard fallback
// Runs in a background thread, emitting selection events to the frontend
//
// Architecture (SPEC §4.2):
//   Mouse events drive popup (mouseup → single UIA check → show popup).
//   Low-frequency keyboard fallback (800ms) catches Ctrl+A, Shift+Arrow.
//   Idle state does NOT call UIA — only checks mouse/keyboard state.
//
// Three-tier polling:
//   idle (200ms)       — no active selection, no mouseup pending → mouse state only
//   active (100ms)     — popup visible or debounce pending → normal UIA polling
//   post-mouseup (20ms)— right after mouseup → fast UIA polling for responsiveness

use crate::{AppState, SelectionInfo, SelectionSource};
use log::{debug, error, info, trace, warn};
use std::sync::atomic::Ordering;
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
    GetCursorPos, GetForegroundWindow, GetWindowTextW,
};

use super::uia::UiaEngine;
use crate::overlay;

// ─── Constants ───────────────────────────────────────────────────────────────

/// Debounce time for keyboard selections (mouse bypasses this via mouseup_time).
/// SPEC 3.1: keyboard fallback with debounce.
const DEBOUNCE_MS: u64 = 300;
/// Minimum text length to trigger popup
const MIN_TEXT_LENGTH: usize = 1;
/// Maximum text length to process
const MAX_TEXT_LENGTH: usize = 5000;
/// How long after mouseup the instant-show window remains valid (ms).
/// UIA typically needs 1-2 poll iterations to detect the new selection.
const MOUSEUP_WINDOW_MS: u64 = 500;
/// Fast poll interval (ms) used right after mouseup for responsive popup appearance.
const FAST_POLL_MS: u64 = 20;
/// Idle poll interval (ms) — no active selection, only checking mouse state.
const IDLE_POLL_MS: u64 = 200;
/// Keyboard fallback UIA poll interval (ms) — SPEC §4.2: 500ms–1s.
const KEYBOARD_POLL_MS: u64 = 800;
/// Maximum time preview_visible can stay true before forced reset (seconds).
const PREVIEW_VISIBLE_TIMEOUT_SECS: u64 = 60;

// ─── DismissReason ───────────────────────────────────────────────────────────

/// Why the popup is being dismissed. Controls which side-effects to apply.
#[derive(Debug, Clone, Copy)]
enum DismissReason {
    /// User clicked somewhere other than the popup icon
    MousedownElsewhere,
    /// The foreground window changed away from the source app
    ForegroundChanged,
    /// UIA returned no selection (selection cleared / lost)
    SelectionCleared,
    /// External dismiss bumped the generation counter
    GenerationChanged,
    /// preview_visible was stuck true for too long
    PreviewVisibleStuck,
}

// ─── MonitorState ────────────────────────────────────────────────────────────

/// All mutable state for the selection monitor loop, collected into a single
/// struct to eliminate scattered locals and make reset operations explicit.
struct MonitorState {
    /// The last confirmed selection text (used for change detection)
    last_selection: Option<String>,
    /// When the selection text last changed (for debounce timing)
    last_change_time: Instant,
    /// Text waiting for debounce to expire (keyboard path only)
    debounce_text: Option<String>,
    /// Whether the last selection was from an input/editable element
    last_is_input: bool,
    /// HWND of the window where the selection originated
    selection_source_hwnd: isize,
    /// Whether the popup icon is currently visible
    popup_icon_visible: bool,
    /// Whether the mouse has gone idle (button released) after popup appeared.
    /// Used to detect "click elsewhere" dismiss.
    mouse_idle_after_popup: bool,
    /// Whether a mouse drag is in progress
    mouse_selecting: bool,
    /// Timestamp of last mouseup event (expires after MOUSEUP_WINDOW_MS)
    mouseup_time: Option<Instant>,
    /// Cursor position captured at mouseup (before mouse may move)
    mouseup_cursor_pos: (i32, i32),
    /// Last seen generation counter (detect external dismiss)
    seen_generation: u64,
    /// Cached foreground window context: (hwnd, app_name, window_title)
    cached_fg_context: Option<(isize, String, String)>,
    /// When preview_visible was first observed true (for stuck detection)
    preview_visible_since: Option<Instant>,
    /// Timer for keyboard-only UIA fallback polling
    keyboard_poll_timer: Instant,
}

impl MonitorState {
    fn new(initial_generation: u64) -> Self {
        Self {
            last_selection: None,
            last_change_time: Instant::now(),
            debounce_text: None,
            last_is_input: true,
            selection_source_hwnd: 0,
            popup_icon_visible: false,
            mouse_idle_after_popup: false,
            mouse_selecting: false,
            mouseup_time: None,
            mouseup_cursor_pos: (0, 0),
            seen_generation: initial_generation,
            cached_fg_context: None,
            preview_visible_since: None,
            keyboard_poll_timer: Instant::now(),
        }
    }

    /// Unified dismiss: resets all selection/popup state and applies
    /// reason-specific side effects (cancel token, preview_visible, generation).
    fn dismiss(
        &mut self,
        reason: DismissReason,
        app_handle: &AppHandle,
        state: &AppState,
        uia: Option<&UiaEngine>,
    ) {
        info!("Dismiss: {:?}", reason);

        // ── Common reset (all reasons) ──
        self.last_selection = None;
        self.debounce_text = None;
        self.popup_icon_visible = false;
        self.selection_source_hwnd = 0;
        self.mouseup_time = None;
        self.mouse_idle_after_popup = false;
        self.mouse_selecting = false;
        self.cached_fg_context = None;
        self.preview_visible_since = None;
        overlay::hide_popup(app_handle);
        *state.current_selection.lock() = None;

        // ── Reason-specific side effects ──
        match reason {
            DismissReason::ForegroundChanged | DismissReason::PreviewVisibleStuck => {
                *state.preview_visible.lock() = false;
                state.cancel_token.lock().cancel();
            }
            DismissReason::SelectionCleared => {
                *state.preview_visible.lock() = false;
                state.cancel_token.lock().cancel();
                state.selection_generation.fetch_add(1, Ordering::Relaxed);
            }
            DismissReason::MousedownElsewhere | DismissReason::GenerationChanged => {
                // MousedownElsewhere: popup is in icon state, no API request to cancel.
                // GenerationChanged: external code already handled cancellation.
            }
        }

        // Clear UIA cache so next check starts fresh
        if let Some(uia_engine) = uia {
            uia_engine.clear_cache();
        }
    }

    /// Compute the appropriate sleep duration based on current state.
    fn sleep_duration(&self, poll_interval: u64) -> u64 {
        if self.mouseup_time.is_some() {
            FAST_POLL_MS // 20ms — post mouseup fast polling
        } else if self.popup_icon_visible || self.debounce_text.is_some() {
            poll_interval // 100ms — active selection state
        } else {
            IDLE_POLL_MS // 200ms — idle, only checking mouse state
        }
    }

    /// Whether UIA should be polled this iteration.
    /// In idle state, UIA is only polled on the keyboard fallback timer.
    fn should_poll_uia(&mut self) -> bool {
        // Always poll UIA when there's active state to track
        if self.popup_icon_visible
            || self.mouseup_time.is_some()
            || self.debounce_text.is_some()
            || self.last_selection.is_some()
        {
            return true;
        }

        // Idle state: only poll on keyboard fallback timer (SPEC §4.2)
        if self.keyboard_poll_timer.elapsed() >= Duration::from_millis(KEYBOARD_POLL_MS) {
            self.keyboard_poll_timer = Instant::now();
            return true;
        }

        false
    }
}

// ─── Health Check ────────────────────────────────────────────────────────────

/// Periodic health check — verifies popup window and WebView2 are alive.
/// Called every 5 minutes. Completely independent of selection detection.
fn run_health_check(app_handle: &AppHandle) {
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
                    info!(
                        "Health check: popup window OK (HWND=0x{:X})",
                        hwnd.0 as isize
                    );
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

// ─── Window Context ──────────────────────────────────────────────────────────

/// Get the process name and window title for a given HWND.
/// Results should be cached per-HWND since they don't change for the same window.
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
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                h,
                Some(&mut pid),
            )
        };
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
    overlay::show_popup_icon(
        &app_handle,
        info.mouse_x,
        info.mouse_y,
        info.input_rect,
        &icon_position,
    );

    if let Err(e) = app_handle.emit("selection-detected", &info) {
        warn!("Failed to emit selection event: {}", e);
    }
}

// ─── Main Engine ─────────────────────────────────────────────────────────────

/// Start the selection monitoring engine.
///
/// Runs on a dedicated OS thread (COM requirement for UIA).
/// The loop structure follows SPEC §4.2 event-driven architecture:
///
/// ```text
/// loop {
///   1. Health check (every 5 min, delegated to function)
///   2. Enabled check
///   3. Generation check → dismiss if changed
///   4. Preview stuck check (60s timeout)
///   5. Mouse state + mousedown dismiss
///   6. Mouseup detection
///   7. Foreground check → dismiss if changed
///   8. Own window → skip
///   9. UIA poll (gated by should_poll_uia — idle skips UIA)
///  10. Process selection / show popup / dismiss
///  11. Sleep (three-tier: idle 200ms / active 100ms / post-mouseup 20ms)
/// }
/// ```
pub fn start_selection_engine(app_handle: AppHandle, state: Arc<AppState>) {
    info!("Selection engine starting...");

    let uia = match UiaEngine::new() {
        Ok(engine) => {
            info!("UIA engine initialized successfully");
            Some(engine)
        }
        Err(e) => {
            error!(
                "Failed to initialize UIA engine: {}. Using clipboard fallback only.",
                e
            );
            None
        }
    };

    let initial_gen = state.selection_generation.load(Ordering::Relaxed);
    let mut ms = MonitorState::new(initial_gen);

    let mut last_health_check = Instant::now();
    let health_check_interval = Duration::from_secs(300);

    // Cache popup and settings window HWNDs (stable for app lifetime)
    let popup_hwnd: isize = app_handle
        .get_webview_window("popup")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);
    let settings_hwnd: isize = app_handle
        .get_webview_window("settings")
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);

    loop {
        // ── 1. Health check (every 5 min) ──
        if last_health_check.elapsed() >= health_check_interval {
            last_health_check = Instant::now();
            run_health_check(&app_handle);
        }

        // ── 2. Monitoring enabled? ──
        if !*state.enabled.lock() {
            ms.last_selection = None;
            ms.debounce_text = None;
            std::thread::sleep(Duration::from_millis(500));
            continue;
        }

        // ── 3. Generation check (external dismiss) ──
        let current_gen = state.selection_generation.load(Ordering::Relaxed);
        if current_gen != ms.seen_generation {
            info!(
                "Generation changed ({} -> {}) -- resetting monitor state",
                ms.seen_generation, current_gen
            );
            ms.seen_generation = current_gen;
            // Only dismiss if we have active state; generation change alone
            // may happen after an external dismiss when there's nothing to reset.
            if ms.last_selection.is_some() || ms.popup_icon_visible {
                ms.dismiss(
                    DismissReason::GenerationChanged,
                    &app_handle,
                    &state,
                    uia.as_ref(),
                );
            } else {
                // Still reset the lightweight fields
                ms.last_selection = None;
                ms.debounce_text = None;
            }
        }

        let mut preview_is_visible = *state.preview_visible.lock();
        let poll_interval = state.settings.lock().poll_interval_ms;

        // ── 4. Preview stuck check ──
        // Track when preview_visible first became true, force-reset after timeout
        if preview_is_visible {
            if ms.preview_visible_since.is_none() {
                ms.preview_visible_since = Some(Instant::now());
            }

            // Check 1: Tauri reports popup hidden but preview_visible is still true
            if let Some(popup) = app_handle.get_webview_window("popup") {
                if let Ok(visible) = popup.is_visible() {
                    if !visible {
                        warn!("preview_visible stuck (popup hidden) -- auto-resetting");
                        *state.preview_visible.lock() = false;
                        preview_is_visible = false;
                        ms.preview_visible_since = None;
                    }
                }
            }

            // Check 2: Timeout — preview_visible stuck true for too long
            if preview_is_visible {
                if let Some(since) = ms.preview_visible_since {
                    if since.elapsed() >= Duration::from_secs(PREVIEW_VISIBLE_TIMEOUT_SECS) {
                        warn!(
                            "preview_visible stuck for {}s -- force dismissing",
                            since.elapsed().as_secs()
                        );
                        ms.dismiss(
                            DismissReason::PreviewVisibleStuck,
                            &app_handle,
                            &state,
                            uia.as_ref(),
                        );
                        std::thread::sleep(Duration::from_millis(poll_interval));
                        continue;
                    }
                }
            }
        } else {
            ms.preview_visible_since = None;
        }

        // ── 5. Mouse state + mousedown dismiss ──
        let lbutton_now = unsafe { GetAsyncKeyState(0x01) } & (0x8000u16 as i16) != 0;

        // Expire stale mouseup_time
        if let Some(t) = ms.mouseup_time {
            if t.elapsed() >= Duration::from_millis(MOUSEUP_WINDOW_MS) {
                debug!(
                    "mouseup_time expired ({}ms elapsed)",
                    t.elapsed().as_millis()
                );
                ms.mouseup_time = None;
            }
        }

        // Mousedown dismiss for icon state:
        // When popup icon is visible (not expanded), detect mousedown elsewhere
        // to dismiss immediately. Applies to BOTH Read Mode and Write Mode:
        // - Read Mode: UIA retains stale selection, mouse is the dismiss signal
        // - Write Mode: UIA TextPattern.GetSelection() takes 1-3s to return
        //   empty after deselection, mousedown provides instant dismiss
        if ms.popup_icon_visible && !preview_is_visible {
            if !ms.mouse_idle_after_popup {
                if !lbutton_now {
                    ms.mouse_idle_after_popup = true;
                    debug!("Mousedown dismiss: mouse idle, watching for next click");
                }
            } else if lbutton_now {
                let click_on_popup = {
                    if let Some(popup) = app_handle.get_webview_window("popup") {
                        if let (Ok(pos), Ok(size)) =
                            (popup.outer_position(), popup.outer_size())
                        {
                            let mut cursor = POINT::default();
                            let _ = unsafe { GetCursorPos(&mut cursor) };
                            cursor.x >= pos.x
                                && cursor.x <= pos.x + size.width as i32
                                && cursor.y >= pos.y
                                && cursor.y <= pos.y + size.height as i32
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if !click_on_popup {
                    info!(
                        "Mousedown elsewhere -- dismissing popup icon (is_input={})",
                        ms.last_is_input
                    );
                    ms.dismiss(
                        DismissReason::MousedownElsewhere,
                        &app_handle,
                        &state,
                        uia.as_ref(),
                    );
                    std::thread::sleep(Duration::from_millis(poll_interval));
                    continue;
                }
            }
        } else {
            ms.mouse_idle_after_popup = false;
        }

        // ── 6. Mouseup detection ──
        // On mouseup, record the timestamp and cursor position. The instant-show
        // window (MOUSEUP_WINDOW_MS) stays valid across multiple loop iterations,
        // bridging the gap where UIA needs 1-2 iterations to detect the selection.
        if !ms.popup_icon_visible && !preview_is_visible {
            if lbutton_now && !ms.mouse_selecting {
                ms.mouse_selecting = true;
            } else if !lbutton_now && ms.mouse_selecting {
                ms.mouse_selecting = false;
                ms.mouseup_time = Some(Instant::now());
                ms.mouseup_cursor_pos = get_cursor_position();
                info!(
                    "Mouseup detected at ({}, {}) -- pending instant show",
                    ms.mouseup_cursor_pos.0, ms.mouseup_cursor_pos.1
                );
            }
        } else if !lbutton_now {
            ms.mouse_selecting = false;
        }

        // ── 7. Foreground window check ──
        let current_fg = unsafe { GetForegroundWindow().0 as isize };

        // Simple HWND comparison — sufficient for detecting window changes.
        // The old PID-based safety net for HWND reuse is an extreme edge case
        // not worth 2× GetWindowThreadProcessId per iteration.
        let fg_changed =
            ms.selection_source_hwnd != 0 && current_fg != ms.selection_source_hwnd;

        let is_own_window = current_fg == popup_hwnd || current_fg == settings_hwnd;

        if (ms.popup_icon_visible || preview_is_visible) && !is_own_window && fg_changed {
            info!(
                "Foreground changed: source=0x{:X} current=0x{:X} -- hiding popup",
                ms.selection_source_hwnd, current_fg
            );
            ms.dismiss(
                DismissReason::ForegroundChanged,
                &app_handle,
                &state,
                uia.as_ref(),
            );
            std::thread::sleep(Duration::from_millis(poll_interval));
            continue;
        }

        // ── 8. Own window foreground → skip UIA ──
        if is_own_window {
            trace!("Own popup is foreground -- preserving current state");
            std::thread::sleep(Duration::from_millis(poll_interval));
            continue;
        }

        // ── 9. UIA poll (gated by should_poll_uia) ──
        let selected_text = if ms.should_poll_uia() {
            if let Some(ref uia_engine) = uia {
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
            }
        } else {
            // Idle: UIA not polled this iteration (no active state).
            // should_poll_uia() returns true when last_selection is Some,
            // so reaching here means no selection → return None.
            None
        };

        // ── 10. Process selection / show popup / dismiss ──
        if let Some((text, is_input)) = selected_text {
            if text.len() >= MIN_TEXT_LENGTH && text.len() <= MAX_TEXT_LENGTH {
                let text_trimmed = text.trim().to_string();

                if !text_trimmed.is_empty() {
                    if preview_is_visible {
                        ms.last_selection = Some(text_trimmed);
                    } else {
                        let text_changed =
                            ms.last_selection.as_ref() != Some(&text_trimmed);
                        let mouseup_active = ms.mouseup_time.is_some();

                        if text_changed {
                            info!(
                                "Selection changed: {} chars (was {} chars), is_input={}, mouseup_active={}",
                                text_trimmed.len(),
                                ms.last_selection.as_ref().map(|s| s.len()).unwrap_or(0),
                                is_input,
                                mouseup_active,
                            );
                            ms.last_selection = Some(text_trimmed.clone());
                            ms.last_is_input = is_input;

                            if !mouseup_active {
                                // Keyboard path -- start debounce timer
                                ms.debounce_text = Some(text_trimmed.clone());
                                ms.last_change_time = Instant::now();
                            }
                            // If mouseup_active, skip debounce -- will show below
                        } else if is_input != ms.last_is_input {
                            info!(
                                "Mode changed: is_input {} -> {} ({} chars)",
                                ms.last_is_input,
                                is_input,
                                text_trimmed.len()
                            );
                            ms.last_is_input = is_input;
                            if !mouseup_active {
                                ms.debounce_text = Some(text_trimmed.clone());
                                ms.last_change_time = Instant::now();
                            }
                        }

                        // -- Show popup? --
                        let instant = mouseup_active && ms.last_selection.is_some();
                        let debounced = ms.debounce_text.is_some()
                            && ms.last_change_time.elapsed()
                                >= Duration::from_millis(DEBOUNCE_MS);

                        if instant || debounced {
                            let show_text = ms.last_selection.as_ref().unwrap();
                            if instant {
                                info!(
                                    "Instant popup via mouseup ({}ms after mouseup)",
                                    ms.mouseup_time.unwrap().elapsed().as_millis()
                                );
                            }
                            // Use mouseup cursor position for instant path (captured
                            // at mouseup time before mouse may have moved), otherwise
                            // use current cursor position for keyboard path.
                            let mouse_pos = if instant {
                                ms.mouseup_cursor_pos
                            } else {
                                get_cursor_position()
                            };
                            let source_hwnd =
                                unsafe { GetForegroundWindow().0 as isize };

                            // Cache foreground context (reuse if HWND unchanged)
                            let (app_name, window_title) = match &ms.cached_fg_context {
                                Some((cached_hwnd, name, title))
                                    if *cached_hwnd == source_hwnd =>
                                {
                                    (name.clone(), title.clone())
                                }
                                _ => {
                                    let ctx = get_window_context(source_hwnd);
                                    ms.cached_fg_context = Some((
                                        source_hwnd,
                                        ctx.0.clone(),
                                        ctx.1.clone(),
                                    ));
                                    ctx
                                }
                            };

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
                                is_input_element: ms.last_is_input,
                            };

                            show_popup(app_handle.clone(), &state, selection_info);
                            ms.selection_source_hwnd = source_hwnd;
                            ms.popup_icon_visible = true;
                            ms.mouse_idle_after_popup = false;
                            ms.mouseup_time = None;
                            ms.debounce_text = None;
                        }
                    }
                }
            }
        } else {
            // -- No selection -- unconditional dismiss --
            // Selection gone = Popup gone, regardless of state (SPEC 3.3)
            if ms.last_selection.is_some() {
                info!(
                    "Selection cleared -- hiding popup (was {} chars, preview_was={})",
                    ms.last_selection.as_ref().map(|s| s.len()).unwrap_or(0),
                    preview_is_visible
                );
                ms.dismiss(
                    DismissReason::SelectionCleared,
                    &app_handle,
                    &state,
                    uia.as_ref(),
                );
            }
        }

        // ── 11. Sleep (three-tier) ──
        std::thread::sleep(Duration::from_millis(ms.sleep_duration(poll_interval)));
    }
}

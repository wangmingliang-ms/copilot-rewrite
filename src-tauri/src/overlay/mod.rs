// Overlay window management module
// Manages a single unified popup window that transitions:
//   icon (48×48) → spinning (48×48) → expanded (auto-sized with content)
// Uses WS_EX_NOACTIVATE in icon/spinning states, removes it when expanded for click events.
//
// POSITION STABILITY: The popup position is set once (in show_popup_icon) and stored.
// All subsequent state transitions (spinning, expanded) reuse the stored position
// to prevent jumping caused by DPI round-trip errors or cursor movement.

use log::{debug, error, info, warn};
use parking_lot::Mutex;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Position};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::MonitorFromPoint;
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, IsWindow, IsWindowVisible, SetWindowLongW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, GWL_STYLE, HWND_TOP, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

/// Icon/spinner size (physical pixels)
const ICON_SIZE: f64 = 48.0;

/// Expanded preview width
const EXPANDED_WIDTH: f64 = 400.0;
const EXPANDED_MIN_HEIGHT: f64 = 120.0;
const EXPANDED_MAX_HEIGHT: f64 = 400.0;
/// Default height for initial streaming expand (before content is measured)
const EXPANDED_STREAMING_HEIGHT: f64 = 250.0;
/// Shadow margin (logical px) — extra space around content for CSS box-shadow
const SHADOW_MARGIN: f64 = 20.0;
/// Button bar height
const BUTTONS_HEIGHT: f64 = 34.0;
/// Text area padding (top 20 + bottom 12 + labels ~16)
const TEXT_PADDING: f64 = 48.0;
/// Approximate line height
const LINE_HEIGHT_PX: f64 = 18.0;
/// Approximate characters per line
const CHARS_PER_LINE: f64 = 50.0;

/// Offset from cursor position
const POPUP_OFFSET_X: f64 = 8.0;
const POPUP_OFFSET_Y: f64 = 16.0;

/// Stored popup position (logical coordinates) and DPI scale — set once, reused across state transitions
static POPUP_POS: Mutex<(f64, f64, f64)> = Mutex::new((0.0, 0.0, 1.0));
/// Stored input element rect (physical pixels) — for expand_popup sizing/positioning
static INPUT_RECT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
/// Stored popup bottom edge (logical Y) — anchored during content resizing
static POPUP_BOTTOM: Mutex<f64> = Mutex::new(0.0);

/// Set up popup window styles — strip frame, apply WS_EX_NOACTIVATE
pub fn setup_popup_window(app_handle: &AppHandle) {
    info!("Setting up popup window...");

    if let Some(window) = app_handle.get_webview_window("popup") {
        match window.hwnd() {
            Ok(hwnd) => {
                unsafe {
                    let hwnd_win = HWND(hwnd.0 as *mut _);

                    // Extended style: no-activate + tool window
                    let ex_style = GetWindowLongW(hwnd_win, GWL_EXSTYLE);
                    let new_ex = ex_style | WS_EX_NOACTIVATE.0 as i32 | WS_EX_TOOLWINDOW.0 as i32;
                    SetWindowLongW(hwnd_win, GWL_EXSTYLE, new_ex);

                    // Strip frame styles to allow 48×48
                    let style = GetWindowLongW(hwnd_win, GWL_STYLE);
                    let strip = WS_THICKFRAME.0
                        | WS_CAPTION.0
                        | WS_SYSMENU.0
                        | WS_MINIMIZEBOX.0
                        | WS_MAXIMIZEBOX.0;
                    let new_style = style & !(strip as i32);
                    SetWindowLongW(hwnd_win, GWL_STYLE, new_style);

                    // Force 48×48
                    let _ = SetWindowPos(
                        hwnd_win,
                        HWND_TOP,
                        0,
                        0,
                        ICON_SIZE as i32,
                        ICON_SIZE as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
                    );
                }
                info!(
                    "Popup window: {}x{} px, WS_EX_NOACTIVATE, no frame",
                    ICON_SIZE, ICON_SIZE
                );
            }
            Err(e) => warn!("Failed to get popup HWND: {}", e),
        }
    }
}

/// Show popup at icon size (48×48) near selected text (or mouse cursor fallback).
/// This is the ONLY place that calculates position — all other states reuse it.
/// input_rect contains the selected text bounding rect (physical pixels) for positioning and sizing.
/// icon_position: "top-center", "top-left", "top-right", "bottom-center", "bottom-left", "bottom-right"
pub fn show_popup_icon(
    app_handle: &AppHandle,
    mouse_x: i32,
    mouse_y: i32,
    input_rect: Option<(i32, i32, i32, i32)>,
    icon_position: &str,
) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let scale = get_scale_at(mouse_x, mouse_y);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);
        let icon_logical = ICON_SIZE / scale;

        // Position icon relative to selection bounding rect, or fallback to mouse
        let (mut x, mut y) = if let Some((sx, sy, sw, sh)) = input_rect {
            let sel_x = sx as f64 / scale;
            let sel_y = sy as f64 / scale;
            let sel_w = sw as f64 / scale;
            let sel_h = sh as f64 / scale;
            let gap = 4.0; // pixels gap between selection and icon

            match icon_position {
                "top-left" => (sel_x, sel_y - icon_logical - gap),
                "top-right" => (sel_x + sel_w - icon_logical, sel_y - icon_logical - gap),
                "bottom-left" => (sel_x, sel_y + sel_h + gap),
                "bottom-right" => (sel_x + sel_w - icon_logical, sel_y + sel_h + gap),
                "bottom-center" => (sel_x + sel_w / 2.0 - icon_logical / 2.0, sel_y + sel_h + gap),
                _ /* top-center */ => (sel_x + sel_w / 2.0 - icon_logical / 2.0, sel_y - icon_logical - gap),
            }
        } else {
            let logical_x = mouse_x as f64 / scale;
            let logical_y = mouse_y as f64 / scale;
            (logical_x + POPUP_OFFSET_X, logical_y + POPUP_OFFSET_Y)
        };

        if x + icon_logical > screen_w {
            x = screen_w - icon_logical - 8.0;
        }
        if y < 0.0 {
            y = 8.0;
        }
        if x < 0.0 {
            x = 8.0;
        }
        if y + icon_logical > screen_h {
            y = screen_h - icon_logical - 8.0;
        }

        // Store position, scale, and input rect for subsequent transitions
        *POPUP_POS.lock() = (x, y, scale);
        *INPUT_RECT.lock() = input_rect;

        // Ensure icon size (+ shadow margin) + WS_EX_NOACTIVATE
        set_noactivate(app_handle, true);
        let sm_physical = SHADOW_MARGIN * scale;
        resize_popup_physical(
            app_handle,
            ICON_SIZE + sm_physical * 2.0,
            ICON_SIZE + sm_physical * 2.0,
        );

        let _ = window.set_position(Position::Logical(LogicalPosition::new(
            x - SHADOW_MARGIN,
            y - SHADOW_MARGIN,
        )));

        // Use Win32 ShowWindow directly for reliability after sleep/resume.
        // Tauri's window.show() can silently fail when WebView2 is in a bad state.
        match window.hwnd() {
            Ok(hwnd) => {
                let hwnd_win = HWND(hwnd.0 as *mut _);
                unsafe {
                    if !IsWindow(hwnd_win).as_bool() {
                        warn!("Popup HWND is no longer valid! Window may need restart.");
                    }
                    // SW_SHOWNOACTIVATE: show without stealing focus
                    let _ = ShowWindow(hwnd_win, SW_SHOWNOACTIVATE);
                    // Force to top of Z-order
                    let _ = SetWindowPos(
                        hwnd_win,
                        HWND_TOP,
                        0, 0, 0, 0,
                        SWP_NOMOVE | SWP_NOSIZE,
                    );
                }
                // Verify it's actually visible
                let visible = unsafe { IsWindowVisible(hwnd_win).as_bool() };
                if !visible {
                    warn!("Popup window is NOT visible after ShowWindow! Trying Tauri fallback...");
                    let _ = window.show();
                }
            }
            Err(e) => {
                warn!("Failed to get popup HWND for ShowWindow: {}. Using Tauri fallback.", e);
                let _ = window.show();
            }
        }

        info!("Popup icon shown at ({:.0}, {:.0})", x, y);
    }
}

/// Expand popup to show result text — removes WS_EX_NOACTIVATE for interactivity.
/// Uses input element rect for width and vertical positioning when available.
pub fn expand_popup(app_handle: &AppHandle, text: &str) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        // Extract translated text from JSON for height estimation
        let est_text = extract_translated(text).unwrap_or(text);
        let height = estimate_height(est_text);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);

        let (x, y, w_logical) = {
            let stored_input = *INPUT_RECT.lock();

            // Determine width: use selection width if available (clamped), otherwise default
            let w = if let Some((_rx, ry, rw, _rh)) = stored_input {
                let scale = get_scale_at(0, ry);
                let elem_w = rw as f64 / scale;
                elem_w.max(EXPANDED_WIDTH).min(screen_w - 16.0)
            } else {
                EXPANDED_WIDTH
            };

            // Position: prefer placing relative to selection rect, fallback to stored popup pos
            if let Some((sx, sy, _sw, sh)) = stored_input {
                let scale = get_scale_at(sx, sy);
                let sel_x = sx as f64 / scale;
                let sel_y = sy as f64 / scale;
                let sel_h = sh as f64 / scale;

                // Try above selection first (12px gap)
                let mut py = sel_y - height - 12.0;
                let mut px = sel_x;

                if py < 0.0 {
                    // Not enough room above — place below selection
                    py = sel_y + sel_h + 12.0;
                }
                // If below also overflows, clamp to screen bottom
                if py + height > screen_h {
                    py = screen_h - height - 8.0;
                }
                if px + w > screen_w {
                    px = screen_w - w - 8.0;
                }
                if px < 0.0 {
                    px = 8.0;
                }
                if py < 0.0 {
                    py = 8.0;
                }

                (px, py, w)
            } else {
                // No selection rect — use stored popup position
                let (stored_x, stored_y, _) = *POPUP_POS.lock();
                let mut x = stored_x;
                let mut y = stored_y;
                if x + w > screen_w {
                    x = screen_w - w - 8.0;
                }
                if y + height > screen_h {
                    y = screen_h - height - 8.0;
                }
                if x < 0.0 {
                    x = 8.0;
                }
                if y < 0.0 {
                    y = 8.0;
                }
                (x, y, w)
            }
        };

        // Remove WS_EX_NOACTIVATE so buttons are clickable
        set_noactivate(app_handle, false);

        // Add shadow margin: window is larger than content, positioned offset by margin
        let win_w = w_logical + SHADOW_MARGIN * 2.0;
        let win_h = height + SHADOW_MARGIN * 2.0;
        let win_x = x - SHADOW_MARGIN;
        let win_y = y - SHADOW_MARGIN;

        // Store the bottom edge position (content bottom Y) for anchor-bottom resizing
        let content_bottom = y + height;
        *POPUP_BOTTOM.lock() = content_bottom;

        let _ = window.set_size(LogicalSize::new(win_w, win_h));
        let _ = window.set_position(Position::Logical(LogicalPosition::new(win_x, win_y)));

        info!(
            "Popup expanded to {:.0}x{:.0} (content {:.0}x{:.0}) at ({:.0}, {:.0}), bottom={:.0}",
            win_w, win_h, w_logical, height, win_x, win_y, content_bottom
        );
    }
}

/// Expand popup for streaming — uses a fixed default height instead of estimating from text.
/// The frontend resize effect will adjust the height as content grows.
pub fn expand_popup_streaming(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let height = EXPANDED_STREAMING_HEIGHT;
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);

        let (x, y, w_logical) = {
            let stored_input = *INPUT_RECT.lock();

            let w = if let Some((_rx, ry, rw, _rh)) = stored_input {
                let scale = get_scale_at(0, ry);
                let elem_w = rw as f64 / scale;
                elem_w.max(EXPANDED_WIDTH).min(screen_w - 16.0)
            } else {
                EXPANDED_WIDTH
            };

            if let Some((sx, sy, _sw, sh)) = stored_input {
                let scale = get_scale_at(sx, sy);
                let sel_x = sx as f64 / scale;
                let sel_y = sy as f64 / scale;
                let sel_h = sh as f64 / scale;

                let mut py = sel_y - height - 12.0;
                let mut px = sel_x;

                if py < 0.0 {
                    py = sel_y + sel_h + 12.0;
                }
                if py + height > screen_h {
                    py = screen_h - height - 8.0;
                }
                if px + w > screen_w {
                    px = screen_w - w - 8.0;
                }
                if px < 0.0 { px = 8.0; }
                if py < 0.0 { py = 8.0; }

                (px, py, w)
            } else {
                let (stored_x, stored_y, _) = *POPUP_POS.lock();
                let mut x = stored_x;
                let mut y = stored_y;
                if x + w > screen_w { x = screen_w - w - 8.0; }
                if y + height > screen_h { y = screen_h - height - 8.0; }
                if x < 0.0 { x = 8.0; }
                if y < 0.0 { y = 8.0; }
                (x, y, w)
            }
        };

        set_noactivate(app_handle, false);

        let win_w = w_logical + SHADOW_MARGIN * 2.0;
        let win_h = height + SHADOW_MARGIN * 2.0;
        let win_x = x - SHADOW_MARGIN;
        let win_y = y - SHADOW_MARGIN;

        let content_bottom = y + height;
        *POPUP_BOTTOM.lock() = content_bottom;

        let _ = window.set_size(LogicalSize::new(win_w, win_h));
        let _ = window.set_position(Position::Logical(LogicalPosition::new(win_x, win_y)));

        info!(
            "Popup streaming expand to {:.0}x{:.0} (content {:.0}x{:.0}) at ({:.0}, {:.0}), bottom={:.0}",
            win_w, win_h, w_logical, height, win_x, win_y, content_bottom
        );
    }
}

/// Shrink popup back to icon size and re-apply WS_EX_NOACTIVATE
pub fn shrink_popup(app_handle: &AppHandle) {
    set_noactivate(app_handle, true);
    // Use the stored scale factor from when the popup was originally positioned
    let scale = POPUP_POS.lock().2;
    let sm_physical = SHADOW_MARGIN * scale;
    resize_popup_physical(
        app_handle,
        ICON_SIZE + sm_physical * 2.0,
        ICON_SIZE + sm_physical * 2.0,
    );
}

/// Hide the popup window
pub fn hide_popup(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let _ = window.hide();
        // Belt-and-suspenders: also use Win32 to ensure window is truly hidden.
        // Tauri's window.hide() can silently fail in some edge cases.
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let hwnd_win = HWND(hwnd.0 as *mut _);
                let _ = ShowWindow(hwnd_win, SW_HIDE);
                if IsWindowVisible(hwnd_win).as_bool() {
                    error!("Popup STILL visible after hide! Forcing SW_HIDE again.");
                    let _ = ShowWindow(hwnd_win, SW_HIDE);
                }
            }
        }
    }
    info!("Popup hidden");
    // Note: icon size + WS_EX_NOACTIVATE are set in show_popup_icon() before the
    // next show, so we don't need to reset them here. This avoids a redundant
    // DPI calculation and SetWindowPos call on a hidden window.
}

/// Toggle WS_EX_NOACTIVATE on the popup window
fn set_noactivate(app_handle: &AppHandle, enable: bool) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let hwnd_win = HWND(hwnd.0 as *mut _);
                let ex_style = GetWindowLongW(hwnd_win, GWL_EXSTYLE);
                let new_style = if enable {
                    ex_style | WS_EX_NOACTIVATE.0 as i32
                } else {
                    ex_style & !(WS_EX_NOACTIVATE.0 as i32)
                };
                SetWindowLongW(hwnd_win, GWL_EXSTYLE, new_style);
                // Apply change without moving or resizing
                let _ = SetWindowPos(
                    hwnd_win,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
            debug!("Popup WS_EX_NOACTIVATE = {}", enable);
        }
    }
}

/// Resize popup to exact physical pixels via SetWindowPos
fn resize_popup_physical(app_handle: &AppHandle, w: f64, h: f64) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                let hwnd_win = HWND(hwnd.0 as *mut _);
                let _ = SetWindowPos(
                    hwnd_win,
                    HWND_TOP,
                    0,
                    0,
                    w as i32,
                    h as i32,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }
    }
}

/// Estimate expanded height based on text content
fn estimate_height(text: &str) -> f64 {
    if text.is_empty() {
        return EXPANDED_MIN_HEIGHT;
    }
    let newline_count = text.chars().filter(|c| *c == '\n').count() as f64;
    let char_count = text.chars().count() as f64;
    let wrapped_lines = (char_count / CHARS_PER_LINE).ceil();
    let total_lines = wrapped_lines.max(newline_count + 1.0);
    let text_height = total_lines * LINE_HEIGHT_PX;
    let height = text_height + TEXT_PADDING + BUTTONS_HEIGHT;
    height.clamp(EXPANDED_MIN_HEIGHT, EXPANDED_MAX_HEIGHT)
}

/// Extract "translated" field from JSON response for height estimation
fn extract_translated(text: &str) -> Option<&str> {
    // Quick JSON parse: find "translated": "..." value
    let marker = "\"translated\"";
    let idx = text.find(marker)?;
    let rest = &text[idx + marker.len()..];
    // Skip whitespace and colon
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('"')?;
    // Find the end of the string value (handle escaped quotes)
    let mut end = 0;
    let mut escaped = false;
    for (i, c) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if c == '"' {
            end = i;
            break;
        }
    }
    if end > 0 {
        Some(&rest[..end])
    } else {
        None
    }
}

/// Resize expanded popup to fit actual rendered content height (called from frontend)
/// Anchors the bottom edge — grows/shrinks upward
pub fn resize_popup_to_content(app_handle: &AppHandle, content_height: f64) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        // Add 20px buffer to avoid sub-pixel scrollbar
        let height = (content_height + 20.0).clamp(EXPANDED_MIN_HEIGHT, EXPANDED_MAX_HEIGHT);
        let stored_input = *INPUT_RECT.lock();
        let bottom = *POPUP_BOTTOM.lock();

        // Determine width: use element width if available (clamped), otherwise default
        let w_logical = if let Some((_rx, ry, rw, _)) = stored_input {
            let scale = get_scale_at(0, ry);
            let elem_w = rw as f64 / scale;
            elem_w.max(EXPANDED_WIDTH).min(1920.0)
        } else {
            EXPANDED_WIDTH
        };

        // Anchor bottom edge: content top = bottom - height
        let mut content_y = bottom - height;
        if content_y < 0.0 {
            content_y = 0.0;
        }

        // Get current X position (keep it stable)
        let content_x = if let Ok(pos) = window.outer_position() {
            let scale = window.scale_factor().unwrap_or(1.0);
            pos.x as f64 / scale + SHADOW_MARGIN
        } else {
            0.0
        };

        let win_w = w_logical + SHADOW_MARGIN * 2.0;
        let win_h = height + SHADOW_MARGIN * 2.0;
        let win_x = content_x - SHADOW_MARGIN;
        let win_y = content_y - SHADOW_MARGIN;

        let _ = window.set_size(LogicalSize::new(win_w, win_h));
        let _ = window.set_position(Position::Logical(LogicalPosition::new(win_x, win_y)));

        debug!("Popup resized to content: {:.0}x{:.0} (content height {:.0}), bottom anchored at {:.0}", win_w, win_h, height, bottom);
    }
}

/// Get scale factor at given physical coordinates
fn get_scale_at(x: i32, y: i32) -> f64 {
    unsafe {
        let point = POINT { x, y };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        dpi_x as f64 / 96.0
    }
}

fn get_primary_screen_info(app_handle: &AppHandle) -> (f64, f64, f64) {
    if let Some(monitor) = app_handle.primary_monitor().ok().flatten() {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        (size.width as f64 / scale, size.height as f64 / scale, scale)
    } else {
        (1920.0, 1080.0, 1.0)
    }
}

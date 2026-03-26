// Overlay window management module
// Manages a single unified popup window that transitions:
//   icon (48×48) → spinning (48×48) → expanded (auto-sized with content)
// Uses WS_EX_NOACTIVATE in icon/spinning states, removes it when expanded for click events.
//
// POSITION STABILITY: The popup position is set once (in show_popup_icon) and stored.
// All subsequent state transitions (spinning, expanded) reuse the stored position
// to prevent jumping caused by DPI round-trip errors or cursor movement.

use log::{debug, info, warn};
use parking_lot::Mutex;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Position};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::MonitorFromPoint;
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, SetWindowLongW, SetWindowPos,
    GWL_EXSTYLE, GWL_STYLE,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_THICKFRAME, WS_CAPTION, WS_SYSMENU, WS_MINIMIZEBOX, WS_MAXIMIZEBOX,
    SWP_NOMOVE, SWP_NOZORDER, SWP_FRAMECHANGED, SWP_NOSIZE, HWND_TOP,
};

/// Icon/spinner size (physical pixels)
const ICON_SIZE: f64 = 48.0;

/// Expanded preview width
const EXPANDED_WIDTH: f64 = 400.0;
const EXPANDED_MIN_HEIGHT: f64 = 80.0;
const EXPANDED_MAX_HEIGHT: f64 = 400.0;
/// Button bar height
const BUTTONS_HEIGHT: f64 = 40.0;
/// Text area padding
const TEXT_PADDING: f64 = 28.0;
/// Approximate line height
const LINE_HEIGHT_PX: f64 = 22.0;
/// Approximate characters per line
const CHARS_PER_LINE: f64 = 50.0;

/// Offset from cursor position
const POPUP_OFFSET_X: f64 = 8.0;
const POPUP_OFFSET_Y: f64 = 16.0;

/// Stored popup position (logical coordinates) — set once, reused across state transitions
static POPUP_POS: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));
/// Stored input element rect (physical pixels) — for expand_popup sizing/positioning
static INPUT_RECT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);

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
                    let new_ex = ex_style
                        | WS_EX_NOACTIVATE.0 as i32
                        | WS_EX_TOOLWINDOW.0 as i32;
                    SetWindowLongW(hwnd_win, GWL_EXSTYLE, new_ex);

                    // Strip frame styles to allow 48×48
                    let style = GetWindowLongW(hwnd_win, GWL_STYLE);
                    let strip = WS_THICKFRAME.0 | WS_CAPTION.0 | WS_SYSMENU.0
                        | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
                    let new_style = style & !(strip as i32);
                    SetWindowLongW(hwnd_win, GWL_STYLE, new_style);

                    // Force 48×48
                    let _ = SetWindowPos(
                        hwnd_win, HWND_TOP,
                        0, 0,
                        ICON_SIZE as i32, ICON_SIZE as i32,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_FRAMECHANGED,
                    );
                }
                info!("Popup window: {}x{} px, WS_EX_NOACTIVATE, no frame", ICON_SIZE, ICON_SIZE);
            }
            Err(e) => warn!("Failed to get popup HWND: {}", e),
        }
    }
}

/// Show popup at icon size (48×48) near cursor.
/// This is the ONLY place that calculates position — all other states reuse it.
pub fn show_popup_icon(app_handle: &AppHandle, mouse_x: i32, mouse_y: i32, input_rect: Option<(i32, i32, i32, i32)>) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let scale = get_scale_at(mouse_x, mouse_y);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);
        let icon_logical = ICON_SIZE / scale;

        // If we have the input element rect, position icon at top-right of input
        // Otherwise fall back to mouse position
        let (mut x, mut y) = if let Some((rx, ry, rw, _rh)) = input_rect {
            let lx = (rx + rw) as f64 / scale - icon_logical - 4.0;
            let ly = ry as f64 / scale - icon_logical - 4.0;
            (lx, ly)
        } else {
            let logical_x = mouse_x as f64 / scale;
            let logical_y = mouse_y as f64 / scale;
            (logical_x + POPUP_OFFSET_X, logical_y + POPUP_OFFSET_Y)
        };

        if x + icon_logical > screen_w { x = screen_w - icon_logical - 8.0; }
        if y < 0.0 { y = 8.0; }
        if x < 0.0 { x = 8.0; }
        if y + icon_logical > screen_h { y = screen_h - icon_logical - 8.0; }

        // Store position and input rect for subsequent transitions
        *POPUP_POS.lock() = (x, y);
        *INPUT_RECT.lock() = input_rect;

        // Ensure icon size + WS_EX_NOACTIVATE
        set_noactivate(app_handle, true);
        resize_popup_physical(app_handle, ICON_SIZE, ICON_SIZE);

        let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));
        let _ = window.show();

        debug!("Popup icon shown at ({:.0}, {:.0})", x, y);
    }
}

/// Expand popup to show result text — removes WS_EX_NOACTIVATE for interactivity.
/// Uses input element rect for width and vertical positioning when available.
pub fn expand_popup(app_handle: &AppHandle, text: &str) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let height = estimate_height(text);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);
        let stored_input = *INPUT_RECT.lock();

        let (x, y, w_logical) = if let Some((rx, ry, rw, rh)) = stored_input {
            // We have the input element rect — align popup to input width
            let scale = get_scale_at(rx, ry);
            let input_x = rx as f64 / scale;
            let input_y = ry as f64 / scale;
            let input_w = (rw as f64 / scale).clamp(200.0, 800.0);
            let input_h = rh as f64 / scale;

            // Try above the input first
            let mut py = input_y - height - 4.0;
            if py < 0.0 {
                // Not enough room above — go below the input
                py = input_y + input_h + 4.0;
            }
            // If still overflows bottom, clamp
            if py + height > screen_h {
                py = screen_h - height - 8.0;
            }

            let mut px = input_x;
            if px + input_w > screen_w { px = screen_w - input_w - 8.0; }
            if px < 0.0 { px = 8.0; }

            (px, py, input_w)
        } else {
            // No input rect — use stored popup position and default width
            let (stored_x, stored_y) = *POPUP_POS.lock();
            let mut x = stored_x;
            let mut y = stored_y;
            if x + EXPANDED_WIDTH > screen_w { x = screen_w - EXPANDED_WIDTH - 8.0; }
            if y + height > screen_h { y = screen_h - height - 8.0; }
            if x < 0.0 { x = 8.0; }
            if y < 0.0 { y = 8.0; }
            (x, y, EXPANDED_WIDTH)
        };

        // Remove WS_EX_NOACTIVATE so buttons are clickable
        set_noactivate(app_handle, false);

        // Resize and reposition
        let _ = window.set_size(LogicalSize::new(w_logical, height));
        let _ = window.set_position(Position::Logical(LogicalPosition::new(x, y)));

        debug!("Popup expanded to {:.0}x{:.0} at ({:.0}, {:.0})", w_logical, height, x, y);
    }
}

/// Shrink popup back to icon size and re-apply WS_EX_NOACTIVATE
pub fn shrink_popup(app_handle: &AppHandle) {
    set_noactivate(app_handle, true);
    resize_popup_physical(app_handle, ICON_SIZE, ICON_SIZE);
}

/// Hide the popup window
pub fn hide_popup(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("popup") {
        let _ = window.hide();
    }
    // Reset to icon size + noactivate for next show
    shrink_popup(app_handle);
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
                    hwnd_win, HWND_TOP,
                    0, 0, 0, 0,
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
                    hwnd_win, HWND_TOP,
                    0, 0,
                    w as i32, h as i32,
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

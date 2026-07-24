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
use tauri::{AppHandle, LogicalPosition, Manager, Position};
use tauri::WebviewWindow;
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITOR_DEFAULTTONEAREST, MONITORINFO};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, IsWindow, IsWindowVisible, SetWindowLongW, SetWindowPos, ShowWindow,
    GWL_EXSTYLE, GWL_STYLE, HWND_TOP, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_EX_NOACTIVATE,
    WS_EX_TOOLWINDOW, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
    GetCursorPos, WindowFromPoint, GetAncestor, GA_ROOT,
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, AllowSetForegroundWindow,
    SetForegroundWindow, ASFW_ANY,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEINPUT,
    MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_VIRTUALDESK, MOUSE_EVENT_FLAGS,
};

/// Icon/spinner size (physical pixels)
const ICON_SIZE: f64 = 48.0;

/// Default expanded content size (logical px) — used when no remembered size exists
const DEFAULT_EXPANDED_W: f64 = 400.0;
const DEFAULT_EXPANDED_H: f64 = 300.0;
/// Minimum expanded content size (logical px) — resize cannot go below this
const MIN_EXPANDED_W: f64 = 300.0;
const MIN_EXPANDED_H: f64 = 200.0;
/// Shadow margin (logical px) — extra space around content for CSS box-shadow
const SHADOW_MARGIN: f64 = 20.0;

/// Offset from cursor position
const POPUP_OFFSET_X: f64 = 8.0;
const POPUP_OFFSET_Y: f64 = 16.0;

/// Stored popup position (logical coordinates) and DPI scale — set once, reused across state transitions
static POPUP_POS: Mutex<(f64, f64, f64)> = Mutex::new((0.0, 0.0, 1.0));
/// Stored input element rect (physical pixels) — for expand_popup positioning
static INPUT_RECT: Mutex<Option<(i32, i32, i32, i32)>> = Mutex::new(None);
/// Remembered expanded content size (logical px) — persisted to settings, reused across popups
static EXPANDED_SIZE: Mutex<(f64, f64)> = Mutex::new((DEFAULT_EXPANDED_W, DEFAULT_EXPANDED_H));
/// Cached popup WebviewWindow — set once in setup_popup_window, avoids HashMap lookup per call
static POPUP_WINDOW: Mutex<Option<WebviewWindow>> = Mutex::new(None);

/// Set the remembered expanded content size (logical px). Called at startup from settings.
pub fn set_expanded_size(w: f64, h: f64) {
    let w = w.max(MIN_EXPANDED_W);
    let h = h.max(MIN_EXPANDED_H);
    *EXPANDED_SIZE.lock() = (w, h);
}

/// Get the cached popup window (set during setup_popup_window).
/// Falls back to app_handle lookup if cache is empty (shouldn't happen after setup).
fn get_popup(app_handle: &AppHandle) -> Option<WebviewWindow> {
    let cached = POPUP_WINDOW.lock();
    if cached.is_some() {
        return cached.clone();
    }
    drop(cached);
    app_handle.get_webview_window("popup")
}

/// Set up popup window styles — strip frame, apply WS_EX_NOACTIVATE.
/// Also caches the WebviewWindow for reuse by all overlay functions.
pub fn setup_popup_window(app_handle: &AppHandle) {
    info!("Setting up popup window...");

    if let Some(window) = app_handle.get_webview_window("popup") {
        // Cache for reuse — avoids HashMap lookup on every overlay call
        *POPUP_WINDOW.lock() = Some(window.clone());

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
    if let Some(window) = get_popup(app_handle) {
        let scale = get_scale_at(mouse_x, mouse_y);
        let (mon_x, mon_y, mon_w, mon_h, _) = get_monitor_info_at(mouse_x, mouse_y);
        let icon_logical = ICON_SIZE / scale;

        // Position icon relative to selection bounding rect, or fallback to mouse
        let (mut x, mut y) = if let Some((sx, sy, sw, sh)) = input_rect {
            let sel_x = sx as f64 / scale;
            let sel_y = sy as f64 / scale;
            let sel_w = sw as f64 / scale;
            let sel_h = sh as f64 / scale;
            let gap = 16.0; // pixels gap between selection and icon (accounts for box-shadow spread)

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

        if x + icon_logical > mon_x + mon_w {
            x = mon_x + mon_w - icon_logical - 8.0;
        }
        if y < mon_y {
            y = mon_y + 8.0;
        }
        if x < mon_x {
            x = mon_x + 8.0;
        }
        if y + icon_logical > mon_y + mon_h {
            y = mon_y + mon_h - icon_logical - 8.0;
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

/// Compute expanded popup position given target width and height.
/// Returns (x, y) in logical coordinates, clamped to screen bounds.
/// Uses input element rect for positioning when available, otherwise stored popup position.
fn compute_expanded_position(_app_handle: &AppHandle, w: f64, height: f64) -> (f64, f64) {
    let stored_input = *INPUT_RECT.lock();

    // Determine which monitor the popup should appear on
    let (mon_x, mon_y, mon_w, mon_h, _mon_scale) = if let Some((sx, sy, _, _)) = stored_input {
        get_monitor_info_at(sx, sy)
    } else {
        let (px, py, s) = *POPUP_POS.lock();
        get_monitor_info_at((px * s) as i32, (py * s) as i32)
    };
    let screen_right = mon_x + mon_w;
    let screen_bottom = mon_y + mon_h;

    // Position relative to selection rect, or fallback to stored popup pos
    if let Some((sx, sy, _sw, sh)) = stored_input {
        let scale = get_scale_at(sx, sy);
        let sel_x = sx as f64 / scale;
        let sel_y = sy as f64 / scale;
        let sel_h = sh as f64 / scale;

        // Try above selection first (12px gap)
        let mut py = sel_y - height - 12.0;
        let mut px = sel_x;

        if py < mon_y {
            // Not enough room above — place below selection
            py = sel_y + sel_h + 12.0;
        }
        if py + height > screen_bottom {
            py = screen_bottom - height - 8.0;
        }
        if px + w > screen_right {
            px = screen_right - w - 8.0;
        }
        if px < mon_x { px = mon_x + 8.0; }
        if py < mon_y { py = mon_y + 8.0; }

        (px, py)
    } else {
        let (stored_x, stored_y, _) = *POPUP_POS.lock();
        let mut x = stored_x;
        let mut y = stored_y;
        if x + w > screen_right { x = screen_right - w - 8.0; }
        if y + height > screen_bottom { y = screen_bottom - height - 8.0; }
        if x < mon_x { x = mon_x + 8.0; }
        if y < mon_y { y = mon_y + 8.0; }
        (x, y)
    }
}

/// Apply expanded size and position to the popup window.
/// Removes WS_EX_NOACTIVATE, sets size/position atomically.
fn apply_expanded_layout(app_handle: &AppHandle, x: f64, y: f64, w_logical: f64, height: f64, label: &str) {
    if let Some(window) = get_popup(app_handle) {
        // Remove WS_EX_NOACTIVATE so buttons are clickable
        set_noactivate(app_handle, false);

        // Add shadow margin: window is larger than content, positioned offset by margin
        let win_w = w_logical + SHADOW_MARGIN * 2.0;
        let win_h = height + SHADOW_MARGIN * 2.0;
        let win_x = x - SHADOW_MARGIN;
        let win_y = y - SHADOW_MARGIN;

        // Use a single SetWindowPos call to set position + size atomically,
        // avoiding the visible intermediate state between separate set_size/set_position.
        if let Ok(hwnd) = window.hwnd() {
            let scale = window.scale_factor().unwrap_or(1.0);
            unsafe {
                let _ = SetWindowPos(
                    HWND(hwnd.0 as *mut _),
                    HWND_TOP,
                    (win_x * scale) as i32,
                    (win_y * scale) as i32,
                    (win_w * scale) as i32,
                    (win_h * scale) as i32,
                    SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }

        info!(
            "Popup {} to {:.0}x{:.0} (content {:.0}x{:.0}) at ({:.0}, {:.0})",
            label, win_w, win_h, w_logical, height, win_x, win_y
        );
    }
}

/// Expand popup to show result — uses remembered content size.
pub fn expand_popup(app_handle: &AppHandle, _text: &str) {
    let (w, h) = *EXPANDED_SIZE.lock();
    let (x, y) = compute_expanded_position(app_handle, w, h);
    apply_expanded_layout(app_handle, x, y, w, h, "expanded");
}

/// Expand popup for streaming — uses remembered content size (same as expand_popup).
pub fn expand_popup_streaming(app_handle: &AppHandle) {
    let (w, h) = *EXPANDED_SIZE.lock();
    let (x, y) = compute_expanded_position(app_handle, w, h);
    apply_expanded_layout(app_handle, x, y, w, h, "streaming expand");
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
    if let Some(window) = get_popup(app_handle) {
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

/// Handle a right-click on the popup window:
///   1. Capture current cursor position
///   2. Hide the popup synchronously (Win32 ShowWindow SW_HIDE for immediacy)
///   3. Find the window under the cursor (excluding our popup) and bring it forward
///   4. SendInput a synthetic right-button down+up at the same position
///   5. Set a brief suppression flag so the imminent mouseup won't re-trigger
///      the popup
pub fn forward_right_click_to_source(app_handle: &AppHandle) {
    // Step 1: cursor position
    let mut point = POINT { x: 0, y: 0 };
    unsafe {
        if GetCursorPos(&mut point).is_err() {
            warn!("forward_right_click: GetCursorPos failed");
            return;
        }
    }
    info!("forward_right_click: cursor at ({}, {})", point.x, point.y);

    // Step 2: hide popup IMMEDIATELY (Win32 path; Tauri's hide is async-ish)
    if let Some(window) = get_popup(app_handle) {
        if let Ok(hwnd) = window.hwnd() {
            let hwnd_win = HWND(hwnd.0 as *mut _);
            unsafe { let _ = ShowWindow(hwnd_win, SW_HIDE); }
        }
        let _ = window.hide();
    }

    // Step 3: WindowFromPoint then walk up to the top-level owner; skip our own popup HWND
    let popup_hwnd: isize = get_popup(app_handle)
        .and_then(|w| w.hwnd().ok())
        .map(|h| h.0 as isize)
        .unwrap_or(0);

    let target = unsafe {
        let mut hwnd = WindowFromPoint(point);
        // Walk up to top-level (parent==null)
        loop {
            let parent = GetAncestor(hwnd, GA_ROOT);
            if parent.0.is_null() || parent == hwnd { break; }
            hwnd = parent;
        }
        hwnd
    };

    if target.0.is_null() || target.0 as isize == popup_hwnd {
        warn!("forward_right_click: no valid target window under cursor");
        // Still notify the suppression flag so monitor doesn't flap.
        crate::selection::monitor::suppress_popup_briefly();
        return;
    }
    info!("forward_right_click: target HWND={:?}", target.0);

    // Step 4: bring target to foreground, then SendInput right-click
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
        let _ = SetForegroundWindow(target);
    }

    // Step 4: SendInput RIGHTDOWN + RIGHTUP at current absolute coords.
    // We use MOUSEEVENTF_ABSOLUTE with normalized screen coordinates (0..65535).
    let (sx, sy) = unsafe {
        let cx = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let cy = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        let ox = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let oy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let nx = ((point.x - ox) as f64 / cx as f64 * 65535.0).round() as i32;
        let ny = ((point.y - oy) as f64 / cy as f64 * 65535.0).round() as i32;
        (nx, ny)
    };

    let inputs = [
        make_mouse_input(sx, sy, MOUSEEVENTF_RIGHTDOWN | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK),
        make_mouse_input(sx, sy, MOUSEEVENTF_RIGHTUP   | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK),
    ];
    let size = std::mem::size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(&inputs, size) };
    if sent != 2 {
        let err = std::io::Error::last_os_error();
        warn!("forward_right_click: SendInput returned {} (expected 2), err={:?}", sent, err);
    }

    // Step 5: suppression flag — don't re-show popup on the upcoming mouseup
    crate::selection::monitor::suppress_popup_briefly();
}

fn make_mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Toggle WS_EX_NOACTIVATE on the popup window
fn set_noactivate(app_handle: &AppHandle, enable: bool) {
    if let Some(window) = get_popup(app_handle) {
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
    if let Some(window) = get_popup(app_handle) {
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

/// Get the current popup CONTENT rect in logical coordinates (x, y, w, h).
/// Content rect excludes the SHADOW_MARGIN border. Returned to the frontend as
/// the baseline for a drag-resize gesture.
pub fn get_popup_rect(app_handle: &AppHandle) -> Option<(f64, f64, f64, f64)> {
    let window = get_popup(app_handle)?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let pos = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    let win_x = pos.x as f64 / scale;
    let win_y = pos.y as f64 / scale;
    let win_w = size.width as f64 / scale;
    let win_h = size.height as f64 / scale;
    // Convert window rect back to content rect (strip shadow margin)
    Some((
        win_x + SHADOW_MARGIN,
        win_y + SHADOW_MARGIN,
        win_w - SHADOW_MARGIN * 2.0,
        win_h - SHADOW_MARGIN * 2.0,
    ))
}

/// Apply a drag-resize: set the popup CONTENT rect (logical x, y, w, h).
/// Clamps width/height to minimums and the whole rect into the monitor work area.
/// Stores the resulting size in EXPANDED_SIZE and returns the clamped content rect.
pub fn set_popup_rect(app_handle: &AppHandle, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
    // Clamp size to minimums first
    let mut w = w.max(MIN_EXPANDED_W);
    let mut h = h.max(MIN_EXPANDED_H);
    let mut x = x;
    let mut y = y;

    // Determine monitor work area at the target position.
    // Use the popup's current scale to convert logical → physical for the monitor query.
    let win_scale = get_popup(app_handle)
        .and_then(|w| w.scale_factor().ok())
        .unwrap_or(1.0);
    let (mon_x, mon_y, mon_w, mon_h, _) = get_monitor_info_at((x * win_scale) as i32, (y * win_scale) as i32);
    let screen_right = mon_x + mon_w;
    let screen_bottom = mon_y + mon_h;

    // Clamp size so the rect fits within the work area from its current top-left
    if w > mon_w { w = mon_w; }
    if h > mon_h { h = mon_h; }
    // Shift left/up if the rect overflows the right/bottom edges
    if x + w > screen_right { x = screen_right - w; }
    if y + h > screen_bottom { y = screen_bottom - h; }
    // Keep top-left inside the work area
    if x < mon_x { x = mon_x; }
    if y < mon_y { y = mon_y; }

    *EXPANDED_SIZE.lock() = (w, h);

    if let Some(window) = get_popup(app_handle) {
        if let Ok(hwnd) = window.hwnd() {
            let s = window.scale_factor().unwrap_or(1.0);
            let win_x = x - SHADOW_MARGIN;
            let win_y = y - SHADOW_MARGIN;
            let win_w = w + SHADOW_MARGIN * 2.0;
            let win_h = h + SHADOW_MARGIN * 2.0;
            unsafe {
                let _ = SetWindowPos(
                    HWND(hwnd.0 as *mut _),
                    HWND_TOP,
                    (win_x * s) as i32,
                    (win_y * s) as i32,
                    (win_w * s) as i32,
                    (win_h * s) as i32,
                    SWP_NOZORDER | SWP_FRAMECHANGED,
                );
            }
        }
    }

    (x, y, w, h)
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

/// Get the monitor work area (logical coordinates) at the given physical pixel coordinates.
/// Returns (x, y, width, height, scale) of the monitor's work area.
/// Work area excludes taskbar, unlike full monitor bounds.
fn get_monitor_info_at(phys_x: i32, phys_y: i32) -> (f64, f64, f64, f64, f64) {
    unsafe {
        let point = POINT { x: phys_x, y: phys_y };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);

        // Get DPI for this monitor
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f64 / 96.0;

        // Get monitor work area (physical pixels)
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmonitor, &mut info).as_bool() {
            let work = info.rcWork;
            (
                work.left as f64 / scale,
                work.top as f64 / scale,
                (work.right - work.left) as f64 / scale,
                (work.bottom - work.top) as f64 / scale,
                scale,
            )
        } else {
            // Fallback: assume primary monitor at origin
            (0.0, 0.0, 1920.0, 1080.0, 1.0)
        }
    }
}

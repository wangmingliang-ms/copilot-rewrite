// Overlay window management module
// Manages the floating toolbar and preview popup windows
// Uses WS_EX_NOACTIVATE to prevent stealing focus from the source application

use log::{debug, info, warn};
use tauri::{AppHandle, LogicalPosition, Manager, Position};
use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::MonitorFromPoint;
use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, GetWindowLongW, SetWindowLongW,
    GWL_EXSTYLE, SM_CXMINTRACK, SM_CYMINTRACK, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// Toolbar window dimensions
/// Windows enforces a minimum track size (~136x39 physical pixels at 100% DPI)
/// We use the actual Windows minimum to avoid invisible whitespace
fn get_toolbar_size() -> (f64, f64) {
    unsafe {
        let min_w = GetSystemMetrics(SM_CXMINTRACK);
        let min_h = GetSystemMetrics(SM_CYMINTRACK);
        // Use the larger of our desired size or Windows minimum
        // These are physical pixels; Tauri will handle DPI internally
        let w = 48.0_f64.max(min_w as f64);
        let h = 48.0_f64.max(min_h as f64);
        (w, h)
    }
}

/// Preview window dimensions
const PREVIEW_WIDTH: f64 = 480.0;
const PREVIEW_HEIGHT: f64 = 400.0;

/// Offset from cursor position for the toolbar
const TOOLBAR_OFFSET_X: f64 = 8.0;
const TOOLBAR_OFFSET_Y: f64 = 16.0;

/// Apply WS_EX_NOACTIVATE and WS_EX_TOOLWINDOW styles to a window
/// This prevents the overlay window from stealing focus and hides it from the taskbar
pub fn apply_noactivate_style(hwnd: isize) {
    unsafe {
        let hwnd = HWND(hwnd as *mut _);
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
        let new_style = ex_style | WS_EX_NOACTIVATE.0 as i32 | WS_EX_TOOLWINDOW.0 as i32;
        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style);
        debug!("Applied WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW to window");
    }
}

/// Set up overlay window styles for both toolbar and preview windows
/// Should be called during app setup after windows are created
pub fn setup_overlay_windows(app_handle: &AppHandle) {
    info!("Setting up overlay windows...");

    let (toolbar_w, toolbar_h) = get_toolbar_size();
    info!("Toolbar size: {}x{} (accounting for Windows minimum)", toolbar_w, toolbar_h);

    // Apply non-focus-stealing style to toolbar
    if let Some(window) = app_handle.get_webview_window("toolbar") {
        match window.hwnd() {
            Ok(hwnd) => {
                apply_noactivate_style(hwnd.0 as isize);
                // Set to actual usable size
                let _ = window.set_size(tauri::PhysicalSize::new(toolbar_w as u32, toolbar_h as u32));
                info!("Toolbar window configured as non-activating overlay");
            }
            Err(e) => warn!("Failed to get toolbar HWND: {}", e),
        }
    }

    // Apply non-focus-stealing style to preview
    // NOTE: Preview needs WS_EX_TOOLWINDOW (hide from taskbar) but NOT WS_EX_NOACTIVATE
    // because it contains interactive buttons (Replace, Copy, Cancel) that need click events.
    // WebView2 in WS_EX_NOACTIVATE windows silently drops click events.
    if let Some(window) = app_handle.get_webview_window("preview") {
        match window.hwnd() {
            Ok(hwnd) => {
                unsafe {
                    let hwnd_win = HWND(hwnd.0 as *mut _);
                    let ex_style = GetWindowLongW(hwnd_win, GWL_EXSTYLE);
                    let new_style = ex_style | WS_EX_TOOLWINDOW.0 as i32;
                    SetWindowLongW(hwnd_win, GWL_EXSTYLE, new_style);
                }
                info!("Preview window configured with WS_EX_TOOLWINDOW (interactive, no taskbar)");
            }
            Err(e) => warn!("Failed to get preview HWND: {}", e),
        }
    }
}

/// Position and show the toolbar window near the given cursor coordinates
/// Converts physical pixel coordinates (from GetCursorPos) to logical coordinates (for Tauri)
/// Uses per-monitor DPI to handle multi-monitor setups with different scaling
pub fn show_toolbar_at(app_handle: &AppHandle, mouse_x: i32, mouse_y: i32) {
    if let Some(window) = app_handle.get_webview_window("toolbar") {
        let scale = get_scale_at(mouse_x, mouse_y);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);
        let (toolbar_w, toolbar_h) = get_toolbar_size();
        let toolbar_w_logical = toolbar_w / scale;
        let toolbar_h_logical = toolbar_h / scale;

        // Convert physical pixel coordinates to logical coordinates
        let logical_x = mouse_x as f64 / scale;
        let logical_y = mouse_y as f64 / scale;

        // Calculate position with offset from cursor (in logical pixels)
        let mut x = logical_x + TOOLBAR_OFFSET_X;
        let mut y = logical_y + TOOLBAR_OFFSET_Y;

        // Screen boundary detection (using logical dimensions)
        if x + toolbar_w_logical > screen_w {
            x = screen_w - toolbar_w_logical - 8.0;
        }
        if y + toolbar_h_logical > screen_h {
            y = logical_y - toolbar_h_logical - 8.0;
        }
        if x < 0.0 {
            x = 8.0;
        }
        if y < 0.0 {
            y = 8.0;
        }

        let position = LogicalPosition::new(x, y);
        if let Err(e) = window.set_position(Position::Logical(position)) {
            warn!("Failed to position toolbar: {}", e);
        }

        if let Err(e) = window.show() {
            warn!("Failed to show toolbar: {}", e);
        }

        debug!("Toolbar shown at logical ({:.0}, {:.0}) [physical ({}, {}), scale {:.2}]", x, y, mouse_x, mouse_y, scale);
    }
}

/// Position and show the preview window near the toolbar
/// Uses per-monitor DPI for accurate positioning
pub fn show_preview_at(app_handle: &AppHandle, mouse_x: i32, mouse_y: i32) {
    if let Some(window) = app_handle.get_webview_window("preview") {
        let scale = get_scale_at(mouse_x, mouse_y);
        let (screen_w, screen_h, _) = get_primary_screen_info(app_handle);
        let (_, toolbar_h) = get_toolbar_size();
        let toolbar_h_logical = toolbar_h / scale;

        // Convert physical to logical coordinates
        let logical_x = mouse_x as f64 / scale;
        let logical_y = mouse_y as f64 / scale;

        // Position preview below and slightly left of the mouse
        let mut x = logical_x - PREVIEW_WIDTH / 2.0;
        let mut y = logical_y + TOOLBAR_OFFSET_Y + toolbar_h_logical + 8.0;

        // Screen boundary detection
        if x + PREVIEW_WIDTH > screen_w {
            x = screen_w - PREVIEW_WIDTH - 8.0;
        }
        if y + PREVIEW_HEIGHT > screen_h {
            y = logical_y - PREVIEW_HEIGHT - toolbar_h_logical - 16.0;
        }
        if x < 0.0 {
            x = 8.0;
        }
        if y < 0.0 {
            y = 8.0;
        }

        let position = LogicalPosition::new(x, y);
        if let Err(e) = window.set_position(Position::Logical(position)) {
            warn!("Failed to position preview: {}", e);
        }

        if let Err(e) = window.show() {
            warn!("Failed to show preview: {}", e);
        }

        debug!("Preview shown at logical ({:.0}, {:.0})", x, y);
    }
}

/// Hide the toolbar window
pub fn hide_toolbar(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("toolbar") {
        let _ = window.hide();
    }
}

/// Hide the preview window
pub fn hide_preview(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("preview") {
        let _ = window.hide();
    }
}

/// Hide both toolbar and preview windows
pub fn hide_all(app_handle: &AppHandle) {
    hide_toolbar(app_handle);
    hide_preview(app_handle);
}

/// Get the scale factor for the monitor at the given physical coordinates
/// Uses per-monitor DPI awareness for multi-monitor setups with different scaling
fn get_scale_at(mouse_x: i32, mouse_y: i32) -> f64 {
    unsafe {
        let point = POINT { x: mouse_x, y: mouse_y };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        dpi_x as f64 / 96.0
    }
}

/// Get primary screen dimensions and scale factor
/// Returns (logical_width, logical_height, scale_factor)
/// Used for screen boundary detection
fn get_primary_screen_info(app_handle: &AppHandle) -> (f64, f64, f64) {
    if let Some(monitor) = app_handle
        .primary_monitor()
        .ok()
        .flatten()
    {
        let size = monitor.size();
        let scale = monitor.scale_factor();
        (size.width as f64 / scale, size.height as f64 / scale, scale)
    } else {
        (1920.0, 1080.0, 1.0)
    }
}

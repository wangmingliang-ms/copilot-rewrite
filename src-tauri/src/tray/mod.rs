// System tray module
// Creates a tray icon with Settings, Enable/Disable toggle, Restart, Quit, and version info
// Icon shows green dot (enabled) or red dot (disabled) in bottom-right corner

use log::info;
use std::sync::Arc;
use tauri::{
    image::Image as TauriImage,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::AppState;

/// Generate a tray icon with a colored status dot in the bottom-right corner.
/// `enabled` = true → green dot, false → red dot.
fn make_status_icon(base_icon: &TauriImage<'_>, enabled: bool) -> TauriImage<'static> {
    let width = base_icon.width();
    let height = base_icon.height();
    let rgba = base_icon.rgba();

    let mut pixels = rgba.to_vec();

    // Dot color: green (#22c55e) for enabled, red (#ef4444) for disabled
    let (dot_r, dot_g, dot_b) = if enabled {
        (0x22u8, 0xC5u8, 0x5Eu8)
    } else {
        (0xEFu8, 0x44u8, 0x44u8)
    };

    // Dot parameters — bottom-right corner
    // For 32×32 icon: radius ~5px, center at (26, 26)
    // For other sizes: scale proportionally
    let scale = width.min(height) as f32 / 32.0;
    let radius = (5.0 * scale).round() as i32;
    let cx = (width as i32) - radius - (2.0 * scale).round() as i32;
    let cy = (height as i32) - radius - (2.0 * scale).round() as i32;

    // Draw a white outline ring (1px wider) then the colored dot
    let outline_r = radius + (1.5 * scale).round().max(1.0) as i32;

    for y in (cy - outline_r)..=(cy + outline_r) {
        for x in (cx - outline_r)..=(cx + outline_r) {
            if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                continue;
            }
            let dx = x - cx;
            let dy = y - cy;
            let dist_sq = dx * dx + dy * dy;

            let idx = ((y as u32 * width + x as u32) * 4) as usize;
            if idx + 3 >= pixels.len() {
                continue;
            }

            if dist_sq <= radius * radius {
                // Inner colored dot
                pixels[idx] = dot_r;
                pixels[idx + 1] = dot_g;
                pixels[idx + 2] = dot_b;
                pixels[idx + 3] = 255;
            } else if dist_sq <= outline_r * outline_r {
                // White outline
                pixels[idx] = 255;
                pixels[idx + 1] = 255;
                pixels[idx + 2] = 255;
                pixels[idx + 3] = 255;
            }
        }
    }

    TauriImage::new_owned(pixels, width, height)
}

/// Set up the system tray icon with menu
pub fn setup_tray(
    app_handle: &AppHandle,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up system tray...");

    let version = app_handle.package_info().version.to_string();

    let version_item =
        MenuItemBuilder::with_id("version", format!("v{}", version)).build(app_handle)?;

    let settings_item =
        MenuItemBuilder::with_id("settings", "⚙️  Settings...").build(app_handle)?;

    let toggle_item =
        MenuItemBuilder::with_id("toggle", "⏸  Disable").build(app_handle)?;

    let restart_item =
        MenuItemBuilder::with_id("restart", "🔄  Restart").build(app_handle)?;

    let quit_item = MenuItemBuilder::with_id("quit", "❌  Quit").build(app_handle)?;

    let menu = MenuBuilder::new(app_handle)
        .item(&version_item)
        .separator()
        .item(&settings_item)
        .item(&toggle_item)
        .separator()
        .item(&restart_item)
        .item(&quit_item)
        .build()?;

    // Keep references for dynamic updates
    let toggle_ref = toggle_item.clone();

    // Generate initial icon with green status dot
    let base_icon = app_handle.default_window_icon().cloned().unwrap();
    let enabled_icon = make_status_icon(&base_icon, true);

    let tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .tooltip("Copilot Rewrite — Active")
        .icon(enabled_icon)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "version" => {
                    info!("Version clicked — opening release page");
                    let ver = app.package_info().version.to_string();
                    let url = format!(
                        "https://github.com/wangmingliang-ms/copilot-rewrite/releases/tag/v{}",
                        ver
                    );
                    #[cfg(target_os = "windows")]
                    {
                        let _ = std::process::Command::new("cmd")
                            .args(["/C", "start", "", &url])
                            .spawn();
                    }
                }
                "settings" => {
                    info!("Opening settings window");
                    if let Some(window) = app.get_webview_window("settings") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        #[cfg(target_os = "windows")]
                        {
                            use windows::Win32::Foundation::HWND;
                            use windows::Win32::UI::WindowsAndMessaging::{
                                BringWindowToTop, SetForegroundWindow, ShowWindow, SW_RESTORE,
                            };
                            if let Ok(hwnd) = window.hwnd() {
                                unsafe {
                                    let h = HWND(hwnd.0);
                                    let _ = ShowWindow(h, SW_RESTORE);
                                    let _ = BringWindowToTop(h);
                                    let _ = SetForegroundWindow(h);
                                }
                            }
                        }
                        let _ = window.set_focus();
                    }
                }
                "toggle" => {
                    let mut enabled = state.enabled.lock();
                    *enabled = !*enabled;
                    let is_enabled = *enabled;
                    info!("Toggled monitoring: enabled={}", is_enabled);

                    if let Some(window) = app.get_webview_window("toolbar") {
                        if !is_enabled {
                            let _ = window.hide();
                        }
                    }

                    // Update menu label
                    let new_label = if is_enabled {
                        "⏸  Disable"
                    } else {
                        "▶️  Enable"
                    };
                    let _ = toggle_ref.set_text(new_label);

                    // Update tray icon with status dot
                    let base = app.default_window_icon().cloned().unwrap();
                    let new_icon = make_status_icon(&base, is_enabled);
                    // Find and update the tray icon
                    if let Some(tray) = app.tray_by_id("main") {
                        let _ = tray.set_icon(Some(new_icon));
                        let tooltip = if is_enabled {
                            "Copilot Rewrite — Active"
                        } else {
                            "Copilot Rewrite — Disabled"
                        };
                        let _ = tray.set_tooltip(Some(tooltip));
                    }
                }
                "restart" => {
                    info!("Restart requested from tray");
                    if let Ok(exe) = std::env::current_exe() {
                        let _ = std::process::Command::new(exe).spawn();
                    }
                    app.exit(0);
                }
                "quit" => {
                    info!("Quit requested from tray");
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app_handle)?;

    // Store initial tray reference (needed for icon updates)
    let _ = tray;

    info!("System tray set up successfully");
    Ok(())
}

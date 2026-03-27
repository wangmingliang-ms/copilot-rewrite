// System tray module
// Creates a tray icon with Settings, Enable/Disable toggle, Restart, Quit, and version info
// Icon shows green dot (enabled) or red dot (disabled) in bottom-right corner
// Menu items use colored custom icons

use log::info;
use std::sync::Arc;
use tauri::{
    image::Image as TauriImage,
    menu::{IconMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::AppState;

/// Generate a tray icon with a colored status dot in the bottom-right corner.
fn make_status_icon(base_icon: &TauriImage<'_>, enabled: bool) -> TauriImage<'static> {
    let width = base_icon.width();
    let height = base_icon.height();
    let rgba = base_icon.rgba();
    let mut pixels = rgba.to_vec();

    let (dot_r, dot_g, dot_b) = if enabled {
        (0x22u8, 0xC5u8, 0x5Eu8)
    } else {
        (0xEFu8, 0x44u8, 0x44u8)
    };

    let scale = width.min(height) as f32 / 32.0;
    let radius = (5.0 * scale).round() as i32;
    let cx = (width as i32) - radius - (2.0 * scale).round() as i32;
    let cy = (height as i32) - radius - (2.0 * scale).round() as i32;
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
                pixels[idx] = dot_r;
                pixels[idx + 1] = dot_g;
                pixels[idx + 2] = dot_b;
                pixels[idx + 3] = 255;
            } else if dist_sq <= outline_r * outline_r {
                pixels[idx] = 255;
                pixels[idx + 1] = 255;
                pixels[idx + 2] = 255;
                pixels[idx + 3] = 255;
            }
        }
    }

    TauriImage::new_owned(pixels, width, height)
}

/// Create a 16×16 RGBA icon with a simple shape and color
fn make_menu_icon(r: u8, g: u8, b: u8, shape: &str) -> TauriImage<'static> {
    let size: u32 = 16;
    let mut pixels = vec![0u8; (size * size * 4) as usize];

    match shape {
        "gear" => {
            // Simple gear: circle with notches
            let cx = 8i32;
            let cy = 8i32;
            for y in 0..size as i32 {
                for x in 0..size as i32 {
                    let dx = x - cx;
                    let dy = y - cy;
                    let dist = ((dx * dx + dy * dy) as f32).sqrt();
                    let idx = ((y as u32 * size + x as u32) * 4) as usize;

                    // Outer ring (gear body)
                    if dist >= 3.0 && dist <= 7.0 {
                        // Cut out inner hole
                        if dist >= 4.5 {
                            pixels[idx] = r;
                            pixels[idx + 1] = g;
                            pixels[idx + 2] = b;
                            pixels[idx + 3] = 255;
                        }
                    }
                    // Gear teeth — 4 rectangular protrusions
                    let in_tooth = (x >= 6 && x <= 10 && (y <= 2 || y >= 13))
                        || (y >= 6 && y <= 10 && (x <= 2 || x >= 13));
                    if in_tooth {
                        pixels[idx] = r;
                        pixels[idx + 1] = g;
                        pixels[idx + 2] = b;
                        pixels[idx + 3] = 255;
                    }
                }
            }
        }
        "pause" => {
            // Two vertical bars
            for y in 2..14u32 {
                for x in 4..7u32 {
                    let idx = ((y * size + x) * 4) as usize;
                    pixels[idx] = r;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = b;
                    pixels[idx + 3] = 255;
                }
                for x in 9..12u32 {
                    let idx = ((y * size + x) * 4) as usize;
                    pixels[idx] = r;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = b;
                    pixels[idx + 3] = 255;
                }
            }
        }
        "play" => {
            // Right-pointing triangle
            for y in 2..14i32 {
                let row = y - 2; // 0..12
                let half_width = row.min(11 - row);
                for x in 5..(5 + half_width + 1) {
                    let idx = ((y as u32 * size + x as u32) * 4) as usize;
                    pixels[idx] = r;
                    pixels[idx + 1] = g;
                    pixels[idx + 2] = b;
                    pixels[idx + 3] = 255;
                }
            }
        }
        "refresh" => {
            // Circular arrow (simplified as a ¾ circle arc)
            let cx = 8i32;
            let cy = 8i32;
            for y in 0..size as i32 {
                for x in 0..size as i32 {
                    let dx = x - cx;
                    let dy = y - cy;
                    let dist = ((dx * dx + dy * dy) as f32).sqrt();
                    let angle = (dy as f32).atan2(dx as f32);

                    let idx = ((y as u32 * size + x as u32) * 4) as usize;

                    // ¾ circle ring (skip top-right quadrant for the gap)
                    if dist >= 4.5 && dist <= 6.5 {
                        // Skip roughly -π/4 to π/4 for the gap
                        if angle < -0.5 || angle > 0.5 {
                            pixels[idx] = r;
                            pixels[idx + 1] = g;
                            pixels[idx + 2] = b;
                            pixels[idx + 3] = 255;
                        }
                    }

                    // Arrow head at the gap end
                    if x >= 9 && x <= 13 && y >= 3 && y <= 7 {
                        let ax = x - 11;
                        let ay = y - 5;
                        if (ax + ay).abs() <= 2 && ax <= 2 {
                            pixels[idx] = r;
                            pixels[idx + 1] = g;
                            pixels[idx + 2] = b;
                            pixels[idx + 3] = 255;
                        }
                    }
                }
            }
        }
        "cross" => {
            // X shape for quit
            for i in 3..13i32 {
                for t in -1..=1i32 {
                    // Top-left to bottom-right diagonal
                    let y1 = i;
                    let x1 = i + t;
                    if x1 >= 0 && x1 < size as i32 && y1 >= 0 && y1 < size as i32 {
                        let idx = ((y1 as u32 * size + x1 as u32) * 4) as usize;
                        pixels[idx] = r;
                        pixels[idx + 1] = g;
                        pixels[idx + 2] = b;
                        pixels[idx + 3] = 255;
                    }
                    // Top-right to bottom-left diagonal
                    let y2 = i;
                    let x2 = (15 - i) + t;
                    if x2 >= 0 && x2 < size as i32 && y2 >= 0 && y2 < size as i32 {
                        let idx = ((y2 as u32 * size + x2 as u32) * 4) as usize;
                        pixels[idx] = r;
                        pixels[idx + 1] = g;
                        pixels[idx + 2] = b;
                        pixels[idx + 3] = 255;
                    }
                }
            }
        }
        "info" => {
            // Info circle (ⓘ) — filled circle with "i" letter inside
            let cx = 8.0f32;
            let cy = 8.0f32;
            let outer_r = 7.0f32;
            let inner_r = 5.5f32;
            for y in 0..size {
                for x in 0..size {
                    let dx = x as f32 - cx;
                    let dy = y as f32 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let idx = ((y * size + x) * 4) as usize;

                    if dist <= outer_r {
                        // Fill the circle background
                        pixels[idx] = r;
                        pixels[idx + 1] = g;
                        pixels[idx + 2] = b;
                        pixels[idx + 3] = 255;

                        // Cut out the "i" letter in white
                        if dist <= inner_r {
                            let is_dot = y >= 3 && y <= 5 && x >= 7 && x <= 9;
                            let is_stem = y >= 7 && y <= 12 && x >= 7 && x <= 9;
                            if is_dot || is_stem {
                                pixels[idx] = 255;
                                pixels[idx + 1] = 255;
                                pixels[idx + 2] = 255;
                                pixels[idx + 3] = 255;
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    TauriImage::new_owned(pixels, size, size)
}

/// Set up the system tray icon with menu
pub fn setup_tray(
    app_handle: &AppHandle,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up system tray...");

    let version = app_handle.package_info().version.to_string();

    // Create colored menu icons
    let info_icon = make_menu_icon(0x64, 0x9E, 0xF0, "info"); // blue info circle
    let gear_icon = make_menu_icon(0x8B, 0x8B, 0x8B, "gear"); // gray
    let pause_icon = make_menu_icon(0xF5, 0xA6, 0x23, "pause"); // amber/orange
    let refresh_icon = make_menu_icon(0x22, 0xC5, 0x5E, "refresh"); // green
    let cross_icon = make_menu_icon(0xEF, 0x44, 0x44, "cross"); // red

    let version_item = IconMenuItemBuilder::with_id("version", format!("v{}", version))
        .icon(info_icon)
        .build(app_handle)?;

    let settings_item = IconMenuItemBuilder::with_id("settings", "Settings...")
        .icon(gear_icon)
        .build(app_handle)?;

    let toggle_item = IconMenuItemBuilder::with_id("toggle", "Disable")
        .icon(pause_icon)
        .build(app_handle)?;

    let restart_item = IconMenuItemBuilder::with_id("restart", "Restart")
        .icon(refresh_icon)
        .build(app_handle)?;

    let quit_item = IconMenuItemBuilder::with_id("quit", "Quit")
        .icon(cross_icon)
        .build(app_handle)?;

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

    let _tray = TrayIconBuilder::with_id("main")
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

                    // Update menu label and icon
                    if is_enabled {
                        let _ = toggle_ref.set_text("Disable");
                        let pause = make_menu_icon(0xF5, 0xA6, 0x23, "pause");
                        let _ = toggle_ref.set_icon(Some(pause));
                    } else {
                        let _ = toggle_ref.set_text("Enable");
                        let play = make_menu_icon(0x22, 0xC5, 0x5E, "play");
                        let _ = toggle_ref.set_icon(Some(play));
                    }

                    // Update tray icon with status dot
                    let base = app.default_window_icon().cloned().unwrap();
                    let new_icon = make_status_icon(&base, is_enabled);
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

    info!("System tray set up successfully");
    Ok(())
}

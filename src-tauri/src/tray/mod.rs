// System tray module
// Creates a tray icon with Settings, Enable/Disable toggle, Restart, Quit, and version info

use log::info;
use std::sync::Arc;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

use crate::AppState;

/// Set up the system tray icon with menu
pub fn setup_tray(
    app_handle: &AppHandle,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Setting up system tray...");

    let version = app_handle.package_info().version.to_string();

    let version_item = MenuItemBuilder::with_id("version", format!("v{}", version))
        .build(app_handle)?;

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

    // Keep a reference to the toggle item for updating text
    let toggle_ref = toggle_item.clone();

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Copilot Rewrite")
        .icon(app_handle.default_window_icon().cloned().unwrap())
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

                    let new_label = if is_enabled {
                        "⏸  Disable"
                    } else {
                        "▶️  Enable"
                    };
                    let _ = toggle_ref.set_text(new_label);
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

// System tray module
// Creates a tray icon with Settings, Enable/Disable toggle, Restart, and Quit options

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

    let settings_item = MenuItemBuilder::with_id("settings", "Settings...").build(app_handle)?;

    let toggle_item = MenuItemBuilder::with_id("toggle", "Disable").build(app_handle)?;

    let restart_item = MenuItemBuilder::with_id("restart", "Restart").build(app_handle)?;

    let quit_item = MenuItemBuilder::with_id("quit", "Quit").build(app_handle)?;

    let menu = MenuBuilder::new(app_handle)
        .item(&settings_item)
        .separator()
        .item(&toggle_item)
        .separator()
        .item(&restart_item)
        .item(&quit_item)
        .build()?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Copilot Rewrite")
        .icon(app_handle.default_window_icon().cloned().unwrap())
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "settings" => {
                    info!("Opening settings window");
                    if let Some(window) = app.get_webview_window("settings") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                }
                "toggle" => {
                    let mut enabled = state.enabled.lock();
                    *enabled = !*enabled;
                    let new_label = if *enabled { "Disable" } else { "Enable" };
                    info!("Toggled monitoring: enabled={}", *enabled);

                    if let Some(window) = app.get_webview_window("toolbar") {
                        if !*enabled {
                            let _ = window.hide();
                        }
                    }

                    if let Some(menu) = app.menu() {
                        if let Some(item) = menu.get("toggle") {
                            if let Some(menu_item) = item.as_menuitem() {
                                let _ = menu_item.set_text(new_label);
                            }
                        }
                    }
                }
                "restart" => {
                    info!("Restart requested from tray");
                    // Spawn new instance after a short delay to allow lock file release
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

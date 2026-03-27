// Autostart module
// Registers/unregisters the application in Windows startup via registry
// Uses HKCU\Software\Microsoft\Windows\CurrentVersion\Run

use anyhow::{Context, Result};
use log::info;
use winreg::enums::*;
use winreg::RegKey;

/// Registry path for auto-start entries
const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Registry value name for this application
const APP_NAME: &str = "CopilotRewrite";

/// Register the application to start on Windows login
pub fn register_autostart() -> Result<()> {
    let exe_path = std::env::current_exe().context("Failed to get current executable path")?;

    let exe_path_str = exe_path
        .to_str()
        .context("Executable path contains invalid Unicode")?;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(RUN_KEY_PATH)
        .context("Failed to open/create Run registry key")?;

    key.set_value(APP_NAME, &exe_path_str)
        .context("Failed to set registry value")?;

    info!("Registered auto-start: {} -> {}", APP_NAME, exe_path_str);
    Ok(())
}

/// Unregister the application from Windows startup
pub fn unregister_autostart() -> Result<()> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    match hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_WRITE) {
        Ok(key) => {
            match key.delete_value(APP_NAME) {
                Ok(_) => info!("Removed auto-start registration for {}", APP_NAME),
                Err(e) => {
                    // Not an error if the value doesn't exist
                    if e.kind() != std::io::ErrorKind::NotFound {
                        return Err(e).context("Failed to delete registry value");
                    }
                    info!("Auto-start was not registered");
                }
            }
        }
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                return Err(e).context("Failed to open Run registry key");
            }
            info!("Run registry key does not exist");
        }
    }

    Ok(())
}

/// Check if auto-start is currently registered
pub fn is_autostart_registered() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    if let Ok(key) = hkcu.open_subkey(RUN_KEY_PATH) {
        key.get_value::<String, _>(APP_NAME).is_ok()
    } else {
        false
    }
}

// Text replacement engine
// Uses clipboard injection + simulated Ctrl+V to replace text in any application

use crate::clipboard;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write as IoWrite;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    VK_CONTROL, VK_V,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, GetForegroundWindow, SetForegroundWindow, ASFW_ANY,
};

fn debug_log(msg: &str) {
    // Use Roaming AppData (same as auth.json location)
    if let Some(dir) = dirs::config_dir() {
        let log_path = dir.join("copilot-rewrite").join("replace-debug.log");
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis();
            let _ = writeln!(f, "[{}] {}", secs, msg);
        }
    }
}

/// Replace the currently selected text in the specified source application
pub fn replace_selected_text(text: &str, source_hwnd: Option<isize>, html: Option<&str>) -> Result<()> {
    debug_log(&format!(
        "=== REPLACE START === text_len={}, html={}, source_hwnd={:?}",
        text.len(),
        html.is_some(),
        source_hwnd
    ));

    // Step 1: Check current foreground window
    let current_fg = unsafe { GetForegroundWindow() };
    debug_log(&format!("Current foreground HWND: {:?}", current_fg.0));

    // Step 2: Restore focus to the source application
    if let Some(hwnd) = source_hwnd {
        debug_log(&format!("Activating source window HWND: {}", hwnd));
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            let target = HWND(hwnd as *mut _);
            let result = SetForegroundWindow(target);
            debug_log(&format!("SetForegroundWindow result: {}", result.as_bool()));
        }
        // Wait for window activation
        thread::sleep(Duration::from_millis(300));

        // Verify focus switched
        let new_fg = unsafe { GetForegroundWindow() };
        debug_log(&format!(
            "After activation, foreground HWND: {:?}",
            new_fg.0
        ));
    } else {
        debug_log("WARNING: No source HWND provided!");
    }

    // Step 3: Write text to clipboard (with optional HTML for rich paste)
    debug_log("Setting clipboard...");
    if let Some(html_content) = html {
        debug_log("Using rich HTML clipboard format");
        clipboard::set_html(html_content, text).context("Failed to write HTML to clipboard")?;
    } else {
        debug_log("Using plain text clipboard format");
        clipboard::set_text(text).context("Failed to write to clipboard")?;
    }
    debug_log("Clipboard set successfully");

    thread::sleep(Duration::from_millis(100));

    // Step 4: Simulate Ctrl+V
    debug_log("Sending Ctrl+V...");
    let sent = unsafe {
        let inputs = [
            make_key_input(VK_CONTROL, false),
            make_key_input(VK_V, false),
            make_key_input(VK_V, true),
            make_key_input(VK_CONTROL, true),
        ];
        let size = std::mem::size_of::<INPUT>() as i32;
        debug_log(&format!("INPUT struct size: {}", size));
        SendInput(&inputs, size)
    };
    debug_log(&format!("SendInput returned: {} (expected 4)", sent));

    if sent != 4 {
        let err = std::io::Error::last_os_error();
        debug_log(&format!("SendInput FAILED! LastError: {:?}", err));
        anyhow::bail!("SendInput returned {}, last error: {}", sent, err);
    }

    // Wait for paste to take effect
    thread::sleep(Duration::from_millis(300));
    debug_log("=== REPLACE DONE ===");

    Ok(())
}

/// Create a keyboard INPUT structure for SendInput
fn make_key_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    let mut flags = windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS(0);
    if key_up {
        flags = KEYEVENTF_KEYUP;
    }

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

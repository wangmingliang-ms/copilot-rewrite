// Copilot Rewrite - Library root
// System-level text translation and polishing tool for Windows

pub mod autostart;
pub mod clipboard;
pub mod copilot;
pub mod overlay;
pub mod replacement;
pub mod selection;
pub mod tray;

use log::{info, warn};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{Emitter, Manager};

/// Global application state shared across all modules
#[derive(Debug)]
pub struct AppState {
    /// Whether the selection monitoring is enabled
    pub enabled: Mutex<bool>,
    /// Whether the preview window is currently showing (pauses UIA monitoring)
    pub preview_visible: Mutex<bool>,
    /// The currently selected text (if any)
    pub current_selection: Mutex<Option<SelectionInfo>>,
    /// User settings
    pub settings: Mutex<Settings>,
    /// Copilot API client
    pub copilot_client: copilot::CopilotClient,
    /// Pending OAuth device code (during login flow)
    pub pending_device_code: Mutex<Option<copilot::DeviceCodeResponse>>,
}

impl AppState {
    pub fn new() -> Self {
        // Load settings from disk (or defaults)
        let mut settings = Settings::load();

        // Override token from saved auth if available
        if let Some(saved_auth) = copilot::oauth::load_saved_auth() {
            settings.api_token = saved_auth.github_token;
            info!("Loaded saved GitHub token for user: {:?}", saved_auth.username);
        }

        Self {
            enabled: Mutex::new(true),
            preview_visible: Mutex::new(false),
            current_selection: Mutex::new(None),
            settings: Mutex::new(settings),
            copilot_client: copilot::CopilotClient::new(),
            pending_device_code: Mutex::new(None),
        }
    }
}

/// Information about a text selection event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionInfo {
    /// The selected text content
    pub text: String,
    /// Mouse X position when selection was detected
    pub mouse_x: i32,
    /// Mouse Y position when selection was detected
    pub mouse_y: i32,
    /// Source method used to detect the selection
    pub source: SelectionSource,
    /// HWND of the source application window (for restoring focus)
    #[serde(skip)]
    pub source_hwnd: Option<isize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SelectionSource {
    /// Detected via UI Automation TextPattern
    UIA,
    /// Detected via clipboard monitoring (fallback)
    Clipboard,
}

/// User-configurable settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// Target language for translation (default: "English")
    pub target_language: String,
    /// Whether auto-start on Windows login is enabled
    pub auto_start: bool,
    /// List of process names to ignore (app blacklist)
    pub blacklisted_apps: Vec<String>,
    /// Copilot API token
    pub api_token: String,
    /// Polling interval in milliseconds (50-500)
    pub poll_interval_ms: u64,
    /// AI model to use (e.g. "gpt-4o", "claude-3.5-sonnet")
    pub model: String,
}

impl Settings {
    /// Path to the settings JSON file
    fn settings_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|d| d.join("copilot-rewrite").join("settings.json"))
    }

    /// Load settings from disk, falling back to defaults
    pub fn load() -> Self {
        if let Some(path) = Self::settings_path() {
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(json) => {
                        match serde_json::from_str::<Settings>(&json) {
                            Ok(s) => {
                                info!("Loaded settings from {:?}", path);
                                return s;
                            }
                            Err(e) => warn!("Failed to parse settings.json: {}", e),
                        }
                    }
                    Err(e) => warn!("Failed to read settings.json: {}", e),
                }
            }
        }
        Self::default()
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::settings_path()
            .ok_or_else(|| "Cannot determine config directory".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write settings.json: {}", e))?;
        info!("Settings saved to {:?}", path);
        Ok(())
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            target_language: "English".to_string(),
            auto_start: false,
            blacklisted_apps: vec![],
            api_token: String::new(),
            poll_interval_ms: 100,
            model: "claude-sonnet-4".to_string(),
        }
    }
}

/// The action the user wants to perform on selected text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RewriteAction {
    Translate,
    Polish,
    TranslateAndPolish,
}

/// Request from frontend to process text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRequest {
    pub text: String,
    pub action: RewriteAction,
}

/// Response from the Copilot API processing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessResponse {
    pub original: String,
    pub result: String,
    pub action: RewriteAction,
}

/// Auth status for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub username: Option<String>,
}

// ─── Tauri Commands ───────────────────────────────────────────────

/// Process selected text with Copilot API
#[tauri::command]
async fn process_text(
    state: tauri::State<'_, Arc<AppState>>,
    request: ProcessRequest,
) -> Result<ProcessResponse, String> {
    let settings = state.settings.lock().clone();

    if settings.api_token.is_empty() {
        return Err("Not logged in. Please click the ⚙ button to log in with GitHub.".to_string());
    }

    let result = state
        .copilot_client
        .process(&request.text, &request.action, &settings.target_language, &settings.api_token, &settings.model)
        .await
        .map_err(|e| format!("Copilot API error: {}", e))?;

    Ok(ProcessResponse {
        original: request.text,
        result,
        action: request.action,
    })
}

/// Process text and show result in popup window
/// This is the main flow: icon click → spinning → API call → expand with result
#[tauri::command]
async fn process_and_show_preview(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    request: ProcessRequest,
) -> Result<(), String> {
    let settings = state.settings.lock().clone();

    if settings.api_token.is_empty() {
        return Err("Not logged in. Please sign in via Settings.".to_string());
    }

    // Mark popup as "processing" to pause UIA monitoring
    *state.preview_visible.lock() = true;

    // Emit loading event (frontend switches to spinning state)
    app.emit("show-preview-loading", ()).map_err(|e| e.to_string())?;

    // Call Copilot API
    match state.copilot_client
        .process(&request.text, &request.action, &settings.target_language, &settings.api_token, &settings.model)
        .await
    {
        Ok(result) => {
            // Expand popup window to fit result text
            overlay::expand_popup(&app, &result);

            let response = ProcessResponse {
                original: request.text,
                result,
                action: request.action,
            };
            app.emit("show-preview-result", &response).map_err(|e| e.to_string())?;
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("Copilot API error: {}", e);
            let _ = app.emit("show-preview-error", &err_msg);
            Err(err_msg)
        }
    }
}

/// Start GitHub OAuth Device Flow - returns device code info for user
#[tauri::command]
async fn start_github_login(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<copilot::DeviceCodeResponse, String> {
    let http = reqwest::Client::new();
    let device_code = copilot::oauth::request_device_code(&http)
        .await
        .map_err(|e| format!("Login failed: {}", e))?;

    // Store device code in state for polling
    *state.pending_device_code.lock() = Some(device_code.clone());

    Ok(device_code)
}

/// Poll for GitHub OAuth token completion
#[tauri::command]
async fn poll_github_login(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<AuthStatus, String> {
    let device_info = state.pending_device_code.lock().clone();

    let device_info = device_info.ok_or("No pending login. Call start_github_login first.")?;

    let http = reqwest::Client::new();
    let token = copilot::oauth::poll_for_token(&http, &device_info.device_code, device_info.interval)
        .await
        .map_err(|e| format!("Login failed: {}", e))?;

    // Save token to settings
    {
        let mut settings = state.settings.lock();
        settings.api_token = token;
    }

    // Clear pending device code
    *state.pending_device_code.lock() = None;

    // Load saved auth for username
    let username = copilot::oauth::load_saved_auth().and_then(|a| a.username);

    Ok(AuthStatus {
        logged_in: true,
        username,
    })
}

/// Check current auth status
#[tauri::command]
fn get_auth_status(state: tauri::State<'_, Arc<AppState>>) -> AuthStatus {
    let settings = state.settings.lock();
    let has_token = !settings.api_token.is_empty();

    let username = if has_token {
        copilot::oauth::load_saved_auth().and_then(|a| a.username)
    } else {
        None
    };

    AuthStatus {
        logged_in: has_token,
        username,
    }
}

/// Log out - clear saved auth
#[tauri::command]
fn logout(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    // Clear token from settings
    {
        let mut settings = state.settings.lock();
        settings.api_token.clear();
    }
    // Delete saved auth file
    copilot::oauth::delete_saved_auth().map_err(|e| format!("Logout failed: {}", e))?;
    Ok(())
}

/// List available AI models from Copilot API
#[tauri::command]
async fn list_models(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<Vec<copilot::client::CopilotModel>, String> {
    let token = state.settings.lock().api_token.clone();
    state
        .copilot_client
        .list_models(&token)
        .await
        .map_err(|e| format!("Failed to list models: {}", e))
}

/// Open a URL in the default browser (fallback for shell plugin)
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    std::process::Command::new("cmd")
        .args(["/C", "start", &url])
        .spawn()
        .map_err(|e| format!("Failed to open URL: {}", e))?;
    Ok(())
}

/// Open the settings window
#[tauri::command]
fn open_settings(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
        Ok(())
    } else {
        Err("Settings window not found".to_string())
    }
}

/// Replace selected text in the source application
/// Must restore focus to the original app window before pasting
/// IMPORTANT: SendInput must run on a dedicated thread (not tokio async pool)
#[tauri::command]
async fn replace_text(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    text: String,
) -> Result<(), String> {
    log::info!("[REPLACE CMD] replace_text called, text_len={}", text.len());

    // Temporarily pause selection monitoring to prevent toolbar re-appearing
    *state.enabled.lock() = false;
    *state.preview_visible.lock() = false;

    // Get source window HWND before hiding preview
    let source_hwnd = state.current_selection.lock().as_ref()
        .and_then(|s| s.source_hwnd);
    log::info!("[REPLACE CMD] source_hwnd={:?}", source_hwnd);

    // Hide popup first
    overlay::hide_popup(&app);
    log::info!("[REPLACE CMD] popup hidden");

    // Run replacement on a dedicated OS thread (NOT tokio pool)
    // SendInput requires proper thread context for input injection
    let text_clone = text.clone();
    let result = tokio::task::spawn_blocking(move || {
        replacement::replace_selected_text(&text_clone, source_hwnd)
    })
    .await
    .map_err(|e| format!("Thread error: {}", e))?
    .map_err(|e| format!("Replace error: {}", e));

    // Re-enable monitoring after delay
    let state_clone = state.inner().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        *state_clone.enabled.lock() = true;
    });

    result
}

/// Copy text to clipboard
#[tauri::command]
async fn copy_to_clipboard(text: String) -> Result<(), String> {
    clipboard::set_text(&text).map_err(|e| format!("Clipboard error: {}", e))
}

/// Get current settings
#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<AppState>>) -> Settings {
    state.settings.lock().clone()
}

/// Update settings
#[tauri::command]
fn update_settings(
    state: tauri::State<'_, Arc<AppState>>,
    settings: Settings,
) -> Result<(), String> {
    // Save to disk first
    settings.save()?;
    // Then update in-memory state
    let mut current = state.settings.lock();
    *current = settings;
    Ok(())
}

/// Toggle the enabled state
#[tauri::command]
fn toggle_enabled(state: tauri::State<'_, Arc<AppState>>) -> bool {
    let mut enabled = state.enabled.lock();
    *enabled = !*enabled;
    *enabled
}

/// Get current enabled state
#[tauri::command]
fn is_enabled(state: tauri::State<'_, Arc<AppState>>) -> bool {
    *state.enabled.lock()
}

/// Dismiss the popup (hide + shrink back to icon size) and briefly pause monitoring
#[tauri::command]
async fn dismiss_popup(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    overlay::hide_popup(&app);
    *state.preview_visible.lock() = false;
    // Briefly pause monitoring so popup doesn't immediately re-appear
    *state.enabled.lock() = false;
    let state_clone = state.inner().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(500));
        *state_clone.enabled.lock() = true;
    });
    Ok(())
}

// ─── App Entry Point ──────────────────────────────────────────────

pub fn run() {
    env_logger::init();

    let app_state = Arc::new(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            process_text,
            process_and_show_preview,
            replace_text,
            copy_to_clipboard,
            get_settings,
            update_settings,
            toggle_enabled,
            is_enabled,
            dismiss_popup,
            start_github_login,
            poll_github_login,
            get_auth_status,
            logout,
            list_models,
            open_url,
            open_settings,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let state = app_state.clone();

            info!("Copilot Rewrite starting up...");

            // Apply window styles to popup (WS_EX_NOACTIVATE, strip frame)
            overlay::setup_popup_window(&app_handle);

            // Set up the system tray icon with Enable/Disable/Quit menu
            if let Err(e) = tray::setup_tray(&app_handle, state.clone()) {
                log::error!("Failed to set up system tray: {}", e);
            }

            // Start the selection monitoring engine in a background thread
            let engine_handle = app_handle.clone();
            let engine_state = state.clone();
            std::thread::spawn(move || {
                selection::start_selection_engine(engine_handle, engine_state);
            });

            info!("Selection engine started");

            // Set up auto-start if configured
            let settings = state.settings.lock().clone();
            if settings.auto_start {
                if let Err(e) = autostart::register_autostart() {
                    log::error!("Failed to register auto-start: {}", e);
                }
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Copilot Rewrite");
}

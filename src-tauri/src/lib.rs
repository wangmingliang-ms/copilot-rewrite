// Copilot Rewrite - Library root
// System-level text translation and polishing tool for Windows

pub mod autostart;
pub mod clipboard;
pub mod copilot;
pub mod overlay;
pub mod replacement;
pub mod selection;
pub mod tray;

use log::{info, warn, LevelFilter};
use parking_lot::Mutex;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Global debug mode flag — checked by the log formatter to decide whether
/// DEBUG-level messages should be written to the log file.
pub static DEBUG_MODE: AtomicBool = AtomicBool::new(false);
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
    /// Incremented on dismiss — signals monitor to reset its local state
    pub selection_generation: std::sync::atomic::AtomicU64,
    /// User settings
    pub settings: Mutex<Settings>,
    /// Copilot API client (for chat completions — has its own HTTP client with 30s timeout)
    pub copilot_client: copilot::CopilotClient,
    /// Shared HTTP client for lightweight requests (OAuth, GitHub API, etc.)
    pub http_client: reqwest::Client,
    /// Pending OAuth device code (during login flow)
    pub pending_device_code: Mutex<Option<copilot::DeviceCodeResponse>>,
    /// Cancellation token for in-flight LLM requests
    pub cancel_token: Mutex<tokio_util::sync::CancellationToken>,
}

impl AppState {
    pub fn new() -> Self {
        // Load settings from disk (or defaults)
        let mut settings = Settings::load();

        // Override token from saved auth if available
        if let Some(saved_auth) = copilot::oauth::load_saved_auth() {
            settings.api_token = saved_auth.github_token;
            info!(
                "Loaded saved GitHub token for user: {:?}",
                saved_auth.username
            );
        }

        // Initialize global debug mode flag from settings
        DEBUG_MODE.store(settings.debug_mode, Ordering::Relaxed);

        Self {
            enabled: Mutex::new(true),
            preview_visible: Mutex::new(false),
            current_selection: Mutex::new(None),
            selection_generation: std::sync::atomic::AtomicU64::new(0),
            settings: Mutex::new(settings),
            copilot_client: copilot::CopilotClient::new(),
            http_client: reqwest::Client::new(),
            pending_device_code: Mutex::new(None),
            cancel_token: Mutex::new(tokio_util::sync::CancellationToken::new()),
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
    /// Bounding rect of the input element (physical pixels, optional)
    #[serde(skip)]
    pub input_rect: Option<(i32, i32, i32, i32)>, // (x, y, w, h)
    /// Name of the source application (e.g. "Teams", "chrome")
    #[serde(default)]
    pub app_name: String,
    /// Window title of the source application
    #[serde(default)]
    pub window_title: String,
    /// Whether the selection is from an input/editable element (Write Mode)
    /// false = non-input element (Read Mode)
    #[serde(default = "default_true")]
    pub is_input_element: bool,
    /// Resolved replace mode for this selection ("markdown", "rendered", or "plain").
    /// Determined at selection time based on app_name, window_title, and replace rules.
    #[serde(default = "default_replace_mode_str")]
    pub replace_mode: String,
}

fn default_replace_mode_str() -> String {
    "rendered".to_string()
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum SelectionSource {
    /// Detected via UI Automation TextPattern
    UIA,
    /// Detected via clipboard monitoring (fallback)
    Clipboard,
}

/// Replace mode for pasting translated text back into the target application
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReplaceMode {
    /// Paste Markdown source text (for apps like GitHub, GitLab, Reddit, Slack)
    Markdown,
    /// Render Markdown to HTML and paste via CF_HTML (for Teams, Outlook, Word, etc.)
    Rendered,
    /// Strip all Markdown formatting and paste plain text (for Notepad, terminals, etc.)
    Plain,
}

impl Default for ReplaceMode {
    fn default() -> Self {
        ReplaceMode::Rendered
    }
}

/// A rule for auto-detecting replace mode based on process name and window title
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplaceRule {
    /// Process name substring match (case-insensitive)
    pub process: String,
    /// Window title substring match (case-insensitive). Empty = match process only.
    pub title_contains: String,
    /// The replace mode to use when this rule matches
    pub mode: ReplaceMode,
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
    /// "More Creative" mode — LLM freely rewrites with full creative freedom
    #[serde(default)]
    pub creative_mode: bool,
    /// AI model to use (e.g. "gpt-4o", "claude-3.5-sonnet")
    pub model: String,
    /// Global default replace mode when no rule matches
    #[serde(default)]
    pub global_replace_mode: ReplaceMode,
    /// Ordered list of rules for auto-detecting replace mode by app/window title
    #[serde(default = "default_replace_rules")]
    pub replace_rules: Vec<ReplaceRule>,
    /// Theme: "system" (follow OS), "light", or "dark"
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Native language (user's mother tongue, for Read Mode translation direction)
    #[serde(default = "default_native_language")]
    pub native_language: String,
    /// Whether Read Mode is enabled (triggers on non-input element selections)
    #[serde(default = "default_true")]
    pub read_mode_enabled: bool,
    /// Read Mode sub-mode: "translate_summarize" or "simple_translate"
    #[serde(default = "default_read_mode_sub")]
    pub read_mode_sub: String,
    /// Popup icon position relative to selected text bounding rect
    /// Values: "top-center", "top-left", "top-right", "bottom-center", "bottom-left", "bottom-right"
    #[serde(default = "default_popup_icon_position")]
    pub popup_icon_position: String,
    /// Write Mode action: "TranslateAndPolish", "Translate", or "Polish"
    #[serde(default = "default_write_action")]
    pub write_action: String,
    /// Debug mode — logs detailed information (LLM prompts, responses) to the log file
    #[serde(default)]
    pub debug_mode: bool,
}

fn default_replace_rules() -> Vec<ReplaceRule> {
    vec![
        // Browsers + GitHub → Markdown
        ReplaceRule { process: "msedge".into(), title_contains: "github.com".into(), mode: ReplaceMode::Markdown },
        ReplaceRule { process: "chrome".into(), title_contains: "github.com".into(), mode: ReplaceMode::Markdown },
        ReplaceRule { process: "firefox".into(), title_contains: "github.com".into(), mode: ReplaceMode::Markdown },
        // Browsers + GitLab → Markdown
        ReplaceRule { process: "msedge".into(), title_contains: "gitlab".into(), mode: ReplaceMode::Markdown },
        ReplaceRule { process: "chrome".into(), title_contains: "gitlab".into(), mode: ReplaceMode::Markdown },
        // Browsers + Reddit → Markdown
        ReplaceRule { process: "msedge".into(), title_contains: "reddit.com".into(), mode: ReplaceMode::Markdown },
        ReplaceRule { process: "chrome".into(), title_contains: "reddit.com".into(), mode: ReplaceMode::Markdown },
        // Rich text apps → Rendered
        ReplaceRule { process: "ms-teams".into(), title_contains: String::new(), mode: ReplaceMode::Rendered },
        ReplaceRule { process: "Teams".into(), title_contains: String::new(), mode: ReplaceMode::Rendered },
        ReplaceRule { process: "OUTLOOK".into(), title_contains: String::new(), mode: ReplaceMode::Rendered },
        ReplaceRule { process: "WINWORD".into(), title_contains: String::new(), mode: ReplaceMode::Rendered },
        ReplaceRule { process: "lark".into(), title_contains: String::new(), mode: ReplaceMode::Rendered },
        // Plain text apps → Plain
        ReplaceRule { process: "notepad".into(), title_contains: String::new(), mode: ReplaceMode::Plain },
        ReplaceRule { process: "WindowsTerminal".into(), title_contains: String::new(), mode: ReplaceMode::Plain },
        ReplaceRule { process: "Code".into(), title_contains: String::new(), mode: ReplaceMode::Plain },
        ReplaceRule { process: "cmd".into(), title_contains: String::new(), mode: ReplaceMode::Plain },
        ReplaceRule { process: "powershell".into(), title_contains: String::new(), mode: ReplaceMode::Plain },
    ]
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_native_language() -> String {
    "Chinese (Simplified)".to_string()
}

fn default_read_mode_sub() -> String {
    "translate_summarize".to_string()
}

fn default_popup_icon_position() -> String {
    "top-left".to_string()
}

fn default_write_action() -> String {
    "TranslateAndPolish".to_string()
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
                    Ok(json) => match serde_json::from_str::<Settings>(&json) {
                        Ok(s) => {
                            info!("Loaded settings from {:?}", path);
                            return s;
                        }
                        Err(e) => warn!("Failed to parse settings.json: {}", e),
                    },
                    Err(e) => warn!("Failed to read settings.json: {}", e),
                }
            }
        }
        Self::default()
    }

    /// Save settings to disk
    pub fn save(&self) -> Result<(), String> {
        let path =
            Self::settings_path().ok_or_else(|| "Cannot determine config directory".to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize settings: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("Failed to write settings.json: {}", e))?;
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
            creative_mode: true,
            model: "claude-sonnet-4".to_string(),
            global_replace_mode: ReplaceMode::Rendered,
            replace_rules: default_replace_rules(),
            theme: "system".to_string(),
            native_language: "Chinese (Simplified)".to_string(),
            read_mode_enabled: true,
            read_mode_sub: "translate_summarize".to_string(),
            popup_icon_position: "top-left".to_string(),
            write_action: "TranslateAndPolish".to_string(),
            debug_mode: false,
        }
    }
}

/// The action the user wants to perform on selected text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RewriteAction {
    Translate,
    Polish,
    TranslateAndPolish,
    /// Read Mode: translate (and optionally summarize) non-input text
    ReadModeTranslate,
}

/// Request from frontend to process text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessRequest {
    pub text: String,
    pub action: RewriteAction,
    #[serde(default)]
    pub is_refresh: bool,
    /// For ReadModeTranslate: the target language to translate into
    #[serde(default)]
    pub read_target_language: String,
    /// For ReadModeTranslate: whether to include a summary
    #[serde(default)]
    pub read_summarize: bool,
    /// Optional creative_mode override per-request ("More Creative" mode)
    #[serde(default)]
    pub creative_mode: Option<bool>,
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

    let result = match &request.action {
        RewriteAction::ReadModeTranslate => {
            state
                .copilot_client
                .process_read_mode(
                    &request.text,
                    &settings.native_language,
                    &settings.target_language,
                    &settings.api_token,
                    &settings.model,
                    None,
                    None,
                )
                .await
                .map_err(|e| format!("Copilot API error: {}", e))?
        }
        _ => {
            state
                .copilot_client
                .process(
                    &request.text,
                    &request.action,
                    &settings.native_language,
                    &settings.target_language,
                    &settings.api_token,
                    &settings.model,
                    settings.creative_mode,
                    "",
                    None,
                    None,
                )
                .await
                .map_err(|e| format!("Copilot API error: {}", e))?
        }
    };

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

    // Get app context from current selection for prompt contextualization
    let app_context = {
        let sel = state.current_selection.lock();
        sel.as_ref()
            .map(|s| format!("App: {}, Window: {}", s.app_name, s.window_title))
            .unwrap_or_default()
    };

    // Create a fresh cancellation token for this request
    let cancel_token = tokio_util::sync::CancellationToken::new();
    *state.cancel_token.lock() = cancel_token.clone();

    // Mark popup as "processing" to pause UIA monitoring
    *state.preview_visible.lock() = true;

    let t0 = std::time::Instant::now();
    let creative_mode = request.creative_mode.unwrap_or(settings.creative_mode);
    info!("[PERF] process_and_show_preview START (action={:?}, model={}, creative={}, refresh={}, text_len={})",
        request.action, settings.model, creative_mode, request.is_refresh, request.text.len());

    // Emit loading event (frontend switches to loading state)
    app.emit("show-preview-loading", ())
        .map_err(|e| e.to_string())?;

    // Expand popup immediately so the full UI (toolbar + loading spinner) is visible
    if !request.is_refresh {
        overlay::expand_popup_streaming(&app);
    }
    info!("[PERF] +{}ms — emitted show-preview-loading, popup expanded", t0.elapsed().as_millis());

    // Resolve replace mode only for Write Mode (Read Mode has no Replace button).
    // Done at icon-click time so the cost is hidden behind the LLM wait.
    if !matches!(request.action, RewriteAction::ReadModeTranslate) {
        let sel = state.current_selection.lock();
        let (app_name, window_title) = match sel.as_ref() {
            Some(s) => (s.app_name.as_str(), s.window_title.as_str()),
            None => ("", ""),
        };
        let mode = resolve_replace_mode(app_name, window_title, &settings.replace_rules, &settings.global_replace_mode);
        let mode_str = match mode {
            ReplaceMode::Markdown => "markdown",
            ReplaceMode::Rendered => "rendered",
            ReplaceMode::Plain => "plain",
        };
        info!("[PERF] +{}ms — resolved replace mode: {} (app={}, title={})", t0.elapsed().as_millis(), mode_str, app_name, window_title);
        let _ = app.emit("replace-mode-resolved", mode_str);
    }

    // Call Copilot API with cancellation support
    let native_lang = settings.native_language.clone();
    let target_lang = settings.target_language.clone();

    // Build chunk callback that emits streaming events
    let app_clone = app.clone();
    let chunk_callback = move |accumulated: &str| {
        let _ = app_clone.emit("show-preview-chunk", accumulated);
    };

    info!("[PERF] +{}ms — LLM call starting with streaming...", t0.elapsed().as_millis());

    let process_result: anyhow::Result<String> = match &request.action {
        RewriteAction::ReadModeTranslate => {
            state.copilot_client.process_read_mode(
                &request.text,
                &native_lang,
                &target_lang,
                &settings.api_token,
                &settings.model,
                Some(&chunk_callback),
                Some(&cancel_token),
            ).await
        }
        _ => {
            state.copilot_client.process(
                &request.text,
                &request.action,
                &native_lang,
                &target_lang,
                &settings.api_token,
                &settings.model,
                creative_mode,
                &app_context,
                Some(&chunk_callback),
                Some(&cancel_token),
            ).await
        }
    };

    let llm_ms = t0.elapsed().as_millis();
    match process_result {
        Ok(result_text) => {
            info!("[PERF] +{}ms — LLM response received ({} chars)", llm_ms, result_text.len());
            // Expand popup to final size based on result text (for non-refresh requests)
            if !request.is_refresh {
                overlay::expand_popup(&app, &result_text);
            }
            info!("[PERF] +{}ms — popup expanded", t0.elapsed().as_millis());

            let response = ProcessResponse {
                original: request.text,
                result: result_text,
                action: request.action,
            };
            app.emit("show-preview-result", &response)
                .map_err(|e| e.to_string())?;
            info!("[PERF] +{}ms — emitted show-preview-result, DONE", t0.elapsed().as_millis());
            Ok(())
        }
        Err(e) => {
            let err_msg = format!("{}", e);
            // Check if this was a cancellation
            if err_msg.contains("cancelled") {
                info!("[PERF] +{}ms — Request cancelled by user", llm_ms);
                *state.preview_visible.lock() = false;
                let _ = app.emit("request-cancelled", ());
                Err("Request cancelled".to_string())
            } else {
                let err_msg = format!("Copilot API error: {}", e);
                warn!("[PERF] +{}ms — LLM ERROR: {}", llm_ms, err_msg);
                *state.preview_visible.lock() = false;
                let _ = app.emit("show-preview-error", &err_msg);
                Err(err_msg)
            }
        }
    }
}

/// Cancel the current in-flight LLM request
#[tauri::command]
fn cancel_request(state: tauri::State<'_, Arc<AppState>>) {
    info!("[POPUP] Cancel requested by user");
    state.cancel_token.lock().cancel();
}

/// Start GitHub OAuth Device Flow - returns device code info for user
#[tauri::command]
async fn start_github_login(
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<copilot::DeviceCodeResponse, String> {
    let device_code = copilot::oauth::request_device_code(&state.http_client)
        .await
        .map_err(|e| format!("Login failed: {}", e))?;

    // Store device code in state for polling
    *state.pending_device_code.lock() = Some(device_code.clone());

    Ok(device_code)
}

/// Poll for GitHub OAuth token completion
#[tauri::command]
async fn poll_github_login(state: tauri::State<'_, Arc<AppState>>) -> Result<AuthStatus, String> {
    let device_info = state.pending_device_code.lock().clone();

    let device_info = device_info.ok_or("No pending login. Call start_github_login first.")?;

    let token =
        copilot::oauth::poll_for_token(&state.http_client, &device_info.device_code, device_info.interval)
            .await
            .map_err(|e| format!("Login failed: {}", e))?;

    // Save token to settings (in memory and to disk)
    {
        let mut settings = state.settings.lock();
        settings.api_token = token;
        if let Err(e) = settings.save() {
            warn!("Failed to save settings after login: {}", e);
        }
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
async fn get_auth_status(state: tauri::State<'_, Arc<AppState>>) -> Result<AuthStatus, String> {
    let settings = state.settings.lock().clone();
    let has_token = !settings.api_token.is_empty();

    let username = if has_token {
        // Try loading from saved auth file first
        let saved = copilot::oauth::load_saved_auth().and_then(|a| a.username);
        if saved.is_some() {
            saved
        } else {
            // auth.json missing or has no username — fetch from GitHub API
            match state.http_client
                .get("https://api.github.com/user")
                .header("Authorization", format!("token {}", settings.api_token))
                .header("User-Agent", "CopilotRewrite/0.1.0")
                .header("Accept", "application/json")
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    #[derive(Deserialize)]
                    struct GhUser {
                        login: String,
                    }
                    match resp.json::<GhUser>().await {
                        Ok(user) => {
                            // Re-save auth.json with username
                            let auth = copilot::oauth::SavedAuth {
                                github_token: settings.api_token.clone(),
                                username: Some(user.login.clone()),
                            };
                            let _ = copilot::oauth::save_auth(&auth);
                            Some(user.login)
                        }
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        }
    } else {
        None
    };

    Ok(AuthStatus {
        logged_in: has_token,
        username,
    })
}

/// Log out - clear saved auth
#[tauri::command]
fn logout(state: tauri::State<'_, Arc<AppState>>) -> Result<(), String> {
    // Clear token and model from settings (in memory)
    {
        let mut settings = state.settings.lock();
        settings.api_token.clear();
        settings.model.clear();
        // Save cleared settings to disk so token doesn't persist across restart
        if let Err(e) = settings.save() {
            warn!("Failed to save settings after logout: {}", e);
        }
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
        let _ = window.unminimize();
        // Force bring to front using Win32 API — Tauri's set_focus() may fail
        // when called from a background or WS_EX_NOACTIVATE window
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                BringWindowToTop, SetForegroundWindow, ShowWindow, SW_RESTORE,
            };
            if let Ok(hwnd) = window.hwnd() {
                unsafe {
                    let h = HWND(hwnd.0 as *mut _);
                    let _ = ShowWindow(h, SW_RESTORE);
                    let _ = BringWindowToTop(h);
                    let _ = SetForegroundWindow(h);
                }
            }
        }
        let _ = window.set_focus();
        Ok(())
    } else {
        Err("Settings window not found".to_string())
    }
}

/// Open today's log file in the default text editor
#[tauri::command]
fn open_log_file() -> Result<(), String> {
    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("copilot-rewrite")
        .join("logs");
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let log_path = log_dir.join(format!("{}.log", today));
    if log_path.exists() {
        std::process::Command::new("explorer")
            .arg(&log_path)
            .spawn()
            .map_err(|e| format!("Failed to open log file: {}", e))?;
        Ok(())
    } else {
        // Try opening the logs directory instead
        if log_dir.exists() {
            std::process::Command::new("explorer")
                .arg(&log_dir)
                .spawn()
                .map_err(|e| format!("Failed to open logs directory: {}", e))?;
            Ok(())
        } else {
            Err("No log files found".to_string())
        }
    }
}

/// Open the logs directory in Explorer
#[tauri::command]
fn open_log_dir() -> Result<(), String> {
    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("copilot-rewrite")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    std::process::Command::new("explorer")
        .arg(&log_dir)
        .spawn()
        .map_err(|e| format!("Failed to open logs directory: {}", e))?;
    Ok(())
}

/// Resolve which replace mode to use based on app_name, window_title, and rules.
/// Iterates through rules in order; first match wins. Falls back to default_mode.
pub fn resolve_replace_mode(
    app_name: &str,
    window_title: &str,
    rules: &[ReplaceRule],
    default_mode: &ReplaceMode,
) -> ReplaceMode {
    let app_lower = app_name.to_lowercase();
    let title_lower = window_title.to_lowercase();
    for rule in rules {
        let process_lower = rule.process.to_lowercase();
        if app_lower.contains(&process_lower) {
            if rule.title_contains.is_empty() || title_lower.contains(&rule.title_contains.to_lowercase()) {
                return rule.mode.clone();
            }
        }
    }
    default_mode.clone()
}

/// Strip Markdown formatting and return plain text using pulldown-cmark.
pub fn strip_markdown(markdown: &str) -> String {
    let parser = Parser::new(markdown);
    let mut output = String::with_capacity(markdown.len());

    for event in parser {
        match event {
            Event::Text(text) => output.push_str(&text),
            Event::Code(code) => output.push_str(&code),
            Event::SoftBreak | Event::HardBreak => output.push('\n'),
            Event::Start(Tag::CodeBlock(_)) => {}
            Event::End(TagEnd::CodeBlock) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::Start(Tag::Paragraph) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::Start(Tag::Item) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str("• ");
            }
            Event::End(TagEnd::Item) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::Start(Tag::Heading { .. }) => {
                if !output.is_empty() && !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
            }
            _ => {}
        }
    }

    // Trim trailing whitespace
    output.trim_end().to_string()
}

/// Get the auto-detected replace mode for the current selection
#[tauri::command]
fn get_replace_mode(state: tauri::State<'_, Arc<AppState>>) -> String {
    let settings = state.settings.lock().clone();
    let selection = state.current_selection.lock().clone();
    let (app_name, window_title) = match &selection {
        Some(sel) => (sel.app_name.as_str(), sel.window_title.as_str()),
        None => ("", ""),
    };
    let mode = resolve_replace_mode(app_name, window_title, &settings.replace_rules, &settings.global_replace_mode);
    match mode {
        ReplaceMode::Markdown => "markdown".to_string(),
        ReplaceMode::Rendered => "rendered".to_string(),
        ReplaceMode::Plain => "plain".to_string(),
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
    html: Option<String>,
    mode_override: Option<String>,
) -> Result<(), String> {
    // Replace mode is always provided by the frontend (resolved at selection time).
    // Parse the mode string; fall back to Rendered if somehow missing.
    let effective_mode = match mode_override.as_deref() {
        Some("markdown") => ReplaceMode::Markdown,
        Some("plain") => ReplaceMode::Plain,
        Some("rendered") | _ => ReplaceMode::Rendered,
    };

    log::info!("[REPLACE CMD] replace_text called, text_len={}, html={}, mode={:?}",
        text.len(), html.is_some(), effective_mode);

    // Temporarily pause selection monitoring to prevent toolbar re-appearing
    *state.enabled.lock() = false;
    *state.preview_visible.lock() = false;

    // Get source window HWND before hiding preview
    let source_hwnd = state
        .current_selection
        .lock()
        .as_ref()
        .and_then(|s| s.source_hwnd);
    log::info!("[REPLACE CMD] source_hwnd={:?}", source_hwnd);

    // Hide popup first
    overlay::hide_popup(&app);
    log::info!("[REPLACE CMD] popup hidden");

    // Prepare clipboard content based on mode
    let (final_text, final_html) = match effective_mode {
        ReplaceMode::Markdown => {
            // Paste raw markdown source text
            (text.clone(), None)
        }
        ReplaceMode::Rendered => {
            // Paste rendered HTML (if available) via CF_HTML
            (text.clone(), html.clone())
        }
        ReplaceMode::Plain => {
            // Strip markdown formatting and paste plain text
            let plain = strip_markdown(&text);
            (plain, None)
        }
    };

    // Run replacement on a dedicated OS thread (NOT tokio pool)
    // SendInput requires proper thread context for input injection
    let result = tokio::task::spawn_blocking(move || {
        replacement::replace_selected_text(&final_text, source_hwnd, final_html.as_deref())
    })
    .await
    .map_err(|e| format!("Thread error: {}", e))?
    .map_err(|e| format!("Replace error: {}", e));

    // Re-enable monitoring after delay.
    // 800ms allows slow apps (Outlook, Teams) to process the paste before
    // UIA polling resumes. Also bump generation to prevent the pasted text
    // from being detected as a new selection.
    let state_clone = state.inner().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(800));
        state_clone
            .selection_generation
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *state_clone.enabled.lock() = true;
    });

    result
}

/// Copy text to clipboard
#[tauri::command]
async fn copy_to_clipboard(text: String) -> Result<(), String> {
    info!("[POPUP] Copied to clipboard — {} chars", text.len());
    clipboard::set_text(&text).map_err(|e| format!("Clipboard error: {}", e))
}

/// Copy HTML + plain text to clipboard (rich text mode)
#[tauri::command]
async fn copy_html_to_clipboard(html: String, text: String) -> Result<(), String> {
    info!("[POPUP] Copied HTML to clipboard — html={} chars, text={} chars", html.len(), text.len());
    clipboard::set_html(&html, &text).map_err(|e| format!("Clipboard error: {}", e))
}

/// Log a frontend action (for actions that don't call backend)
#[tauri::command]
fn log_action(action: String) {
    info!("[POPUP] {}", action);
}

/// Get current settings
#[tauri::command]
fn get_settings(state: tauri::State<'_, Arc<AppState>>) -> Settings {
    state.settings.lock().clone()
}

/// Detect whether Windows is in dark mode by reading the registry
#[tauri::command]
fn get_system_theme() -> String {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = hkcu.open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize") {
        if let Ok(val) = key.get_value::<u32, _>("AppsUseLightTheme") {
            return if val == 0 { "dark".to_string() } else { "light".to_string() };
        }
    }
    "light".to_string()
}

/// Update settings
#[tauri::command]
fn update_settings(
    state: tauri::State<'_, Arc<AppState>>,
    settings: Settings,
) -> Result<(), String> {
    // Sync global debug mode flag
    DEBUG_MODE.store(settings.debug_mode, Ordering::Relaxed);
    // Save to disk first
    settings.save()?;
    // Then update in-memory state
    let mut current = state.settings.lock();
    *current = settings;
    Ok(())
}

/// Upsert a replace rule based on user's choice in the popup.
/// If an existing rule already matches this (app_name, window_title), update its mode.
/// Otherwise, create a new rule — for browser processes, extract a domain from the title;
/// for other apps, use process-only matching.
#[tauri::command]
fn upsert_replace_rule(
    state: tauri::State<'_, Arc<AppState>>,
    app_name: String,
    window_title: String,
    mode: String,
) -> Result<(), String> {
    let replace_mode = match mode.as_str() {
        "markdown" => ReplaceMode::Markdown,
        "plain" => ReplaceMode::Plain,
        _ => ReplaceMode::Rendered,
    };

    let mut settings = state.settings.lock();

    // 1) Check if an existing rule matches this (app_name, window_title).
    //    Use the same matching logic as resolve_replace_mode.
    let app_lower = app_name.to_lowercase();
    let title_lower = window_title.to_lowercase();
    let mut found = false;
    for rule in settings.replace_rules.iter_mut() {
        let process_lower = rule.process.to_lowercase();
        if app_lower.contains(&process_lower) {
            if rule.title_contains.is_empty() || title_lower.contains(&rule.title_contains.to_lowercase()) {
                // This rule currently matches — update its mode
                rule.mode = replace_mode.clone();
                found = true;
                info!("Updated existing replace rule: process={}, title={}, mode={}", rule.process, rule.title_contains, mode);
                break;
            }
        }
    }

    if !found {
        // 2) No matching rule — create a new one.
        //    For browsers, try to extract a domain from window title.
        let title_pattern = extract_title_pattern(&app_lower, &title_lower);
        settings.replace_rules.insert(
            0,
            ReplaceRule {
                process: app_name.clone(),
                title_contains: title_pattern.clone(),
                mode: replace_mode,
            },
        );
        info!("Inserted new replace rule: process={}, title={}, mode={}", app_name, title_pattern, mode);
    }

    settings.save().map_err(|e| e.to_string())?;
    Ok(())
}

/// Extract a meaningful title_contains pattern from a window title.
/// For browsers, tries to find a domain (e.g. "github.com" from "...github.com...").
/// For non-browser apps, returns empty string (match by process only).
fn extract_title_pattern(app_lower: &str, title_lower: &str) -> String {
    let browsers = ["edge", "chrome", "firefox", "brave", "opera", "safari", "arc"];
    let is_browser = browsers.iter().any(|b| app_lower.contains(b));

    if !is_browser {
        return String::new();
    }

    // Try to find a domain-like pattern in the title: something.tld
    // Browser titles often look like: "Page Title - site.com - Microsoft Edge"
    // We scan for words that look like domains
    for word in title_lower.split(|c: char| c.is_whitespace() || c == '-' || c == '—' || c == '|') {
        let word = word.trim();
        if word.contains('.') && !word.starts_with('.') && !word.ends_with('.') {
            // Looks like a domain — check it has a TLD-like part
            let parts: Vec<&str> = word.split('.').collect();
            if parts.len() >= 2 && parts.last().map_or(false, |tld| tld.len() >= 2) {
                return word.to_string();
            }
        }
    }

    String::new()
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

/// Dismiss the popup (hide + shrink back to icon size) and signal monitor to reset
#[tauri::command]
async fn dismiss_popup(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
) -> Result<(), String> {
    info!("[POPUP] Dismissed");
    overlay::hide_popup(&app);
    *state.preview_visible.lock() = false;
    *state.current_selection.lock() = None;
    // Bump generation — monitor will clear its local state when it sees this
    state
        .selection_generation
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// Resize popup to fit actual rendered content height (called from frontend after render)
#[tauri::command]
async fn resize_popup_content(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    overlay::resize_popup_to_content(&app, height);
    Ok(())
}

// ─── App Entry Point ──────────────────────────────────────────────

pub fn run() {
    // Set up logging to both stderr and date-rotated log files
    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("copilot-rewrite")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_dir_for_closure = log_dir.clone();

    // Write a session separator to today's log
    {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let log_path = log_dir.join(format!("{}.log", today));
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            use std::io::Write;
            let _ = writeln!(
                f,
                "\n--- Session started: {} ---",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }
    }

    env_logger::Builder::new()
        .filter_level(LevelFilter::Info)
        .filter_module("copilot_rewrite", LevelFilter::Debug)
        .format({
            // Cache the log file handle — only re-open when the date changes
            let cached_log_file: Arc<Mutex<Option<(String, std::fs::File)>>> =
                Arc::new(Mutex::new(None));
            move |buf, record| {
                use std::io::Write;
                let now = chrono::Local::now();
                let line = format!(
                    "[{} {} {}] {}\n",
                    now.format("%Y-%m-%d %H:%M:%S%.3f"),
                    record.level(),
                    record.module_path().unwrap_or(""),
                    record.args()
                );
                // Write to stderr (all levels — for terminal debugging)
                let _ = buf.write_all(line.as_bytes());
                // Write to date-rotated log file (INFO+ always, DEBUG when debug mode is on)
                if record.level() <= log::Level::Info
                    || (record.level() == log::Level::Debug && DEBUG_MODE.load(Ordering::Relaxed))
                {
                    let today = now.format("%Y-%m-%d").to_string();
                    let mut guard = cached_log_file.lock();
                    // Re-open if no cached handle or date has changed
                    let needs_reopen = match guard.as_ref() {
                        Some((cached_date, _)) => *cached_date != today,
                        None => true,
                    };
                    if needs_reopen {
                        let log_path = log_dir_for_closure.join(format!("{}.log", &today));
                        if let Ok(f) = std::fs::OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&log_path)
                        {
                            *guard = Some((today, f));
                        }
                    }
                    if let Some((_, ref mut f)) = *guard {
                        let _ = f.write_all(line.as_bytes());
                        // No flush per line — OS will flush on its own schedule,
                        // or we flush on date rotation / process exit
                    }
                }
                Ok(())
            }
        })
        .init();

    let app_state = Arc::new(AppState::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            process_text,
            process_and_show_preview,
            cancel_request,
            replace_text,
            copy_to_clipboard,
            copy_html_to_clipboard,
            get_settings,
            get_system_theme,
            update_settings,
            upsert_replace_rule,
            toggle_enabled,
            is_enabled,
            dismiss_popup,
            resize_popup_content,
            start_github_login,
            poll_github_login,
            get_auth_status,
            logout,
            list_models,
            open_url,
            open_settings,
            open_log_file,
            open_log_dir,
            log_action,
            get_replace_mode,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let state = app_state.clone();

            info!("Copilot Rewrite starting up...");

            // Apply window styles to popup (WS_EX_NOACTIVATE, strip frame)
            overlay::setup_popup_window(&app_handle);

            // Settings window: hide on close instead of destroy, so it can be re-shown
            if let Some(settings_win) = app_handle.get_webview_window("settings") {
                let settings_handle = app_handle.clone();
                settings_win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(win) = settings_handle.get_webview_window("settings") {
                            let _ = win.hide();
                        }
                    }
                });
            }

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

            // Inject version into splash screen
            if let Some(splash) = app_handle.get_webview_window("splashscreen") {
                let version = app.package_info().version.to_string();
                let _ = splash.eval(&format!(
                    "document.getElementById('version').textContent = 'v{}'",
                    version
                ));
            }

            // Close splash screen after a brief display (min 1.5s for branding)
            let splash_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                if let Some(splash) = splash_handle.get_webview_window("splashscreen") {
                    // Trigger CSS fade-out animation
                    let _ = splash.eval(
                        "document.getElementById('splash').classList.add('fade-out');"
                    );
                    // Wait for animation to complete, then close
                    tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                    let _ = splash.close();
                    info!("Splash screen closed");
                }
            });

            // Set up auto-start if configured
            let settings = state.settings.lock().clone();
            if settings.auto_start {
                if let Err(e) = autostart::register_autostart() {
                    log::error!("Failed to register auto-start: {}", e);
                }
            }

            // Check for updates on startup (after 10s delay)
            let update_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                info!("Checking for updates on startup...");
                use tauri_plugin_updater::UpdaterExt;
                match update_handle.updater().expect("updater").check().await {
                    Ok(Some(update)) => {
                        let new_version = update.version.clone();
                        info!("Update available: v{}", new_version);

                        // Show system notification
                        use tauri_plugin_notification::NotificationExt;
                        let _ = update_handle.notification()
                            .builder()
                            .title("🔄 Copilot Rewrite Update Available")
                            .body(format!(
                                "v{} is ready! Open Settings to update.",
                                new_version
                            ))
                            .show();

                        // Auto-open Settings window so user can update immediately
                        // (Windows toast notifications don't support reliable click callbacks)
                        if let Some(settings_win) = update_handle.get_webview_window("settings") {
                            let _ = settings_win.show();
                            let _ = settings_win.set_focus();
                        }
                    }
                    Ok(None) => {
                        info!("App is up to date");
                    }
                    Err(e) => {
                        log::warn!("Failed to check for updates: {}", e);
                    }
                }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Copilot Rewrite");
}

use anyhow::{Context, Result};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::process::{Output, Stdio};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration, Instant};
use tokio_util::sync::CancellationToken;

const TENANT_ID: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const FOUNDRY_RESOURCE: &str = "https://ai.azure.com";
const TOKEN_REFRESH_BUFFER_SECONDS: i64 = 5 * 60;
const CLI_COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
const LOGIN_PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(250);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
static COMMAND_CAPTURE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub environment_override: bool,
    pub cli_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AzureAccount {
    tenant_id: String,
    user: AzureUser,
}

#[derive(Debug, Deserialize)]
struct AzureUser {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AzureAccessToken {
    access_token: String,
    expires_on: UnixTimestamp,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum UnixTimestamp {
    Number(i64),
    String(String),
}

impl UnixTimestamp {
    fn into_i64(self) -> Result<i64> {
        match self {
            Self::Number(value) => Ok(value),
            Self::String(value) => value
                .parse()
                .context("Azure CLI returned an invalid token expiration time"),
        }
    }
}

#[derive(Debug)]
struct CachedAccessToken {
    value: String,
    expires_at: i64,
}

impl CachedAccessToken {
    fn is_usable(&self, now: i64) -> bool {
        self.expires_at - TOKEN_REFRESH_BUFFER_SECONDS > now
    }
}

pub struct AzureCliAuth {
    pending_login: Mutex<PendingLogin>,
    access_token_cache: Mutex<Option<CachedAccessToken>>,
    command_lock: Mutex<()>,
}

struct PendingLogin {
    cancel: Arc<CancellationToken>,
    existing_sign_in_windows: Vec<isize>,
}

pub(crate) struct AzureCliLoginAttempt {
    cancel: Arc<CancellationToken>,
    existing_sign_in_windows: Vec<isize>,
}

impl PendingLogin {
    fn idle() -> Self {
        let cancel = Arc::new(CancellationToken::new());
        cancel.cancel();
        Self {
            cancel,
            existing_sign_in_windows: Vec::new(),
        }
    }
}

impl std::fmt::Debug for AzureCliAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AzureCliAuth")
            .field("tenant_id", &TENANT_ID)
            .finish_non_exhaustive()
    }
}

impl AzureCliAuth {
    pub fn new() -> Self {
        remove_legacy_app_credentials();
        Self {
            pending_login: Mutex::new(PendingLogin::idle()),
            access_token_cache: Mutex::new(None),
            command_lock: Mutex::new(()),
        }
    }

    pub async fn status(&self) -> AuthStatus {
        let _command_guard = self.command_lock.lock().await;
        self.status_unlocked().await
    }

    async fn status_unlocked(&self) -> AuthStatus {
        match current_account().await {
            Ok(Some(account)) if account.tenant_id.eq_ignore_ascii_case(TENANT_ID) => {
                auth_status(Some(account.user.name), true)
            }
            Ok(Some(account)) => {
                debug!(
                    "Azure CLI is signed in to tenant {}, expected {}",
                    account.tenant_id, TENANT_ID
                );
                auth_status(None, true)
            }
            Ok(None) => auth_status(None, true),
            Err(error) if error.downcast_ref::<AzureCliNotInstalled>().is_some() => {
                auth_status(None, false)
            }
            Err(error) => {
                warn!("Failed to inspect Azure CLI account: {error:#}");
                auth_status(None, true)
            }
        }
    }

    pub(crate) async fn begin_login(&self) -> AzureCliLoginAttempt {
        let cancel = Arc::new(CancellationToken::new());
        let existing_sign_in_windows = sign_in_windows();
        {
            let mut pending = self.pending_login.lock().await;
            if !pending.cancel.is_cancelled() {
                pending.cancel.cancel();
                close_new_sign_in_windows(&pending.existing_sign_in_windows);
            }
            *pending = PendingLogin {
                cancel: Arc::clone(&cancel),
                existing_sign_in_windows: existing_sign_in_windows.clone(),
            };
        }

        AzureCliLoginAttempt {
            cancel,
            existing_sign_in_windows,
        }
    }

    pub(crate) async fn login(&self, attempt: AzureCliLoginAttempt) -> Result<AuthStatus> {
        let _command_guard = self.command_lock.lock().await;
        info!("Starting Azure CLI browser login");
        let result = if attempt.cancel.is_cancelled() {
            Err(anyhow::anyhow!("Azure CLI sign-in was cancelled."))
        } else {
            run_az_login(attempt.cancel.as_ref()).await
        };
        if result.is_err() {
            close_new_sign_in_windows(&attempt.existing_sign_in_windows);
        }
        self.clear_login_cancel(&attempt.cancel).await;
        let account = result?;
        info!("[AUTH LOGIN] Azure CLI process exited successfully");
        *self.access_token_cache.lock().await = None;

        if !account.tenant_id.eq_ignore_ascii_case(TENANT_ID) {
            anyhow::bail!(
                "Azure CLI login returned tenant {}, expected {}.",
                account.tenant_id,
                TENANT_ID
            );
        }
        Ok(auth_status(Some(account.user.name), true))
    }

    pub async fn access_token(&self) -> Result<String> {
        if let Some(token) = self.cached_access_token().await? {
            return Ok(token);
        }

        let _command_guard = self.command_lock.lock().await;
        if let Some(token) = self.cached_access_token().await? {
            return Ok(token);
        }

        let output = run_az(&[
            "account",
            "get-access-token",
            "--tenant",
            TENANT_ID,
            "--resource",
            FOUNDRY_RESOURCE,
            "--query",
            "{accessToken:accessToken,expiresOn:expires_on}",
            "--output",
            "json",
        ])
        .await?;
        let output = ensure_success(
            output,
            "Azure CLI could not acquire a Microsoft Foundry access token",
        )?;
        let token: AzureAccessToken = serde_json::from_slice(&output.stdout)
            .context("Failed to parse the Microsoft Foundry access token from Azure CLI")?;
        let value = token.access_token.trim();
        if value.is_empty() {
            anyhow::bail!("Azure CLI returned an empty Microsoft Foundry access token.");
        }
        let cached = CachedAccessToken {
            value: value.to_string(),
            expires_at: token.expires_on.into_i64()?,
        };
        let value = cached.value.clone();
        *self.access_token_cache.lock().await = Some(cached);
        Ok(value)
    }

    pub async fn cancel_login(&self) {
        let pending = self.pending_login.lock().await;
        pending.cancel.cancel();
        close_new_sign_in_windows(&pending.existing_sign_in_windows);
    }

    async fn cached_access_token(&self) -> Result<Option<String>> {
        let now = unix_timestamp()?;
        let cache = self.access_token_cache.lock().await;
        Ok(cache
            .as_ref()
            .filter(|token| token.is_usable(now))
            .map(|token| token.value.clone()))
    }

    async fn clear_login_cancel(&self, cancel: &Arc<CancellationToken>) {
        let mut pending = self.pending_login.lock().await;
        if Arc::ptr_eq(&pending.cancel, cancel) {
            *pending = PendingLogin::idle();
        }
    }
}

impl Default for AzureCliAuth {
    fn default() -> Self {
        Self::new()
    }
}

fn auth_status(username: Option<String>, cli_available: bool) -> AuthStatus {
    let logged_in = username.is_some();
    AuthStatus {
        display_name: username.clone(),
        username,
        logged_in,
        environment_override: false,
        cli_available,
    }
}

async fn current_account() -> Result<Option<AzureAccount>> {
    let output = run_az(&[
        "account",
        "show",
        "--query",
        "{tenantId:tenantId,user:user}",
        "--output",
        "json",
    ])
    .await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if requires_login(&stderr) {
            return Ok(None);
        }
        anyhow::bail!("Azure CLI account check failed: {}", stderr.trim());
    }

    let account = serde_json::from_slice(&output.stdout)
        .context("Failed to parse the active Azure CLI account")?;
    Ok(Some(account))
}

async fn run_az(args: &[&str]) -> Result<Output> {
    run_az_command(
        az_command(args),
        None,
        CLI_COMMAND_TIMEOUT,
        args.first().copied().unwrap_or("command"),
    )
    .await
}

async fn run_az_login(cancel: &CancellationToken) -> Result<AzureAccount> {
    let query = format!("[?tenantId=='{TENANT_ID}'] | [0].{{tenantId:tenantId,user:user}}");
    let command = az_command(&[
        "login",
        "--tenant",
        TENANT_ID,
        "--allow-no-subscriptions",
        "--query",
        &query,
        "--output",
        "json",
    ]);
    let output = run_az_command(command, Some(cancel), LOGIN_TIMEOUT, "login").await?;
    let output = ensure_success(output, "Azure CLI login failed")?;
    serde_json::from_slice(&output.stdout)
        .context("Azure CLI login completed but returned an invalid account")
}

fn az_command(args: &[&str]) -> Command {
    #[cfg(windows)]
    let mut command = if let Some(python) = azure_cli_python() {
        let mut command = Command::new(python);
        command.args(["-IBm", "azure.cli"]);
        command
    } else {
        Command::new("az.cmd")
    };
    #[cfg(not(windows))]
    let mut command = Command::new("az");

    command.args(args).kill_on_drop(true);
    if args.first() == Some(&"login") {
        command.env("AZURE_CORE_LOGIN_EXPERIENCE_V2", "off");
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    command
}

async fn run_az_command(
    mut command: Command,
    cancel: Option<&CancellationToken>,
    command_timeout: Duration,
    label: &str,
) -> Result<Output> {
    let capture = CommandCapture::new();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(capture.stdout_file()?))
        .stderr(Stdio::from(capture.stderr_file()?));

    let mut child = command.spawn().map_err(map_az_start_error)?;
    info!("[AUTH CLI] spawned {label} process pid={:?}", child.id());
    let deadline = Instant::now() + command_timeout;
    let status = loop {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            stop_child(&mut child, "cancel Azure CLI command").await?;
            anyhow::bail!("Azure CLI sign-in was cancelled.");
        }
        if Instant::now() >= deadline {
            stop_child(&mut child, "stop timed-out Azure CLI command").await?;
            anyhow::bail!("Azure CLI {label} timed out.");
        }
        if let Some(status) = child
            .try_wait()
            .context("Failed to inspect Azure CLI process")?
        {
            break status;
        }
        sleep(LOGIN_PROCESS_POLL_INTERVAL).await;
    };
    info!("[AUTH CLI] {label} process status={status}");
    Ok(Output {
        status,
        stdout: capture.read_stdout()?,
        stderr: capture.read_stderr()?,
    })
}

async fn stop_child(child: &mut tokio::process::Child, action: &str) -> Result<()> {
    if child
        .try_wait()
        .context("Failed to inspect Azure CLI process before stopping it")?
        .is_some()
    {
        return Ok(());
    }

    child
        .start_kill()
        .with_context(|| format!("Failed to {action}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if child
            .try_wait()
            .context("Failed to inspect Azure CLI process while stopping it")?
            .is_some()
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            anyhow::bail!("Timed out while attempting to {action}.");
        }
        sleep(Duration::from_millis(50)).await;
    }
}

struct CommandCapture {
    stdout_path: std::path::PathBuf,
    stderr_path: std::path::PathBuf,
}

impl CommandCapture {
    fn new() -> Self {
        let id = COMMAND_CAPTURE_ID.fetch_add(1, Ordering::Relaxed);
        let prefix = format!("copilot-rewrite-az-{}-{id}", std::process::id());
        let directory = std::env::temp_dir();
        Self {
            stdout_path: directory.join(format!("{prefix}.stdout")),
            stderr_path: directory.join(format!("{prefix}.stderr")),
        }
    }

    fn stdout_file(&self) -> Result<File> {
        File::create(&self.stdout_path).context("Failed to create Azure CLI stdout capture")
    }

    fn stderr_file(&self) -> Result<File> {
        File::create(&self.stderr_path).context("Failed to create Azure CLI stderr capture")
    }

    fn read_stdout(&self) -> Result<Vec<u8>> {
        std::fs::read(&self.stdout_path).context("Failed to read Azure CLI stdout")
    }

    fn read_stderr(&self) -> Result<Vec<u8>> {
        std::fs::read(&self.stderr_path).context("Failed to read Azure CLI stderr")
    }
}

impl Drop for CommandCapture {
    fn drop(&mut self) {
        remove_capture_file(&self.stdout_path);
        remove_capture_file(&self.stderr_path);
    }
}

fn remove_capture_file(path: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!(
                "Failed to remove Azure CLI command capture at {:?}: {}",
                path, error
            );
        }
    }
}

#[cfg(windows)]
fn sign_in_windows() -> Vec<isize> {
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetWindowTextW, IsWindowVisible,
    };

    unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let windows = &mut *(lparam.0 as *mut Vec<isize>);
        if !IsWindowVisible(hwnd).as_bool() {
            return BOOL(1);
        }

        let mut title = [0u16; 64];
        let title_len = GetWindowTextW(hwnd, &mut title);
        let mut class = [0u16; 64];
        let class_len = GetClassNameW(hwnd, &mut class);
        if title_len > 0
            && class_len > 0
            && String::from_utf16_lossy(&title[..title_len as usize]) == "Sign in"
            && String::from_utf16_lossy(&class[..class_len as usize]) == "ApplicationFrameWindow"
        {
            windows.push(hwnd.0 as isize);
        }
        BOOL(1)
    }

    let mut windows = Vec::new();
    let _ = unsafe {
        EnumWindows(
            Some(collect),
            LPARAM((&mut windows as *mut Vec<isize>) as isize),
        )
    };
    windows
}

#[cfg(not(windows))]
fn sign_in_windows() -> Vec<isize> {
    Vec::new()
}

#[cfg(windows)]
fn close_new_sign_in_windows(existing: &[isize]) {
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

    for window in sign_in_windows()
        .into_iter()
        .filter(|window| !existing.contains(window))
    {
        let hwnd = HWND(window as *mut _);
        match unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) } {
            Ok(()) => info!("[AUTH LOGIN] closed cancelled sign-in window"),
            Err(error) => warn!("[AUTH LOGIN] failed to close sign-in window: {error}"),
        }
    }
}

#[cfg(not(windows))]
fn close_new_sign_in_windows(_existing: &[isize]) {}

#[cfg(windows)]
fn azure_cli_python() -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .find_map(|directory| {
            if !directory.join("az.cmd").is_file() {
                return None;
            }
            let python = directory.parent()?.join("python.exe");
            python.is_file().then_some(python)
        })
}

fn map_az_start_error(error: std::io::Error) -> anyhow::Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        AzureCliNotInstalled.into()
    } else {
        anyhow::Error::new(error).context("Failed to start Azure CLI")
    }
}

fn ensure_success(output: Output, context: &str) -> Result<Output> {
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim();
    if detail.is_empty() {
        anyhow::bail!("{context} (exit code {:?}).", output.status.code());
    }
    anyhow::bail!("{context}: {detail}");
}

fn requires_login(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("az login")
        || stderr.contains("please run 'az login'")
        || stderr.contains("please run \"az login\"")
}

fn unix_timestamp() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is before the Unix epoch")?;
    i64::try_from(duration.as_secs()).context("System clock value is too large")
}

fn remove_legacy_app_credentials() {
    let Some(config_dir) = dirs::config_dir() else {
        return;
    };
    for filename in ["auth.dat", "auth.json"] {
        let path = config_dir.join("copilot-rewrite").join(filename);
        if !path.exists() {
            continue;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => info!(
                "Removed obsolete app-managed credential cache at {:?}",
                path
            ),
            Err(error) => warn!(
                "Failed to remove obsolete app-managed credential cache at {:?}: {}",
                path, error
            ),
        }
    }
}

#[derive(Debug)]
struct AzureCliNotInstalled;

impl std::fmt::Display for AzureCliNotInstalled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "Azure CLI is not installed. Install Azure CLI, restart Copilot Rewrite, and sign in.",
        )
    }
}

impl std::error::Error for AzureCliNotInstalled {}

#[cfg(test)]
mod tests {
    use super::{
        auth_status, requires_login, CachedAccessToken, UnixTimestamp, TOKEN_REFRESH_BUFFER_SECONDS,
    };

    #[test]
    fn reports_signed_in_azure_cli_user() {
        let status = auth_status(Some("user@example.com".to_string()), true);

        assert!(status.logged_in);
        assert!(status.cli_available);
        assert_eq!(status.username.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn reports_missing_azure_cli() {
        let status = auth_status(None, false);

        assert!(!status.logged_in);
        assert!(!status.cli_available);
    }

    #[test]
    fn recognizes_azure_cli_login_prompt() {
        assert!(requires_login("Please run 'az login' to setup account."));
        assert!(!requires_login("A different Azure CLI error occurred."));
    }

    #[test]
    fn reuses_token_until_refresh_buffer() {
        let now = 1_000;
        let token = CachedAccessToken {
            value: "token".to_string(),
            expires_at: now + TOKEN_REFRESH_BUFFER_SECONDS + 1,
        };

        assert!(token.is_usable(now));
        assert!(!token.is_usable(now + 1));
    }

    #[test]
    fn parses_numeric_and_string_expiration_times() {
        assert_eq!(UnixTimestamp::Number(123).into_i64().unwrap(), 123);
        assert_eq!(
            UnixTimestamp::String("456".to_string()).into_i64().unwrap(),
            456
        );
    }
}

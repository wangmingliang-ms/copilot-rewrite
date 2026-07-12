use anyhow::{Context, Result};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use std::process::Output;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const TENANT_ID: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const FOUNDRY_RESOURCE: &str = "https://ai.azure.com";

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

pub struct AzureCliAuth {
    login_cancel: Mutex<CancellationToken>,
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
            login_cancel: Mutex::new(CancellationToken::new()),
        }
    }

    pub async fn status(&self) -> AuthStatus {
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

    pub async fn login(&self) -> Result<AuthStatus> {
        self.cancel_login().await;
        let current = self.status().await;
        if current.logged_in {
            return Ok(current);
        }
        if !current.cli_available {
            return Err(AzureCliNotInstalled.into());
        }

        info!("Starting Azure CLI browser login");
        let cancel = CancellationToken::new();
        *self.login_cancel.lock().await = cancel.clone();
        let output = run_az(
            &[
                "login",
                "--tenant",
                TENANT_ID,
                "--allow-no-subscriptions",
                "--output",
                "none",
            ],
            Some(&cancel),
        )
        .await?;
        ensure_success(output, "Azure CLI login failed")?;

        let status = self.status().await;
        if !status.logged_in {
            anyhow::bail!(
                "Azure CLI login completed, but no account is active in the required tenant."
            );
        }
        Ok(status)
    }

    pub async fn access_token(&self) -> Result<String> {
        let output = run_az(
            &[
                "account",
                "get-access-token",
                "--tenant",
                TENANT_ID,
                "--resource",
                FOUNDRY_RESOURCE,
                "--query",
                "accessToken",
                "--output",
                "tsv",
            ],
            None,
        )
        .await?;
        let output = ensure_success(
            output,
            "Azure CLI could not acquire a Microsoft Foundry access token",
        )?;
        let token =
            String::from_utf8(output.stdout).context("Azure CLI returned a non-UTF-8 token")?;
        let token = token.trim();
        if token.is_empty() {
            anyhow::bail!("Azure CLI returned an empty Microsoft Foundry access token.");
        }
        Ok(token.to_string())
    }

    pub async fn cancel_login(&self) {
        self.login_cancel.lock().await.cancel();
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
    let output = run_az(
        &[
            "account",
            "show",
            "--query",
            "{tenantId:tenantId,user:user}",
            "--output",
            "json",
        ],
        None,
    )
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

async fn run_az(args: &[&str], cancel: Option<&CancellationToken>) -> Result<Output> {
    let executable = if cfg!(windows) { "az.cmd" } else { "az" };
    let mut command = Command::new(executable);
    command.args(args).kill_on_drop(true);

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.as_std_mut().creation_flags(CREATE_NO_WINDOW);
    }

    let result = if let Some(cancel) = cancel {
        let output = command.output();
        tokio::pin!(output);
        tokio::select! {
            _ = cancel.cancelled() => anyhow::bail!("Azure CLI sign-in was cancelled."),
            result = &mut output => result,
        }
    } else {
        command.output().await
    };

    match result {
        Ok(output) => Ok(output),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Err(AzureCliNotInstalled.into())
        }
        Err(error) => Err(error).context("Failed to start Azure CLI"),
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
    use super::{auth_status, requires_login};

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
}

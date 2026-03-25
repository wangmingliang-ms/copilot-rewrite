// GitHub OAuth Device Flow for Copilot authentication
// Flow: request device code → user authorizes in browser → poll for token → save

use anyhow::{Context, Result};
use log::{debug, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

// GitHub OAuth App - uses Copilot's well-known client ID
// This is the same client ID used by VS Code Copilot extension
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Response from the device code request
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Response from the token polling request
#[derive(Debug, Deserialize)]
struct TokenPollResponse {
    access_token: Option<String>,
    token_type: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Saved auth state
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedAuth {
    pub github_token: String,
    pub username: Option<String>,
}

/// Get the path to the saved auth file
fn auth_file_path() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push("copilot-rewrite");
    path.push("auth.json");
    path
}

/// Load saved auth from disk
pub fn load_saved_auth() -> Option<SavedAuth> {
    let path = auth_file_path();
    if path.exists() {
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(auth) => {
                    info!("Loaded saved auth from {:?}", path);
                    Some(auth)
                }
                Err(e) => {
                    warn!("Failed to parse auth file: {}", e);
                    None
                }
            },
            Err(e) => {
                warn!("Failed to read auth file: {}", e);
                None
            }
        }
    } else {
        None
    }
}

/// Save auth to disk
fn save_auth(auth: &SavedAuth) -> Result<()> {
    let path = auth_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create config directory")?;
    }
    let json = serde_json::to_string_pretty(auth)?;
    std::fs::write(&path, json).context("Failed to write auth file")?;
    info!("Saved auth to {:?}", path);
    Ok(())
}

/// Delete saved auth from disk
pub fn delete_saved_auth() -> Result<()> {
    let path = auth_file_path();
    if path.exists() {
        std::fs::remove_file(&path).context("Failed to delete auth file")?;
        info!("Deleted auth file at {:?}", path);
    }
    Ok(())
}

/// Step 1: Request a device code from GitHub
pub async fn request_device_code(http: &Client) -> Result<DeviceCodeResponse> {
    info!("Requesting GitHub device code...");

    let response = http
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .form(&[
            ("client_id", GITHUB_CLIENT_ID),
            ("scope", "read:user"),
        ])
        .send()
        .await
        .context("Failed to request device code")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Device code request failed (HTTP {}): {}", status, body);
    }

    let device_code: DeviceCodeResponse = response
        .json()
        .await
        .context("Failed to parse device code response")?;

    info!(
        "Got device code. User code: {}, URL: {}",
        device_code.user_code, device_code.verification_uri
    );

    Ok(device_code)
}

/// Step 2: Poll for the access token after user authorizes
pub async fn poll_for_token(
    http: &Client,
    device_code: &str,
    interval: u64,
) -> Result<String> {
    let poll_interval = Duration::from_secs(interval.max(5)); // GitHub requires minimum 5s

    loop {
        debug!("Polling for OAuth token...");

        let response = http
            .post(TOKEN_URL)
            .header("Accept", "application/json")
            .form(&[
                ("client_id", GITHUB_CLIENT_ID),
                ("device_code", device_code),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ])
            .send()
            .await
            .context("Failed to poll for token")?;

        let poll: TokenPollResponse = response
            .json()
            .await
            .context("Failed to parse token poll response")?;

        if let Some(token) = poll.access_token {
            info!("OAuth token obtained! Type: {:?}", poll.token_type);

            // Fetch username
            let username = fetch_github_username(http, &token).await.ok();

            // Save to disk
            let auth = SavedAuth {
                github_token: token.clone(),
                username,
            };
            if let Err(e) = save_auth(&auth) {
                warn!("Failed to save auth: {}", e);
            }

            return Ok(token);
        }

        match poll.error.as_deref() {
            Some("authorization_pending") => {
                debug!("Authorization pending, waiting {}s...", interval);
            }
            Some("slow_down") => {
                debug!("Rate limited, slowing down...");
                tokio::time::sleep(poll_interval + Duration::from_secs(5)).await;
                continue;
            }
            Some("expired_token") => {
                anyhow::bail!("Device code expired. Please try logging in again.");
            }
            Some("access_denied") => {
                anyhow::bail!("User denied access. Please try again and click 'Authorize'.");
            }
            Some(err) => {
                anyhow::bail!(
                    "OAuth error: {} - {}",
                    err,
                    poll.error_description.unwrap_or_default()
                );
            }
            None => {
                anyhow::bail!("Unexpected empty response from GitHub OAuth");
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Fetch the GitHub username for display
async fn fetch_github_username(http: &Client, token: &str) -> Result<String> {
    let response = http
        .get("https://api.github.com/user")
        .header("Authorization", format!("token {}", token))
        .header("User-Agent", "CopilotRewrite/0.1.0")
        .header("Accept", "application/json")
        .send()
        .await?;

    #[derive(Deserialize)]
    struct GithubUser {
        login: String,
    }

    let user: GithubUser = response.json().await?;
    Ok(user.login)
}

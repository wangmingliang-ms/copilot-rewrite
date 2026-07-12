use anyhow::{Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use log::{debug, info, warn};
use rand::{rngs::OsRng, RngCore};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

const TENANT_ID: &str = "72f988bf-86f1-41af-91ab-2d7cd011db47";
const CLIENT_ID: &str = "4e1c6d8d-5a86-4d34-a74d-b860ddbc5d69";
const FOUNDRY_SCOPES: &str = "https://ai.azure.com/.default offline_access openid profile";
const TOKEN_REFRESH_BUFFER_SECONDS: u64 = 120;
const LOGIN_TIMEOUT_SECONDS: u64 = 300;
const AUTH_FILE_MAGIC: &[u8] = b"CRAUTH1";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthorizationRequest {
    pub authorization_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AuthStatus {
    pub logged_in: bool,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub environment_override: bool,
}

#[derive(Clone, Deserialize, Serialize)]
struct SavedAuth {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    username: Option<String>,
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenEndpointResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct IdTokenClaims {
    preferred_username: Option<String>,
    upn: Option<String>,
    email: Option<String>,
    name: Option<String>,
}

struct PendingAuthorization {
    listener: TcpListener,
    redirect_uri: String,
    code_verifier: String,
    state: String,
}

pub struct MicrosoftAuth {
    http: Client,
    saved: Mutex<Option<SavedAuth>>,
    pending_authorization: Mutex<Option<PendingAuthorization>>,
    authorization_cancel: Mutex<CancellationToken>,
}

impl std::fmt::Debug for MicrosoftAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MicrosoftAuth")
            .field("tenant_id", &TENANT_ID)
            .field("client_id", &CLIENT_ID)
            .finish_non_exhaustive()
    }
}

impl MicrosoftAuth {
    pub fn new() -> Self {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build Microsoft authentication HTTP client");

        Self {
            http,
            saved: Mutex::new(load_saved_auth()),
            pending_authorization: Mutex::new(None),
            authorization_cancel: Mutex::new(CancellationToken::new()),
        }
    }

    pub async fn start_authorization_flow(&self) -> Result<AuthorizationRequest> {
        self.cancel_authorization_flow().await;
        info!("Starting Microsoft authorization code flow with PKCE");

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .context("Failed to start the local Microsoft sign-in callback")?;
        let port = listener
            .local_addr()
            .context("Failed to determine the local sign-in callback address")?
            .port();
        let redirect_uri = format!("http://localhost:{port}");
        let code_verifier = random_urlsafe(64);
        let code_challenge = pkce_challenge(&code_verifier);
        let state = random_urlsafe(32);

        let mut authorization_url =
            reqwest::Url::parse(&authorization_url()).context("Invalid Microsoft login URL")?;
        authorization_url
            .query_pairs_mut()
            .append_pair("client_id", CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_mode", "query")
            .append_pair("scope", FOUNDRY_SCOPES)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("prompt", "select_account");

        *self.authorization_cancel.lock().await = CancellationToken::new();
        *self.pending_authorization.lock().await = Some(PendingAuthorization {
            listener,
            redirect_uri,
            code_verifier,
            state,
        });

        Ok(AuthorizationRequest {
            authorization_url: authorization_url.into(),
        })
    }

    pub async fn complete_authorization_flow(&self) -> Result<AuthStatus> {
        let pending = self
            .pending_authorization
            .lock()
            .await
            .take()
            .context("No pending Microsoft sign-in. Start sign-in again.")?;
        let cancel = self.authorization_cancel.lock().await.clone();

        let authorization_code = receive_authorization_code(&pending, &cancel).await?;
        if cancel.is_cancelled() {
            anyhow::bail!("Microsoft sign-in was cancelled.");
        }
        let saved = self
            .exchange_authorization_code(
                &authorization_code,
                &pending.redirect_uri,
                &pending.code_verifier,
            )
            .await?;
        let mut saved_guard = self.saved.lock().await;
        if cancel.is_cancelled() {
            anyhow::bail!("Microsoft sign-in was cancelled.");
        }
        let status = auth_status(Some(&saved));
        save_auth(&saved)?;
        *saved_guard = Some(saved);
        Ok(status)
    }

    pub async fn status(&self) -> AuthStatus {
        auth_status(self.saved.lock().await.as_ref())
    }

    pub async fn access_token(&self) -> Result<String> {
        let mut saved_guard = self.saved.lock().await;
        let saved = saved_guard
            .as_ref()
            .context("Sign in with Microsoft in Settings to use Foundry Agents.")?;

        if token_is_valid(&saved, unix_timestamp()) {
            return Ok(saved.access_token.clone());
        }

        match self.refresh_access_token(saved).await {
            Ok(refreshed) => {
                let token = refreshed.access_token.clone();
                save_auth(&refreshed)?;
                *saved_guard = Some(refreshed);
                Ok(token)
            }
            Err(error) if error.downcast_ref::<ReauthenticationRequired>().is_some() => {
                *saved_guard = None;
                delete_saved_auth().context(
                    "Microsoft session expired and the local credential cache could not be cleared",
                )?;
                Err(error).context("Microsoft session expired. Sign in again in Settings.")
            }
            Err(error) => Err(error),
        }
    }

    pub async fn cancel_authorization_flow(&self) {
        self.authorization_cancel.lock().await.cancel();
        *self.pending_authorization.lock().await = None;
    }

    pub async fn logout(&self) -> Result<()> {
        self.cancel_authorization_flow().await;
        *self.saved.lock().await = None;
        delete_saved_auth()
    }

    async fn exchange_authorization_code(
        &self,
        authorization_code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<SavedAuth> {
        debug!("Exchanging Microsoft authorization code");
        let response = self
            .http
            .post(token_url())
            .form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "authorization_code"),
                ("code", authorization_code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", code_verifier),
                ("scope", FOUNDRY_SCOPES),
            ])
            .send()
            .await
            .context("Failed to exchange the Microsoft authorization code")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Microsoft token response")?;
        let token: TokenEndpointResponse =
            serde_json::from_str(&body).context("Failed to parse Microsoft token response")?;

        let access_token = token.access_token.context(format!(
            "Microsoft sign-in failed: {} - {}",
            token
                .error
                .unwrap_or_else(|| format!("HTTP {}", status.as_u16())),
            token.error_description.unwrap_or_default()
        ))?;
        let refresh_token = token
            .refresh_token
            .context("Microsoft did not return a refresh token")?;
        let claims = token
            .id_token
            .as_deref()
            .and_then(parse_id_token_claims)
            .unwrap_or_default();

        info!(
            "Microsoft sign-in completed for {:?}",
            claims.preferred_username.as_ref().or(claims.upn.as_ref())
        );
        Ok(SavedAuth {
            access_token,
            refresh_token,
            expires_at: unix_timestamp().saturating_add(token.expires_in.unwrap_or(3600)),
            username: claims.preferred_username.or(claims.upn).or(claims.email),
            display_name: claims.name,
        })
    }

    async fn refresh_access_token(&self, saved: &SavedAuth) -> Result<SavedAuth> {
        info!("Refreshing Microsoft Foundry access token");

        let response = self
            .http
            .post(token_url())
            .form(&[
                ("client_id", CLIENT_ID),
                ("grant_type", "refresh_token"),
                ("refresh_token", saved.refresh_token.as_str()),
                ("scope", FOUNDRY_SCOPES),
            ])
            .send()
            .await
            .context("Failed to refresh Microsoft access token")?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read Microsoft token refresh response")?;
        let token: TokenEndpointResponse = serde_json::from_str(&body)
            .context("Failed to parse Microsoft token refresh response")?;

        let error_code = token.error.as_deref();
        let error_detail = token
            .error_description
            .clone()
            .or_else(|| token.error.clone())
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        let access_token = match token.access_token {
            Some(access_token) => access_token,
            None if matches!(
                error_code,
                Some(
                    "invalid_grant"
                        | "interaction_required"
                        | "invalid_client"
                        | "unauthorized_client"
                )
            ) =>
            {
                return Err(ReauthenticationRequired(error_detail).into());
            }
            None => anyhow::bail!("Microsoft session refresh failed: {error_detail}"),
        };

        Ok(SavedAuth {
            access_token,
            refresh_token: token
                .refresh_token
                .unwrap_or_else(|| saved.refresh_token.clone()),
            expires_at: unix_timestamp().saturating_add(token.expires_in.unwrap_or(3600)),
            username: saved.username.clone(),
            display_name: saved.display_name.clone(),
        })
    }
}

#[derive(Debug)]
struct ReauthenticationRequired(String);

impl std::fmt::Display for ReauthenticationRequired {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ReauthenticationRequired {}

impl Default for MicrosoftAuth {
    fn default() -> Self {
        Self::new()
    }
}

fn auth_status(saved: Option<&SavedAuth>) -> AuthStatus {
    AuthStatus {
        logged_in: saved.is_some_and(|auth| !auth.refresh_token.is_empty()),
        username: saved.and_then(|auth| auth.username.clone()),
        display_name: saved.and_then(|auth| auth.display_name.clone()),
        environment_override: false,
    }
}

fn token_is_valid(saved: &SavedAuth, now: u64) -> bool {
    !saved.access_token.is_empty()
        && saved.expires_at > now.saturating_add(TOKEN_REFRESH_BUFFER_SECONDS)
}

async fn receive_authorization_code(
    pending: &PendingAuthorization,
    cancel: &CancellationToken,
) -> Result<String> {
    let (mut stream, _) = tokio::select! {
        _ = cancel.cancelled() => anyhow::bail!("Microsoft sign-in was cancelled."),
        result = tokio::time::timeout(
            Duration::from_secs(LOGIN_TIMEOUT_SECONDS),
            pending.listener.accept(),
        ) => result
            .context("Microsoft sign-in timed out. Please try again.")?
            .context("Failed to receive the Microsoft sign-in callback")?,
    };

    let mut request = Vec::with_capacity(2048);
    loop {
        let mut buffer = [0_u8; 1024];
        let count = stream
            .read(&mut buffer)
            .await
            .context("Failed to read the Microsoft sign-in callback")?;
        if count == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..count]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if request.len() > 16 * 1024 {
            anyhow::bail!("Microsoft sign-in callback was unexpectedly large.");
        }
    }

    let request = std::str::from_utf8(&request)
        .context("Microsoft sign-in callback contained invalid text")?;
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Microsoft sign-in callback was malformed")?;
    let callback_url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .context("Microsoft sign-in callback URL was invalid")?;

    let error = callback_url
        .query_pairs()
        .find(|(key, _)| key == "error")
        .map(|(_, value)| value.into_owned());
    let error_description = callback_url
        .query_pairs()
        .find(|(key, _)| key == "error_description")
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default();
    if let Some(error) = error {
        write_callback_response(&mut stream, false).await;
        anyhow::bail!("Microsoft sign-in failed: {error} - {error_description}");
    }

    let returned_state = callback_url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .map(|(_, value)| value.into_owned())
        .context("Microsoft sign-in callback did not include state")?;
    if returned_state != pending.state {
        write_callback_response(&mut stream, false).await;
        anyhow::bail!("Microsoft sign-in callback failed state validation.");
    }

    let authorization_code = callback_url
        .query_pairs()
        .find(|(key, _)| key == "code")
        .map(|(_, value)| value.into_owned())
        .context("Microsoft sign-in callback did not include an authorization code")?;
    write_callback_response(&mut stream, true).await;
    Ok(authorization_code)
}

async fn write_callback_response(stream: &mut tokio::net::TcpStream, success: bool) {
    let (title, message) = if success {
        (
            "Copilot Rewrite sign-in complete",
            "You can close this window and return to Copilot Rewrite.",
        )
    } else {
        (
            "Copilot Rewrite sign-in failed",
            "Return to Copilot Rewrite to see the error details.",
        )
    };
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title></head>\
         <body style=\"font-family:Segoe UI,sans-serif;padding:48px\"><h1>{title}</h1>\
         <p>{message}</p></body></html>"
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

fn random_urlsafe(byte_count: usize) -> String {
    let mut bytes = vec![0_u8; byte_count];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

fn authorization_url() -> String {
    format!("https://login.microsoftonline.com/{TENANT_ID}/oauth2/v2.0/authorize")
}

fn token_url() -> String {
    format!("https://login.microsoftonline.com/{TENANT_ID}/oauth2/v2.0/token")
}

fn parse_id_token_claims(id_token: &str) -> Option<IdTokenClaims> {
    let payload = id_token.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&decoded).ok()
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn auth_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("copilot-rewrite")
        .join("auth.dat")
}

fn legacy_auth_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("copilot-rewrite")
        .join("auth.json")
}

fn load_saved_auth() -> Option<SavedAuth> {
    let path = auth_file_path();
    if path.exists() {
        return match std::fs::read(&path)
            .context("Failed to read encrypted auth file")
            .and_then(|content| {
                let encrypted = content
                    .strip_prefix(AUTH_FILE_MAGIC)
                    .context("Encrypted auth file has an invalid header")?;
                let json = unprotect_auth_data(encrypted)?;
                serde_json::from_slice(&json).context("Failed to parse decrypted auth file")
            }) {
            Ok(auth) => {
                info!("Loaded DPAPI-protected Microsoft auth from {:?}", path);
                Some(auth)
            }
            Err(error) => {
                warn!("Failed to load encrypted Microsoft auth: {error:#}");
                None
            }
        };
    }

    let legacy_path = legacy_auth_file_path();
    if !legacy_path.exists() {
        return None;
    }

    match std::fs::read_to_string(&legacy_path)
        .context("Failed to read legacy auth file")
        .and_then(|content| serde_json::from_str(&content).context("Failed to parse auth file"))
    {
        Ok(auth) => match save_auth(&auth) {
            Ok(()) => {
                info!(
                    "Migrated legacy Microsoft auth to DPAPI storage from {:?}",
                    legacy_path
                );
                Some(auth)
            }
            Err(error) => {
                warn!("Failed to migrate legacy Microsoft auth: {error:#}");
                None
            }
        },
        Err(error) => {
            warn!("Ignoring incompatible saved auth: {error:#}");
            match std::fs::remove_file(&legacy_path) {
                Ok(()) => info!("Deleted incompatible legacy auth at {:?}", legacy_path),
                Err(delete_error) => warn!(
                    "Failed to delete incompatible legacy auth at {:?}: {}",
                    legacy_path, delete_error
                ),
            }
            None
        }
    }
}

fn save_auth(auth: &SavedAuth) -> Result<()> {
    let path = auth_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create auth directory")?;
    }
    let json = serde_json::to_string_pretty(auth)?;
    let encrypted = protect_auth_data(json.as_bytes())?;
    let mut content = Vec::with_capacity(AUTH_FILE_MAGIC.len() + encrypted.len());
    content.extend_from_slice(AUTH_FILE_MAGIC);
    content.extend_from_slice(&encrypted);
    std::fs::write(&path, content).context("Failed to save encrypted Microsoft auth")?;

    let legacy_path = legacy_auth_file_path();
    if legacy_path.exists() {
        std::fs::remove_file(&legacy_path).context("Failed to delete legacy auth file")?;
    }

    info!("Saved DPAPI-protected Microsoft auth to {:?}", path);
    Ok(())
}

fn delete_saved_auth() -> Result<()> {
    for path in [auth_file_path(), legacy_auth_file_path()] {
        if path.exists() {
            std::fs::remove_file(&path).context("Failed to delete Microsoft auth")?;
            info!("Deleted Microsoft auth at {:?}", path);
        }
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn protect_auth_data(plaintext: &[u8]) -> Result<Vec<u8>> {
    use windows::core::w;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext
            .len()
            .try_into()
            .context("Auth data is too large to encrypt")?,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptProtectData(
            &input,
            w!("Copilot Rewrite Foundry authentication"),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context("Windows DPAPI failed to encrypt Microsoft auth")?;
        let protected = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(protected)
    }
}

#[cfg(target_os = "windows")]
fn unprotect_auth_data(ciphertext: &[u8]) -> Result<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: ciphertext
            .len()
            .try_into()
            .context("Encrypted auth data is too large")?,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .context("Windows DPAPI failed to decrypt Microsoft auth")?;
        let plaintext = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(output.pbData.cast()));
        Ok(plaintext)
    }
}

#[cfg(not(target_os = "windows"))]
fn protect_auth_data(_plaintext: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!("Microsoft credential storage is supported only on Windows")
}

#[cfg(not(target_os = "windows"))]
fn unprotect_auth_data(_ciphertext: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!("Microsoft credential storage is supported only on Windows")
}

#[cfg(test)]
mod tests {
    use super::{
        auth_status, parse_id_token_claims, pkce_challenge, protect_auth_data, token_is_valid,
        unprotect_auth_data, AuthStatus, SavedAuth, TOKEN_REFRESH_BUFFER_SECONDS,
    };
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};

    fn saved_auth(expires_at: u64) -> SavedAuth {
        SavedAuth {
            access_token: "access-token".to_string(),
            refresh_token: "refresh-token".to_string(),
            expires_at,
            username: Some("user@example.com".to_string()),
            display_name: Some("Example User".to_string()),
        }
    }

    #[test]
    fn accepts_tokens_outside_refresh_buffer() {
        assert!(token_is_valid(
            &saved_auth(1000 + TOKEN_REFRESH_BUFFER_SECONDS + 1),
            1000
        ));
        assert!(!token_is_valid(
            &saved_auth(1000 + TOKEN_REFRESH_BUFFER_SECONDS),
            1000
        ));
    }

    #[test]
    fn reports_cached_account_status_without_exposing_tokens() {
        assert_eq!(
            serde_json::to_value(auth_status(Some(&saved_auth(2000)))).unwrap(),
            serde_json::json!({
                "logged_in": true,
                "username": "user@example.com",
                "display_name": "Example User"
                ,"environment_override": false
            })
        );
        assert_eq!(
            serde_json::to_value(AuthStatus {
                logged_in: false,
                username: None,
                display_name: None,
                environment_override: false,
            })
            .unwrap(),
            serde_json::json!({
                "logged_in": false,
                "username": null,
                "display_name": null
                ,"environment_override": false
            })
        );
    }

    #[test]
    fn reads_display_identity_from_id_token() {
        let payload = URL_SAFE_NO_PAD
            .encode(br#"{"preferred_username":"user@example.com","name":"Example User"}"#);
        let token = format!("header.{payload}.signature");
        let claims = parse_id_token_claims(&token).unwrap();

        assert_eq!(
            claims.preferred_username.as_deref(),
            Some("user@example.com")
        );
        assert_eq!(claims.name.as_deref(), Some("Example User"));
    }

    #[test]
    fn creates_rfc_7636_pkce_challenge() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn protects_auth_data_for_current_windows_user() {
        let plaintext = b"refresh-token";
        let protected = protect_auth_data(plaintext).unwrap();

        assert_ne!(protected, plaintext);
        assert_eq!(unprotect_auth_data(&protected).unwrap(), plaintext);
    }
}

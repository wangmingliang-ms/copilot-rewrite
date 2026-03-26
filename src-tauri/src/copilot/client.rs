// Copilot API client implementation
// Uses GitHub Copilot token exchange flow:
// 1. Use GitHub token to get Copilot session token from api.github.com
// 2. Use session token to call chat completions API

use crate::RewriteAction;
use anyhow::{Context, Result};
use log::{debug, info, warn};
use parking_lot::Mutex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_CHAT_URL: &str = "https://api.githubcopilot.com/chat/completions";
const COPILOT_MODELS_URL: &str = "https://api.githubcopilot.com/models";

/// Cached Copilot session token
#[derive(Debug)]
struct CachedToken {
    token: String,
    expires_at: Instant,
}

/// System prompt for translation mode
fn translate_system_prompt(target_language: &str) -> String {
    format!(
        r#"You are a professional translator. Translate the given text into {target_language}.

Rules:
- Auto-detect the source language
- Preserve the original meaning
- You may freely reorder sentences, adjust wording, and restructure paragraphs to make the translation clear, logical, and natural in {target_language}
- If the text is already in {target_language}, just polish it for clarity
- Do NOT add explanations, notes, or any extra text
- Return ONLY the translated text"#,
    )
}

/// System prompt for polishing mode
const POLISH_SYSTEM_PROMPT: &str = r#"You are a professional writing assistant. Polish and improve the given text.

Rules:
- Reorganize the text to be logical, well-structured, and easy to understand
- The user's input may be casual, disorganized, or lack structure — you should freely reorder sentences, adjust wording, and restructure paragraphs
- Fix grammar, spelling, and punctuation errors
- Keep the same language as the input
- Preserve the original meaning (the ideas must stay the same, but expression can change freely)
- Do NOT add explanations, notes, or any extra text
- Return ONLY the polished text"#;

/// System prompt for translate + polish mode (default action)
fn translate_and_polish_system_prompt(target_language: &str) -> String {
    format!(
        r#"You are a professional writing assistant and translator.

Your task has two steps:

Step 1 — REORGANIZE: The user's input may be casual, disorganized, rambling, or poorly structured. Reorganize the content IN THE ORIGINAL LANGUAGE so it becomes logical, well-structured, and coherent. You may freely reorder sentences, merge or split ideas, adjust wording, and restructure paragraphs. The meaning must stay the same, but the expression can change as needed. Use Markdown formatting when it improves readability (bullet lists, numbered lists, **bold** for emphasis, etc.).

Step 2 — TRANSLATE: Translate the reorganized content into clear, natural, and idiomatic {target_language}. Avoid colloquial expressions and slang. The result should read as if originally written by a native {target_language} speaker in a professional context. Use the same Markdown formatting as Step 1.

You MUST respond with a JSON object containing exactly two fields:
{{"reorganized": "the polished text in the original language", "translated": "the translation in {target_language}"}}

Rules:
- Auto-detect the source language
- Both outputs must be accurate, well-organized, logical, and easy to understand
- Respond with ONLY the JSON object, no markdown code fences, no explanation, no other text
- Use \n for newlines within the JSON string values"#,
    )
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    #[serde(default)]
    message: Option<ResponseMessage>,
    #[serde(default)]
    delta: Option<DeltaMessage>,
}

#[derive(Debug, Deserialize)]
struct ResponseMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct DeltaMessage {
    #[serde(default)]
    content: Option<String>,
}

/// Response from the Copilot token endpoint
#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: i64,
}

/// A model available from the Copilot API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub owned_by: String,
}

/// Response from /models endpoint
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelEntry>,
}

#[derive(Debug, Deserialize)]
struct ModelEntry {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    owned_by: Option<String>,
}

/// Client for the GitHub Copilot API
#[derive(Debug)]
pub struct CopilotClient {
    http: Client,
    cached_token: Mutex<Option<CachedToken>>,
}

impl CopilotClient {
    pub fn new() -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            http,
            cached_token: Mutex::new(None),
        }
    }

    /// Get a valid Copilot session token, refreshing if needed
    async fn get_copilot_token(&self, github_token: &str) -> Result<String> {
        // Check cache first
        {
            let cache = self.cached_token.lock();
            if let Some(ref cached) = *cache {
                // Use cached token if it has at least 60 seconds of life left
                if cached.expires_at > Instant::now() + Duration::from_secs(60) {
                    debug!("Using cached Copilot token");
                    return Ok(cached.token.clone());
                }
            }
        }

        // Need to fetch a new token
        info!("Fetching new Copilot session token...");

        let response = self
            .http
            .get(COPILOT_TOKEN_URL)
            .header("Authorization", format!("token {}", github_token))
            .header("User-Agent", "CopilotRewrite/0.1.0")
            .header("Accept", "application/json")
            .send()
            .await
            .context("Failed to request Copilot token")?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            anyhow::bail!(
                "Failed to get Copilot token (HTTP {}): {}. Make sure you have an active GitHub Copilot subscription.",
                status.as_u16(),
                error_body
            );
        }

        let token_response: CopilotTokenResponse = response
            .json()
            .await
            .context("Failed to parse Copilot token response")?;

        // Cache the token
        // expires_at is Unix timestamp - convert to Instant
        let now_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let expires_in_secs = (token_response.expires_at - now_unix).max(0) as u64;

        let cached = CachedToken {
            token: token_response.token.clone(),
            expires_at: Instant::now() + Duration::from_secs(expires_in_secs),
        };

        info!("Got Copilot token, expires in {}s", expires_in_secs);

        *self.cached_token.lock() = Some(cached);

        Ok(token_response.token)
    }

    /// Process text using the Copilot API
    pub async fn process(
        &self,
        text: &str,
        action: &RewriteAction,
        target_language: &str,
        github_token: &str,
        model: &str,
    ) -> Result<String> {
        if github_token.is_empty() {
            anyhow::bail!("GitHub token is not configured. Please set your GitHub token (with Copilot access) in Settings.");
        }

        // Step 1: Get Copilot session token
        let copilot_token = self.get_copilot_token(github_token).await?;

        // Step 2: Build the request
        let system_prompt = match action {
            RewriteAction::Translate => translate_system_prompt(target_language),
            RewriteAction::Polish => POLISH_SYSTEM_PROMPT.to_string(),
            RewriteAction::TranslateAndPolish => {
                translate_and_polish_system_prompt(target_language)
            }
        };

        info!(
            "Processing text ({} chars) with action {:?}",
            text.len(),
            action
        );

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: text.to_string(),
                },
            ],
            temperature: 0.3,
            stream: true,
        };

        // Step 3: Call Copilot chat completions API with session token
        debug!("Sending request to Copilot API...");

        let response = self
            .http
            .post(COPILOT_CHAT_URL)
            .header("Authorization", format!("Bearer {}", copilot_token))
            .header("Content-Type", "application/json")
            .header("Editor-Version", "vscode/1.96.0")
            .header("Editor-Plugin-Version", "copilot-chat/0.24")
            .header("Copilot-Integration-Id", "vscode-chat")
            .header("Openai-Intent", "conversation-panel")
            .header("User-Agent", "CopilotRewrite/0.1.0")
            .json(&request)
            .send()
            .await
            .context("Failed to send request to Copilot API")?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();

            // If token expired, clear cache and retry once
            if status.as_u16() == 401 {
                warn!("Copilot token expired, clearing cache...");
                *self.cached_token.lock() = None;
            }

            anyhow::bail!(
                "Copilot API returned HTTP {}: {}",
                status.as_u16(),
                error_body
            );
        }

        // Parse SSE stream response
        let body = response.text().await.context("Failed to read response body")?;
        let mut result = String::new();
        
        for line in body.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<ChatCompletionResponse>(data) {
                    if let Some(choice) = chunk.choices.first() {
                        if let Some(ref delta) = choice.delta {
                            if let Some(ref content) = delta.content {
                                result.push_str(content);
                            }
                        }
                        // Also handle non-streaming format just in case
                        if let Some(ref message) = choice.message {
                            result.push_str(&message.content);
                        }
                    }
                }
            }
        }

        info!("Copilot API returned {} chars", result.len());

        Ok(result.trim().to_string())
    }
}

impl Default for CopilotClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CopilotClient {
    /// List available models from the Copilot API
    pub async fn list_models(&self, github_token: &str) -> Result<Vec<CopilotModel>> {
        if github_token.is_empty() {
            anyhow::bail!("Not logged in");
        }

        let copilot_token = self.get_copilot_token(github_token).await?;

        let response = self
            .http
            .get(COPILOT_MODELS_URL)
            .header("Authorization", format!("Bearer {}", copilot_token))
            .header("User-Agent", "CopilotRewrite/0.1.0")
            .header("Accept", "application/json")
            .header("Editor-Version", "vscode/1.96.0")
            .header("Editor-Plugin-Version", "copilot-chat/0.24")
            .header("Copilot-Integration-Id", "vscode-chat")
            .header("Openai-Intent", "model-access")
            .send()
            .await
            .context("Failed to fetch models")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!("Models endpoint returned {}: {}", status, body);
            // Return default models as fallback
            return Ok(Self::default_models());
        }

        let models_resp: ModelsResponse = match response.json().await {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to parse models response: {}", e);
                return Ok(Self::default_models());
            }
        };

        let models: Vec<CopilotModel> = models_resp
            .data
            .into_iter()
            .map(|m| CopilotModel {
                id: m.id.clone(),
                name: m.name.unwrap_or_else(|| m.id.clone()),
                version: m.version.unwrap_or_default(),
                owned_by: m.owned_by.unwrap_or_default(),
            })
            .collect();

        if models.is_empty() {
            Ok(Self::default_models())
        } else {
            info!("Fetched {} models from Copilot API", models.len());
            Ok(models)
        }
    }

    /// Fallback model list when API is unavailable
    fn default_models() -> Vec<CopilotModel> {
        vec![
            CopilotModel { id: "claude-sonnet-4".into(), name: "Claude Sonnet 4".into(), version: String::new(), owned_by: "Anthropic".into() },
            CopilotModel { id: "gpt-4o".into(), name: "GPT-4o".into(), version: String::new(), owned_by: "OpenAI".into() },
            CopilotModel { id: "gpt-4o-mini".into(), name: "GPT-4o Mini".into(), version: String::new(), owned_by: "OpenAI".into() },
            CopilotModel { id: "claude-sonnet-4.5".into(), name: "Claude Sonnet 4.5".into(), version: String::new(), owned_by: "Anthropic".into() },
            CopilotModel { id: "claude-haiku-4.5".into(), name: "Claude Haiku 4.5".into(), version: String::new(), owned_by: "Anthropic".into() },
            CopilotModel { id: "gpt-5-mini".into(), name: "GPT-5 Mini".into(), version: String::new(), owned_by: "Azure OpenAI".into() },
            CopilotModel { id: "gemini-2.5-pro".into(), name: "Gemini 2.5 Pro".into(), version: String::new(), owned_by: "Google".into() },
        ]
    }
}

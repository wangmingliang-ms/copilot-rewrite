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
- The output MUST sound like it was originally written by a native {target_language} speaker — eliminate all "translationese" (awkward literal phrasing, unnatural word order, sentence patterns borrowed from the source language)
- If the text is already in {target_language}, just polish it for clarity
- Do NOT add explanations, notes, or any extra text
- Return ONLY the translated text"#,
    )
}

/// System prompt for polishing mode
const POLISH_SYSTEM_PROMPT: &str = r#"You are a professional writing assistant. Polish and improve the given text.

Rules:
- Reorganize the text to be logical, well-structured, and easy to understand
- Prefer using Markdown lists (bullet points or numbered lists) to organize multiple points, steps, or ideas — lists are clearer and more scannable than long paragraphs
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

Follow this chain of thought:

Step 1 — UNDERSTAND INTENT: Read the user's text carefully. Identify the core message, key points, and the intent behind what they are trying to communicate.

Step 2 — REORGANIZE IN ORIGINAL LANGUAGE: Rewrite the content in the original language to be logical, well-structured, and coherent. You may freely reorder sentences, merge or split ideas, adjust wording, and restructure paragraphs. The meaning must stay the same, but the expression should be clear and polished. Prefer using Markdown lists (bullet points or numbered lists) to organize multiple points, steps, or ideas — lists are clearer and more scannable than long paragraphs. Use other Markdown formatting (**bold** for emphasis, headings, etc.) when it improves readability.

Step 3 — THINK IN {target_language}: Before translating word by word, re-think the content using {target_language} thought patterns and conventions. Different languages organize ideas differently — {target_language} may prefer different sentence structures, emphasis patterns, or logical flows. Restructure the content to feel natural in {target_language} thinking.

Step 4 — OUTPUT IN {target_language}: Write the final version in clear, natural, and idiomatic {target_language}. The result MUST read as if originally written by a native {target_language} speaker — NOT as a translation. Eliminate all signs of "translationese": avoid awkward literal phrasing, unnatural word order, or sentence patterns borrowed from the source language. Every sentence should sound like something a native speaker would actually write. Use the same Markdown formatting as Step 2.

You MUST respond with a JSON object containing exactly two fields:
{{"reorganized": "the polished text in the original language (Step 2)", "translated": "the {target_language} output (Step 4)"}}

Rules:
- Auto-detect the source language
- Both outputs must be accurate, well-organized, logical, and easy to understand
- Respond with ONLY the JSON object, no markdown code fences, no explanation, no other text
- Use \n for newlines within the JSON string values"#,
    )
}

// =====================================================================
// Beast Mode Prompts — LLM has full creative freedom to rewrite
// =====================================================================

fn beast_translate_system_prompt(target_language: &str) -> String {
    format!(
        r#"You are a world-class writer and translator with FULL CREATIVE FREEDOM.

Follow this chain of thought:

Step 1 — UNDERSTAND INTENT: Read the user's text deeply. Look beyond the surface words — understand what they truly want to communicate, their underlying purpose, and the effect they want to achieve.

Step 2 — REWRITE FROM SCRATCH: Using the original language, rewrite the content as if you were the one writing it from scratch. You may freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible. Prefer using Markdown lists (bullet points or numbered lists) to organize multiple points, steps, or ideas.

Step 3 — THINK IN {target_language}: Before producing the final output, re-think the entire content using {target_language} thought patterns. Different languages have fundamentally different ways of organizing arguments, building emphasis, and flowing logically. Restructure the content to feel native in {target_language} thinking.

Step 4 — OUTPUT IN {target_language}: Write the final version as if you were the best native {target_language} writer crafting this from scratch. It MUST NOT read like a translation at all — zero translationese, zero borrowed sentence patterns from the source language. Every phrase should sound completely natural to a native {target_language} reader.

CRITICAL RULES:
- Your freedom is in HOW to express ideas, NOT in WHAT to express. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add your own opinions.
- If the text contains questions, rewrite them as better-phrased questions — do NOT answer them.
- If the text describes a problem, rewrite the description more clearly — do NOT solve the problem.
- Do NOT add explanations, notes, or meta-commentary. Return ONLY the rewritten text in {target_language}."#,
    )
}

const BEAST_POLISH_SYSTEM_PROMPT: &str = r#"You are a world-class writer with FULL CREATIVE FREEDOM.

Follow this chain of thought:

Step 1 — UNDERSTAND INTENT: Read the user's text deeply. Look beyond the surface words — understand what they truly want to communicate, their underlying purpose, and the effect they want to achieve.

Step 2 — REWRITE FROM SCRATCH: In the same language, rewrite the content as if you were the one writing it from scratch. You may freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible. Prefer using Markdown lists (bullet points or numbered lists) to organize multiple points, steps, or ideas — lists are clearer and more scannable than long paragraphs. Use other Markdown formatting freely when it helps.

CRITICAL RULES:
- Your freedom is in HOW to express ideas, NOT in WHAT to express. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add your own opinions.
- If the text contains questions, rewrite them as better-phrased questions — do NOT answer them.
- If the text describes a problem, rewrite the description more clearly — do NOT solve the problem.
- Do NOT add explanations, notes, or meta-commentary. Return ONLY the rewritten text."#;

fn beast_translate_and_polish_system_prompt(target_language: &str) -> String {
    format!(
        r#"You are a world-class writer and translator with FULL CREATIVE FREEDOM.

Follow this chain of thought:

Step 1 — UNDERSTAND INTENT: Read the user's text deeply. Look beyond the surface words — understand what they truly want to communicate, their underlying purpose, and the effect they want to achieve.

Step 2 — REWRITE IN ORIGINAL LANGUAGE: Using the original language, rewrite the content as if you were the one writing it from scratch. You may freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible. Prefer using Markdown lists (bullet points or numbered lists) to organize multiple points, steps, or ideas — lists are clearer and more scannable than long paragraphs. Use other Markdown formatting freely when it helps.

Step 3 — THINK IN {target_language}: Before translating, re-think the entire content using {target_language} thought patterns. Different languages have fundamentally different ways of organizing arguments, building emphasis, and flowing logically. Restructure the content to feel native in {target_language} thinking — not just a word-for-word conversion.

Step 4 — OUTPUT IN {target_language}: Write the final version as if you were the best native {target_language} writer crafting this from scratch. It MUST NOT read like a translation at all — zero translationese, zero borrowed sentence patterns from the source language. Every phrase should sound completely natural to a native {target_language} reader. Use the same Markdown formatting as Step 2.

You MUST respond with a JSON object containing exactly two fields:
{{"reorganized": "your rewrite in the original language (Step 2)", "translated": "your {target_language} output (Step 4)"}}

CRITICAL RULES:
- Your freedom is in HOW to express ideas, NOT in WHAT to express. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add your own opinions.
- If the text contains questions, rewrite them as better-phrased questions — do NOT answer them.
- If the text describes a problem, rewrite the description more clearly — do NOT solve the problem.
- Auto-detect the source language
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
    #[serde(default)]
    model: Option<String>,
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
    pub vendor: String,
    #[serde(default)]
    pub preview: bool,
    #[serde(default)]
    pub category: String,
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
    vendor: Option<String>,
    #[serde(default)]
    model_picker_enabled: Option<bool>,
    #[serde(default)]
    preview: Option<bool>,
    #[serde(default)]
    capabilities: Option<ModelCapabilities>,
    #[serde(default)]
    supported_endpoints: Option<Vec<String>>,
    #[serde(default)]
    model_picker_category: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelCapabilities {
    #[serde(default, rename = "type")]
    model_type: Option<String>,
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
            .header("User-Agent", "CopilotRewrite/0.2.0")
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
        beast_mode: bool,
        app_context: &str,
    ) -> Result<String> {
        if github_token.is_empty() {
            anyhow::bail!("GitHub token is not configured. Please set your GitHub token (with Copilot access) in Settings.");
        }

        // Step 1: Get Copilot session token
        let copilot_token = self.get_copilot_token(github_token).await?;

        // Step 2: Build the request
        let system_prompt = if beast_mode {
            match action {
                RewriteAction::Translate => beast_translate_system_prompt(target_language),
                RewriteAction::Polish => BEAST_POLISH_SYSTEM_PROMPT.to_string(),
                RewriteAction::TranslateAndPolish => {
                    beast_translate_and_polish_system_prompt(target_language)
                }
            }
        } else {
            match action {
                RewriteAction::Translate => translate_system_prompt(target_language),
                RewriteAction::Polish => POLISH_SYSTEM_PROMPT.to_string(),
                RewriteAction::TranslateAndPolish => {
                    translate_and_polish_system_prompt(target_language)
                }
            }
        };

        // Append app context to system prompt if available
        let system_prompt = if !app_context.is_empty() {
            format!(
                "{}\n\nCONTEXT: The user is writing in: {}. Adapt the tone and formality level to match this context (e.g., casual for chat apps like Teams/Slack, professional for email apps like Outlook, technical for developer tools like GitHub/VS Code).",
                system_prompt,
                app_context
            )
        } else {
            system_prompt
        };

        info!(
            "Processing text ({} chars) with action {:?}, model: {}, beast_mode: {}, context: {}",
            text.len(),
            action,
            model,
            beast_mode,
            if app_context.is_empty() { "none" } else { app_context }
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
            .header("User-Agent", "CopilotRewrite/0.2.0")
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
        let mut actual_model_logged = false;
        
        for line in body.lines() {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    break;
                }
                if let Ok(chunk) = serde_json::from_str::<ChatCompletionResponse>(data) {
                    // Log the actual model used by the API (from first chunk)
                    if !actual_model_logged {
                        info!(
                            "API responding with model: {} (requested: {})",
                            chunk.model.as_deref().unwrap_or("unknown"),
                            model
                        );
                        actual_model_logged = true;
                    }
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
            .header("User-Agent", "CopilotRewrite/0.2.0")
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

        let body_text = response.text().await.context("Failed to read models body")?;
        let models_resp: ModelsResponse = match serde_json::from_str(&body_text) {
            Ok(m) => m,
            Err(e) => {
                warn!("Failed to parse models response: {}", e);
                return Ok(Self::default_models());
            }
        };

        let models: Vec<CopilotModel> = models_resp
            .data
            .into_iter()
            // Filter: only model_picker_enabled=true and chat-capable models
            .filter(|m| {
                // Use official model_picker_enabled flag — false means deprecated/hidden
                if !m.model_picker_enabled.unwrap_or(false) {
                    return false;
                }
                // Skip non-chat models (e.g. embeddings)
                if let Some(ref caps) = m.capabilities {
                    if let Some(ref t) = caps.model_type {
                        if t != "chat" { return false; }
                    }
                }
                // Skip models that don't support /chat/completions endpoint
                if let Some(ref endpoints) = m.supported_endpoints {
                    if !endpoints.iter().any(|e| e == "/chat/completions") {
                        debug!("Skipping model {} — no /chat/completions support (endpoints: {:?})", m.id, endpoints);
                        return false;
                    }
                }
                // Skip internal-only models
                let name = m.name.as_deref().unwrap_or("");
                if name.to_lowercase().contains("internal") { return false; }
                true
            })
            .map(|m| CopilotModel {
                id: m.id.clone(),
                name: m.name.unwrap_or_else(|| m.id.clone()),
                version: m.version.unwrap_or_default(),
                vendor: m.vendor.unwrap_or_default(),
                preview: m.preview.unwrap_or(false),
                category: m.model_picker_category.unwrap_or_default(),
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
            CopilotModel { id: "claude-sonnet-4".into(), name: "Claude Sonnet 4".into(), version: String::new(), vendor: "Anthropic".into(), preview: false, category: "versatile".into() },
            CopilotModel { id: "gpt-4o".into(), name: "GPT-4o".into(), version: String::new(), vendor: "OpenAI".into(), preview: false, category: "versatile".into() },
            CopilotModel { id: "gpt-5-mini".into(), name: "GPT-5 Mini".into(), version: String::new(), vendor: "OpenAI".into(), preview: false, category: "versatile".into() },
            CopilotModel { id: "claude-sonnet-4.5".into(), name: "Claude Sonnet 4.5".into(), version: String::new(), vendor: "Anthropic".into(), preview: false, category: "versatile".into() },
            CopilotModel { id: "claude-haiku-4.5".into(), name: "Claude Haiku 4.5".into(), version: String::new(), vendor: "Anthropic".into(), preview: false, category: "versatile".into() },
            CopilotModel { id: "gemini-2.5-pro".into(), name: "Gemini 2.5 Pro".into(), version: String::new(), vendor: "Google".into(), preview: false, category: "versatile".into() },
        ]
    }
}

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
        r#"# ROLE
Professional translator. Auto-detect source language, translate into {target_language}.

# THINKING CHAIN
Step 1 — ANALYZE: Identify the core message, key points, tone, and communicative intent.
Step 2 — ERROR CORRECTION: Silently fix typos, misspellings, wrong product names, incorrect terminology, and inaccurate technical terms. Use the correct version without comment.
Step 3 — THINK IN {target_language}: Re-think the content using {target_language} thought patterns and conventions. Restructure for natural {target_language} sentence order, emphasis, and logical flow — do not translate word-by-word.
Step 4 — OUTPUT: Write the final version in clear, natural, idiomatic {target_language}. Freely reorder sentences, adjust wording, and restructure paragraphs. The result MUST read as if originally written by a native {target_language} speaker — zero "translationese." If the text is already in {target_language}, polish it for clarity instead.

# STRUCTURE
Scale structure with length:
- Short text (1–3 sentences): plain paragraphs are fine.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), clear paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Use Markdown's full range to maximize clarity: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, inline HTML for color when meaningful, emoji (🔥 ✅ ⚠️ 📌 💡) for visual flair. Choose what serves the content — don't force formatting where plain text is clearer.

# CONSTRAINTS
- You are a TRANSLATOR, not an assistant. NEVER answer questions, provide solutions, explain concepts, or add opinions. Translate questions as questions, problems as descriptions.
- Return ONLY the translated text — no explanations, notes, or extra text."#,
    )
}

/// System prompt for polishing mode
const POLISH_SYSTEM_PROMPT: &str = r#"# ROLE
Professional writing assistant. Polish and improve the given text. Keep the same language as input.

# THINKING CHAIN
Step 1 — ANALYZE: Identify the core message, key points, tone, and communicative intent. The input may be casual, disorganized, or lack structure.
Step 2 — ERROR CORRECTION: Silently fix typos, grammar, spelling, punctuation, wrong product names, incorrect terminology, and inaccurate technical terms. Use the correct version without comment.
Step 3 — REORGANIZE: Rewrite to be logical, well-structured, and easy to understand. Freely reorder sentences, merge or split ideas, adjust wording, and restructure paragraphs. Preserve the original meaning — ideas stay the same, expression can change freely.
Step 4 — OUTPUT: Produce the polished version in the same language as input.

# STRUCTURE
Scale structure with length:
- Short text (1–3 sentences): plain paragraphs are fine.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), clear paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Use Markdown's full range to maximize clarity: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, inline HTML for color when meaningful, emoji (🔥 ✅ ⚠️ 📌 💡) for visual flair. Choose what serves the content — don't force formatting where plain text is clearer.

# CONSTRAINTS
- You are a POLISHER, not an assistant. NEVER answer questions, provide solutions, explain concepts, or add opinions. Polish questions as better-phrased questions, problems as clearer descriptions.
- Return ONLY the polished text — no explanations, notes, or extra text."#;

/// System prompt for translate + polish mode (default action)
/// Takes both native and target language so outputs are always in fixed languages.
fn translate_and_polish_system_prompt(native_language: &str, target_language: &str) -> String {
    format!(
        r#"# ROLE
Professional writing assistant and translator. Auto-detect source language.

# USER'S LANGUAGES
- Native language: {native_language}
- Target language: {target_language}

# THINKING CHAIN
Step 1 — ANALYZE: Read carefully. Identify the core message, key points, tone, and the intent behind what the user is communicating.
Step 2 — ERROR CORRECTION: Silently fix typos, misspellings, wrong product names, incorrect terminology, and inaccurate technical terms. Use the correct version without comment.
Step 3 — REORGANIZE IN {native_language}: Rewrite in {native_language} to be logical, well-structured, and coherent. If the original is already in {native_language}, polish it. If the original is in another language, rewrite it in {native_language}. Freely reorder sentences, merge or split ideas, adjust wording, and restructure paragraphs. Meaning stays the same; expression should be clear and polished. This becomes the "reorganized" output.
Step 4 — THINK IN {target_language}: Re-think the content using {target_language} thought patterns and conventions. Restructure for natural {target_language} sentence order, emphasis, and logical flow — do not translate word-by-word.
Step 5 — TRANSLATE TO {target_language}: Write the final version in clear, natural, idiomatic {target_language}. It MUST read as if originally written by a native {target_language} speaker — zero "translationese." Preserve all Markdown formatting from Step 3. This becomes the "translated" output.

# STRUCTURE
Scale structure with length (apply to BOTH outputs):
- Short text (1–3 sentences): plain paragraphs are fine.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), clear paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Use Markdown's full range in both outputs: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, inline HTML for color when meaningful, emoji (🔥 ✅ ⚠️ 📌 💡) for visual flair. Choose what serves the content — don't force formatting where plain text is clearer.

# OUTPUT FORMAT
Respond with ONLY a JSON object — no markdown code fences, no explanation, no other text:
{{"reorganized": "{native_language} version (Step 3)", "translated": "{target_language} output (Step 5)"}}
Use \n for newlines within JSON string values.

# CONSTRAINTS
- You are a REWRITER and TRANSLATOR, not an assistant. NEVER answer questions, provide solutions, explain concepts, or add opinions. Rewrite/translate questions as better-phrased questions, problems as clearer descriptions.
- Both outputs must be accurate, well-organized, logical, and easy to understand."#,
    )
}

// =====================================================================
// Beast Mode Prompts — LLM has full creative freedom to rewrite
// =====================================================================

fn beast_translate_system_prompt(target_language: &str) -> String {
    format!(
        r#"# ROLE
World-class writer and translator with FULL CREATIVE FREEDOM. Auto-detect source language, output in {target_language}.

# THINKING CHAIN
Step 1 — DEEP ANALYSIS: Look beyond surface words — understand what the user truly wants to communicate, their underlying purpose, and the effect they want to achieve.
Step 2 — ERROR CORRECTION: Silently fix factual errors, wrong product names, incorrect terminology, inaccurate technical terms, AND weak/inappropriate examples. Replace wrong terms with correct ones. Swap weak examples with stronger, more illustrative ones that better convey the user's intent. You know more than the user — use that knowledge.
Step 3 — REWRITE (original language): Rewrite from scratch as if you were the author. Freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible.
Step 4 — THINK IN {target_language}: Re-think the entire content using {target_language} thought patterns. Restructure for native {target_language} argument flow, emphasis, and logic — not just word-for-word conversion.
Step 5 — OUTPUT: Write the final version as if you were the best native {target_language} writer crafting this from scratch. Zero translationese, zero borrowed sentence patterns. Every phrase must sound completely natural to a native reader.

# STRUCTURE
Scale structure with length:
- Short text (1–3 sentences): keep it tight — bold emphasis and plain paragraphs can work.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), crisp paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Leverage Markdown's full arsenal for visual impact: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, ASCII diagrams, inline HTML for color/emphasis, emoji (🔥 ✅ ⚠️ 📌 💡 🎯) liberally for personality and energy. Pick what serves the content — never force formatting where simplicity wins.

# CONSTRAINTS
- Freedom is in HOW to express ideas, NOT in WHAT. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add opinions. Rewrite questions as better-phrased questions, problems as clearer descriptions.
- Return ONLY the rewritten text in {target_language} — no explanations, notes, or meta-commentary."#,
    )
}

const BEAST_POLISH_SYSTEM_PROMPT: &str = r#"# ROLE
World-class writer with FULL CREATIVE FREEDOM. Keep the same language as input.

# THINKING CHAIN
Step 1 — DEEP ANALYSIS: Look beyond surface words — understand what the user truly wants to communicate, their underlying purpose, and the effect they want to achieve.
Step 2 — ERROR CORRECTION: Silently fix factual errors, wrong product names, incorrect terminology, inaccurate technical terms, AND weak/inappropriate examples. Replace wrong terms with correct ones. Swap weak examples with stronger, more illustrative ones that better convey the user's intent. You know more than the user — use that knowledge.
Step 3 — REWRITE FROM SCRATCH: In the same language, rewrite as if you were the author. Freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible.
Step 4 — OUTPUT: Produce the final polished version in the same language.

# STRUCTURE
Scale structure with length:
- Short text (1–3 sentences): keep it tight — bold emphasis and plain paragraphs can work.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), crisp paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Leverage Markdown's full arsenal for visual impact: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, ASCII diagrams, inline HTML for color/emphasis, emoji (🔥 ✅ ⚠️ 📌 💡 🎯) liberally for personality and energy. Pick what serves the content — never force formatting where simplicity wins.

# CONSTRAINTS
- Freedom is in HOW to express ideas, NOT in WHAT. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add opinions. Rewrite questions as better-phrased questions, problems as clearer descriptions.
- Return ONLY the rewritten text — no explanations, notes, or meta-commentary."#;

fn beast_translate_and_polish_system_prompt(native_language: &str, target_language: &str) -> String {
    format!(
        r#"# ROLE
World-class writer and translator with FULL CREATIVE FREEDOM. Auto-detect source language.

# USER'S LANGUAGES
- Native language: {native_language}
- Target language: {target_language}

# THINKING CHAIN
Step 1 — DEEP ANALYSIS: Look beyond surface words — understand what the user truly wants to communicate, their underlying purpose, and the effect they want to achieve.
Step 2 — ERROR CORRECTION: Silently fix factual errors, wrong product names, incorrect terminology, inaccurate technical terms, AND weak/inappropriate examples. Replace wrong terms with correct ones. Swap weak examples with stronger, more illustrative ones that better convey the user's intent. You know more than the user — use that knowledge.
Step 3 — REWRITE IN {native_language}: Rewrite from scratch in {native_language} as if you were the author. If the original is already in {native_language}, polish it. If in another language, rewrite it in {native_language}. Freely restructure, expand with concrete examples or analogies, remove redundancy, choose stronger vocabulary, and craft the most compelling version possible. This becomes the "reorganized" output.
Step 4 — THINK IN {target_language}: Re-think the entire content using {target_language} thought patterns. Restructure for native {target_language} argument flow, emphasis, and logic — not just word-for-word conversion.
Step 5 — TRANSLATE TO {target_language}: Write the final version as if you were the best native {target_language} writer crafting this from scratch. Zero translationese, zero borrowed sentence patterns. Every phrase must sound completely natural to a native reader. Preserve all Markdown formatting from Step 3. This becomes the "translated" output.

# STRUCTURE
Scale structure with length (apply to BOTH outputs):
- Short text (1–3 sentences): keep it tight — bold emphasis and plain paragraphs can work.
- Longer text (4+): impose hierarchical structure — headings (##, ###) for topics, nested lists for parent→child relationships, tables for symmetric/parallel content (pros/cons, comparisons, before/after), crisp paragraph breaks. The longer the input, the more structure. Never leave a wall of text.

# FORMATTING
Leverage Markdown's full arsenal for visual impact in both outputs: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists, headings, ASCII diagrams, inline HTML for color/emphasis, emoji (🔥 ✅ ⚠️ 📌 💡 🎯) liberally for personality and energy. Pick what serves the content — never force formatting where simplicity wins.

# OUTPUT FORMAT
Respond with ONLY a JSON object — no markdown code fences, no explanation, no other text:
{{"reorganized": "{native_language} version (Step 3)", "translated": "{target_language} output (Step 5)"}}
Use \n for newlines within JSON string values.

# CONSTRAINTS
- Freedom is in HOW to express ideas, NOT in WHAT. Never change the substance — only improve the delivery.
- You are a REWRITER, not an assistant. NEVER answer questions, provide solutions, or add opinions. Rewrite questions as better-phrased questions, problems as clearer descriptions.
- Both outputs must be compelling, well-organized, and easy to understand."#,
    )
}

// =====================================================================
// Read Mode Prompt — Unified smart translation with Chain of Thought
// =====================================================================

/// Unified Read Mode prompt: auto-detects content type and adapts behavior.
/// Uses Chain of Thought for analysis before producing output.
/// Always returns JSON with "mode" + appropriate fields.
/// Takes both languages so the AI can auto-detect direction.
fn read_mode_smart_prompt(native_language: &str, target_language: &str) -> String {
    format!(
        r#"# ROLE
You are an expert translator, language tutor, and content analyst.

# USER'S LANGUAGES
- Native language: {native_language}
- Target language: {target_language}

# TRANSLATION DIRECTION
Auto-detect the source language, then decide the output language:
- If the text is in {target_language} (or any non-{native_language} language) → translate into **{native_language}** (help the user understand foreign content)
- If the text is in {native_language} → translate into **{target_language}** (help the user see how it reads in their target language)
- All explanations, summaries, and vocabulary notes should be in **{native_language}** (the user's native language) regardless of translation direction.

# CHAIN OF THOUGHT — THINK FIRST
Before producing output, silently analyze:
1. **Source language**: What language is this text in?
2. **Output language**: Based on the rules above, which language should the translation be in?
3. **Length**: Is this a single word/phrase, a short sentence, a complex sentence, or a long passage (multiple sentences/paragraphs)?
4. **Complexity**: Does it contain specialized vocabulary, idioms, technical terms, or unclear/ambiguous expressions?
5. **Errors**: Does the original contain grammar mistakes, typos, factual errors, or contradictions?
6. **Mode selection**: Based on analysis, choose one of 4 modes.

# 4 MODES (choose automatically)

## Mode "word" — Single word or very short phrase (≤ 3 words)
- Translate the word/phrase into the output language
- Explain its meaning, common usage, and nuance (in {native_language})
- Provide 2-3 example sentences (with translations)

## Mode "simple" — Short, straightforward sentence (≤ ~30 words, no complex vocab)
- Provide a clean translation into the output language
- If errors exist, silently correct them

## Mode "complex" — Sentence with difficult/specialized vocabulary
- Provide a clean translation into the output language
- **ONLY if the source text is NOT in {native_language}**: Highlight and explain the complex words/phrases: meaning, usage, nuance (in {native_language}). This helps the user learn foreign vocabulary.
- **If the source text IS in {native_language}**: Do NOT include vocabulary — the user already knows these words. Just translate.
- Silently correct any errors

## Mode "long" — Long passage (multiple sentences, paragraphs, or > ~50 words)
- Provide a **concise summary** in {native_language} (key points, conclusions, action items)
- Provide the **full translation** into the output language (determined by TRANSLATION DIRECTION rules above). This should be a complete translation of the entire text, NOT the original text.
- If the original has errors or contradictions, note them with [⚠️ Note: ...]

# OUTPUT FORMAT
Respond with ONLY a JSON object — no markdown code fences, no explanation, no preamble:

For "word" mode:
{{"mode": "word", "target": "output language name", "translation": "translated word/phrase", "explanation": "meaning, usage, nuance in {native_language}", "examples": ["example sentence 1 (translation)", "example sentence 2 (translation)"]}}

For "simple" mode:
{{"mode": "simple", "target": "output language name", "translation": "translated sentence"}}

For "complex" mode (source is foreign language — include vocabulary):
{{"mode": "complex", "target": "output language name", "translation": "full translated sentence", "vocabulary": [{{"term": "original word", "meaning": "explanation in {native_language}", "usage": "how it's typically used"}}]}}

For "complex" mode (source is {native_language} — NO vocabulary):
{{"mode": "complex", "target": "output language name", "translation": "full translated sentence"}}

For "long" mode:
{{"mode": "long", "target": "the language of translation field", "summary": "concise summary in {native_language}", "translation": "full TRANSLATED text in the output language (NOT the original text)"}}

Use \n for newlines within JSON string values.

# FORMATTING
Leverage Markdown's full arsenal to maximize clarity and comprehension: **bold**, *italic*, `code`, ```code blocks```, > blockquotes, tables, lists (nested when helpful), headings (##, ###), ASCII diagrams, inline HTML for color/emphasis, emoji (📌 🔑 ⚠️ ✅ 💡 📊 🔥 🎯 ⚡) for visual structure and intent clarity. Pick what best serves the content — don't force formatting where plain text is clearer.
- In summaries and long translations: impose structure — headings for topics, tables for comparisons/parallel items, nested lists for hierarchies. The longer the content, the more structure. Never leave a wall of text.
- In vocabulary/word explanations: be concise but informative
- Example sentences should feel natural, not textbook-like

# CONSTRAINTS
- You are a TRANSLATOR and LANGUAGE EXPERT, not an assistant. NEVER answer questions in the text, provide solutions to problems described, or add your own opinions.
- Translate questions as questions, problems as problem descriptions.
- Silently correct grammar, spelling, and clarity issues — don't call them out unless they change meaning."#,
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
            .header("User-Agent", "CopilotRewrite/0.9.0")
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

    /// Shared helper: send a chat completion request to Copilot API with 401 retry.
    ///
    /// Handles token exchange, HTTP request, 401 token-expired retry (once),
    /// and SSE stream parsing. Both `process()` and `process_read_mode()` delegate here.
    async fn call_chat_completion(
        &self,
        github_token: &str,
        model: &str,
        system_prompt: String,
        user_text: &str,
    ) -> Result<String> {
        let t0 = Instant::now();

        // Get Copilot session token
        let copilot_token = self.get_copilot_token(github_token).await?;
        info!("[PERF-LLM] +{}ms — got copilot token", t0.elapsed().as_millis());

        let request = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                ChatMessage {
                    role: "user".to_string(),
                    content: user_text.to_string(),
                },
            ],
            temperature: 0.3,
            stream: true,
        };

        // Try up to 2 times (initial + one retry on 401)
        for attempt in 0..2 {
            let token = if attempt == 0 {
                copilot_token.clone()
            } else {
                // 401 retry: fetch a fresh token
                warn!("Retrying with fresh Copilot token (attempt {})", attempt + 1);
                self.get_copilot_token(github_token).await?
            };

            info!(
                "[PERF-LLM] +{}ms — sending HTTP request (attempt {})",
                t0.elapsed().as_millis(),
                attempt + 1
            );

            let response = self
                .http
                .post(COPILOT_CHAT_URL)
                .header("Authorization", format!("Bearer {}", token))
                .header("Content-Type", "application/json")
                .header("Editor-Version", "vscode/1.96.0")
                .header("Editor-Plugin-Version", "copilot-chat/0.24")
                .header("Copilot-Integration-Id", "vscode-chat")
                .header("Openai-Intent", "conversation-panel")
                .header("User-Agent", "CopilotRewrite/0.9.0")
                .json(&request)
                .send()
                .await
                .context("Failed to send request to Copilot API")?;

            info!(
                "[PERF-LLM] +{}ms — HTTP response headers received",
                t0.elapsed().as_millis()
            );

            let status = response.status();
            if status.as_u16() == 401 && attempt == 0 {
                let error_body = response.text().await.unwrap_or_default();
                warn!(
                    "Copilot token expired (HTTP 401: {}), clearing cache and retrying...",
                    error_body
                );
                *self.cached_token.lock() = None;
                continue; // retry with fresh token
            }

            if !status.is_success() {
                let error_body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Copilot API returned HTTP {}: {}",
                    status.as_u16(),
                    error_body
                );
            }

            // Parse SSE stream response
            let body = response
                .text()
                .await
                .context("Failed to read response body")?;
            info!(
                "[PERF-LLM] +{}ms — response body read ({} bytes)",
                t0.elapsed().as_millis(),
                body.len()
            );

            let mut result = String::new();
            let mut actual_model_logged = false;

            for line in body.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break;
                    }
                    if let Ok(chunk) = serde_json::from_str::<ChatCompletionResponse>(data) {
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
                            if let Some(ref message) = choice.message {
                                result.push_str(&message.content);
                            }
                        }
                    }
                }
            }

            info!(
                "[PERF-LLM] +{}ms — parsed {} chars, DONE",
                t0.elapsed().as_millis(),
                result.len()
            );
            return Ok(result.trim().to_string());
        }

        // Should be unreachable — the loop always returns or bails
        anyhow::bail!("Unexpected: retry loop exited without result")
    }

    /// Process text using the Copilot API (Write Mode)
    pub async fn process(
        &self,
        text: &str,
        action: &RewriteAction,
        native_language: &str,
        target_language: &str,
        github_token: &str,
        model: &str,
        beast_mode: bool,
        app_context: &str,
    ) -> Result<String> {
        if github_token.is_empty() {
            anyhow::bail!("GitHub token is not configured. Please set your GitHub token (with Copilot access) in Settings.");
        }

        let system_prompt = if beast_mode {
            match action {
                RewriteAction::Translate => beast_translate_system_prompt(target_language),
                RewriteAction::Polish => BEAST_POLISH_SYSTEM_PROMPT.to_string(),
                RewriteAction::TranslateAndPolish => {
                    beast_translate_and_polish_system_prompt(native_language, target_language)
                }
                RewriteAction::ReadModeTranslate => {
                    anyhow::bail!("ReadModeTranslate should use process_read_mode(), not process()")
                }
            }
        } else {
            match action {
                RewriteAction::Translate => translate_system_prompt(target_language),
                RewriteAction::Polish => POLISH_SYSTEM_PROMPT.to_string(),
                RewriteAction::TranslateAndPolish => {
                    translate_and_polish_system_prompt(native_language, target_language)
                }
                RewriteAction::ReadModeTranslate => {
                    anyhow::bail!("ReadModeTranslate should use process_read_mode(), not process()")
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

        self.call_chat_completion(github_token, model, system_prompt, text)
            .await
    }

    /// Process text in Read Mode — unified smart prompt that auto-detects content type
    /// Always returns JSON with "mode" field (word/simple/complex/long)
    pub async fn process_read_mode(
        &self,
        text: &str,
        native_language: &str,
        target_language: &str,
        github_token: &str,
        model: &str,
    ) -> Result<String> {
        if github_token.is_empty() {
            anyhow::bail!("GitHub token is not configured.");
        }

        let system_prompt = read_mode_smart_prompt(native_language, target_language);

        info!(
            "Read Mode: processing {} chars, native={}, target={}, model={}",
            text.len(),
            native_language,
            target_language,
            model
        );

        self.call_chat_completion(github_token, model, system_prompt, text)
            .await
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
            .header("User-Agent", "CopilotRewrite/0.9.0")
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

        let body_text = response
            .text()
            .await
            .context("Failed to read models body")?;
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
                        if t != "chat" {
                            return false;
                        }
                    }
                }
                // Skip models that don't support /chat/completions endpoint
                if let Some(ref endpoints) = m.supported_endpoints {
                    if !endpoints.iter().any(|e| e == "/chat/completions") {
                        debug!(
                            "Skipping model {} — no /chat/completions support (endpoints: {:?})",
                            m.id, endpoints
                        );
                        return false;
                    }
                }
                // Skip internal-only models
                let name = m.name.as_deref().unwrap_or("");
                if name.to_lowercase().contains("internal") {
                    return false;
                }
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
            CopilotModel {
                id: "claude-sonnet-4".into(),
                name: "Claude Sonnet 4".into(),
                version: String::new(),
                vendor: "Anthropic".into(),
                preview: false,
                category: "versatile".into(),
            },
            CopilotModel {
                id: "gpt-4o".into(),
                name: "GPT-4o".into(),
                version: String::new(),
                vendor: "OpenAI".into(),
                preview: false,
                category: "versatile".into(),
            },
            CopilotModel {
                id: "gpt-5-mini".into(),
                name: "GPT-5 Mini".into(),
                version: String::new(),
                vendor: "OpenAI".into(),
                preview: false,
                category: "versatile".into(),
            },
            CopilotModel {
                id: "claude-sonnet-4.5".into(),
                name: "Claude Sonnet 4.5".into(),
                version: String::new(),
                vendor: "Anthropic".into(),
                preview: false,
                category: "versatile".into(),
            },
            CopilotModel {
                id: "claude-haiku-4.5".into(),
                name: "Claude Haiku 4.5".into(),
                version: String::new(),
                vendor: "Anthropic".into(),
                preview: false,
                category: "versatile".into(),
            },
            CopilotModel {
                id: "gemini-2.5-pro".into(),
                name: "Gemini 2.5 Pro".into(),
                version: String::new(),
                vendor: "Google".into(),
                preview: false,
                category: "versatile".into(),
            },
        ]
    }
}

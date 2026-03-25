// Copilot API client module
// Handles communication with GitHub Copilot chat completions API

pub mod client;
pub mod oauth;

pub use client::CopilotClient;
pub use oauth::{DeviceCodeResponse, SavedAuth};

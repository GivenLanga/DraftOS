//! Model Adapter SDK: every LLM DraftOS can talk to sits behind [`ModelAdapter`].
//!
//! This crate (together with draftos-embed) is the only place in the workspace
//! allowed to make network calls. No other crate may import a provider SDK or
//! hardcode a provider API shape (CLAUDE.md §9).
//!
//! Adapters are synchronous, matching the rest of the core crates; the desktop
//! shell runs them on background threads. Streaming is not implemented yet —
//! `Capabilities::streaming` reports it honestly.

mod key;

#[cfg(feature = "anthropic")]
mod anthropic;
#[cfg(feature = "openai-compat")]
mod openai_compat;

#[cfg(feature = "anthropic")]
pub use anthropic::AnthropicAdapter;
#[cfg(feature = "openai-compat")]
pub use openai_compat::OpenAiCompatAdapter;

pub use key::{delete_key, store_key};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("no API key for '{adapter}': set the {env_var} environment variable or run `draftos model key {adapter}`")]
    MissingKey { adapter: String, env_var: String },

    #[error("keychain error: {0}")]
    Keychain(String),

    #[error("HTTP {status} from {adapter}: {body}")]
    Api {
        adapter: String,
        status: u16,
        body: String,
    },

    #[error("network error talking to {adapter}: {message}")]
    Network { adapter: String, message: String },

    #[error("the model declined this request: {0}")]
    Refused(String),

    #[error("unexpected response shape from {adapter}: {message}")]
    BadResponse { adapter: String, message: String },

    #[error("unknown adapter '{0}' — known: anthropic, openai, ollama, vllm, deepseek, mistral, qwen")]
    UnknownAdapter(String),
}

pub type Result<T> = std::result::Result<T, ModelError>;

/// What a mounted model can do. The prompt compiler and UI consult this.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Capabilities {
    /// Provider can be asked to emit strict JSON.
    pub json_mode: bool,
    /// Streaming completions implemented in this adapter.
    pub streaming: bool,
    /// Runs on the user's machine — no data leaves it.
    pub local: bool,
    /// Rough context window in tokens, for prompt budgeting.
    pub max_context: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// A completion request. Only draftos-prompt builds these for drafting tasks;
/// ad-hoc prompt strings elsewhere in the codebase are a defect (CLAUDE.md §9).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    /// Ask the provider for strict JSON output where supported.
    pub json: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub text: String,
    /// Model that actually served the request, as reported by the provider.
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub stop_reason: Option<String>,
}

pub trait ModelAdapter: Send + Sync {
    /// Stable adapter id: "anthropic", "openai", "ollama", …
    fn id(&self) -> &str;
    fn capabilities(&self) -> Capabilities;
    fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse>;
}

/// Persisted model selection (stored in app settings — never the API key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Adapter id: "anthropic", "openai", "ollama", "vllm", "deepseek",
    /// "mistral", "qwen".
    pub adapter: String,
    /// Provider model id, e.g. "claude-opus-4-8". Empty = adapter default.
    #[serde(default)]
    pub model: String,
    /// Endpoint override for OpenAI-compatible/self-hosted providers.
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Instantiate the adapter a [`ModelConfig`] describes.
pub fn build_adapter(cfg: &ModelConfig) -> Result<Box<dyn ModelAdapter>> {
    let model = if cfg.model.is_empty() { None } else { Some(cfg.model.clone()) };
    match cfg.adapter.as_str() {
        #[cfg(feature = "anthropic")]
        "anthropic" => Ok(Box::new(AnthropicAdapter::new(model, cfg.base_url.clone()))),
        #[cfg(feature = "openai-compat")]
        "openai" | "ollama" | "vllm" | "deepseek" | "mistral" | "qwen" => Ok(Box::new(
            OpenAiCompatAdapter::new(&cfg.adapter, model, cfg.base_url.clone()),
        )),
        other => Err(ModelError::UnknownAdapter(other.to_string())),
    }
}

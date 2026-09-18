pub mod anthropic;
pub mod anthropic_compat;
pub mod custom;
pub mod factory;
pub mod minimax;
pub mod minimax_vlm;
pub mod openai;
pub mod openai_compat;
pub mod openrouter;
#[cfg(test)]
pub mod test_support;
pub mod validate;

/// How long a streaming response may go **without a single byte** before the
/// HTTP client gives up on the body.
///
/// This is `reqwest`'s inter-read timeout, not a total deadline: it restarts on
/// every byte. It was 60 s, and 60 s is too short for a streaming chat
/// completion — a model that thinks before it emits, or a slow link, produces
/// exactly this shape (a few tokens, a long silence, the rest). Measured
/// 2026-09-19 against a server that pauses mid-stream on purpose: a 40 s gap is
/// delivered intact, a 70 s gap is not.
///
/// What made it expensive to find is the error it produces. `reqwest` surfaces
/// an expired read timeout as a **body error**, so the user sees
/// `error decoding response body` — which reads as "the server sent something
/// malformed" and sends you looking at TLS, chunked framing and the network
/// stack. Nothing was wrong with any of them; the client simply stopped
/// listening. (A deliberately unframed test server produces the *same* message,
/// which is worth knowing before trusting it.)
///
/// 300 s still catches a genuinely dead connection — a peer that vanishes
/// without a FIN — while leaving room for a model to think.
pub const STREAM_READ_TIMEOUT_SECS: u64 = 300;

use std::path::Path;

use async_trait::async_trait;
use nca_common::message::Message;
use nca_common::tool::{ToolCall, ToolDefinition};

/// A streamed chunk from the provider.
#[derive(Debug, Clone)]
pub enum StreamChunk {
    TextDelta(String),
    ToolUse(ToolCall),
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    Done,
}

/// Abstraction over LLM providers (Anthropic, OpenAI, Gemini, etc.).
#[async_trait]
pub trait Provider: Send + Sync {
    /// Rewrite conversation history before an HTTP request (e.g. MiniMax `coding_plan/vlm` for images).
    /// Default: no-op.
    async fn prepare_messages_for_request(
        &self,
        _messages: &mut Vec<Message>,
        _workspace_root: &Path,
    ) -> Result<(), ProviderError> {
        Ok(())
    }

    /// Send a conversation and receive a streaming response.
    ///
    /// `workspace_root` is used to resolve on-disk image paths embedded in user messages.
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        model: &str,
        workspace_root: &Path,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamChunk>, ProviderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider configuration error: {0}")]
    Configuration(String),
    #[error("API request failed: {0}")]
    RequestFailed(String),
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Rate limited, retry after {retry_after_ms}ms")]
    RateLimited { retry_after_ms: u64 },
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("{0}")]
    Other(String),
}

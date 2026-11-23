#![forbid(unsafe_code)]

pub mod claude_api;
pub mod claude_cli;
pub mod error;
pub mod grouping;
pub mod local;
pub mod markdown;
pub mod ollama;
pub mod page_context;
pub mod provider;
pub mod secrets;
pub mod transport;

pub use claude_api::{ApiToken, ClaudeApi};
pub use claude_cli::ClaudeCli;
pub use error::AiError;
pub use local::{Availability, LocalModel};
pub use ollama::Ollama;
pub use page_context::PageContext;
pub use provider::{Provider, Request};
pub use secrets::Secrets;
pub use transport::Transport;
#[cfg(not(target_os = "android"))]
pub use transport::UreqTransport;

#![forbid(unsafe_code)]

pub mod claude_api;
pub mod claude_cli;
pub mod grouping;
#[cfg(any(target_os = "macos", windows))]
pub mod keychain;
pub mod markdown;
pub mod ollama;
pub mod page_context;
pub mod provider;

pub use claude_api::{ApiToken, ClaudeApi};
pub use claude_cli::ClaudeCli;
pub use ollama::Ollama;
pub use page_context::PageContext;
pub use provider::{AiError, Provider, Request};

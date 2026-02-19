#![forbid(unsafe_code)]

pub mod adapter;

pub use adapter::ClaudeProviderAdapter;
pub use claude_core::{client, config, prompts};

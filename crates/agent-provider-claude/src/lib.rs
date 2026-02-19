#![forbid(unsafe_code)]

pub mod adapter;
pub mod client;
pub mod config;
pub mod prompts;

pub use adapter::ClaudeProviderAdapter;
pub use config::{
    ClaudeAuthState, ClaudeConfig, ConfigError, api_key_configured, auth_scopes, auth_state,
    auth_subject, claude_cli_available, max_concurrency,
};

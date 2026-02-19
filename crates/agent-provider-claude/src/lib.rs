#![forbid(unsafe_code)]

pub mod adapter;
pub mod client;
pub mod config;
pub mod prompts;

pub use adapter::ClaudeProviderAdapter;

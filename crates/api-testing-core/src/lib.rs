//! Shared core primitives for the api-testing CLIs.

pub mod config;
pub mod env_file;
pub mod graphql;
pub mod history;
pub mod http;
pub mod jq;
pub mod jwt;
pub mod markdown;
pub mod redact;
pub mod report;
pub mod rest;
pub mod suite;

pub type Result<T> = std::result::Result<T, anyhow::Error>;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

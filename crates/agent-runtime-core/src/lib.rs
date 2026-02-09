#![forbid(unsafe_code)]
//! Provider-neutral runtime contracts shared by provider adapters and `agentctl`.
//!
//! The current adapter contract is versioned as `provider-adapter.v1` and covers:
//! `capabilities`, `healthcheck`, `execute`, `limits`, and `auth-state`.

pub mod provider;
pub mod schema;

pub use provider::{ProviderAdapter, ProviderAdapterV1};
pub use schema::CONTRACT_VERSION_V1;

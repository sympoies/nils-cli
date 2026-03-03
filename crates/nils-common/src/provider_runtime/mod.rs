pub mod auth;
pub mod config;
pub mod error;
pub mod exec;
pub mod json;
pub mod jwt;
pub mod paths;
pub mod persistence;
pub mod profile;

pub use error::{CoreError, CoreErrorCategory, ProviderCategoryHint};
pub use profile::{
    ExecInvocation, ExecProfile, HomePathSelection, PathsProfile, ProviderDefaults,
    ProviderEnvKeys, ProviderProfile,
};

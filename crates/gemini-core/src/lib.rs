#![forbid(unsafe_code)]
//! Shared runtime primitives for gemini integrations.
//!
//! This crate intentionally excludes CLI parsing/rendering concerns. Consumers
//! should map core outputs/errors to their own user-facing contracts.

pub mod auth;
pub mod config;
pub mod error;
pub mod exec;
pub mod json;
pub mod jwt;
pub mod paths;

pub use error::{CoreError, CoreErrorCategory, ProviderCategoryHint};

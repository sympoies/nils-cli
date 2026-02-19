#![forbid(unsafe_code)]
//! Shared runtime primitives for claude integrations.
//!
//! This crate intentionally excludes CLI parsing/rendering concerns. Consumers
//! should map core outputs/errors to their own user-facing contracts.

pub mod client;
pub mod config;
pub mod exec;
pub mod prompts;

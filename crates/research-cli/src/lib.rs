//! One research engine shared by the CLI and local HTTP adapter.
pub mod cli;
mod contract;
mod engine;
pub mod fetch;
pub mod http;
mod output;
pub mod providers;
pub mod transport;

pub use contract::*;
pub use engine::Engine;
pub use output::{exit_code, render_text};

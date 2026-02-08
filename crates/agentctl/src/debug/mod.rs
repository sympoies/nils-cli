pub mod bundle;
pub mod schema;
pub mod sources {
    pub mod git_context;
    pub mod image_processing;
    pub mod macos_agent;
    pub mod screen_record;
}

use clap::{Args, Subcommand};

pub const EXIT_OK: i32 = 0;
pub const EXIT_RUNTIME_ERROR: i32 = 1;

#[derive(Debug, Args)]
pub struct DebugArgs {
    #[command(subcommand)]
    pub command: Option<DebugSubcommand>,
}

#[derive(Debug, Subcommand)]
pub enum DebugSubcommand {
    /// Collect one-shot debug artifacts with a versioned manifest
    Bundle(bundle::BundleArgs),
}

pub fn run(command: DebugSubcommand) -> i32 {
    match command {
        DebugSubcommand::Bundle(args) => bundle::run(args),
    }
}

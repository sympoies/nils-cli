use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agentctl",
    version,
    about = "Provider-neutral control plane for local agent workflows",
    disable_help_subcommand = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Provider registry and selection
    Provider,
    /// Provider-neutral diagnostics
    Diag,
    /// Debug bundle and troubleshooting tools
    Debug,
    /// Declarative workflow orchestration
    Workflow,
    /// Local automation integrations
    Automation,
}

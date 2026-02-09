use crate::debug::DebugArgs;
use crate::diag::DiagArgs;
use crate::provider::commands::ProviderArgs;
use crate::workflow::WorkflowArgs;
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
    Provider(ProviderArgs),
    /// Provider-neutral diagnostics
    Diag(DiagArgs),
    /// Debug bundle and troubleshooting tools
    Debug(DebugArgs),
    /// Declarative workflow orchestration
    Workflow(WorkflowArgs),
    /// Local automation integrations
    Automation,
}

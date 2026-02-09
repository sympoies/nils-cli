pub mod run;
pub mod schema;
pub mod steps;

use clap::{Args, Subcommand};

#[derive(Debug, Args)]
pub struct WorkflowArgs {
    #[command(subcommand)]
    pub command: Option<WorkflowSubcommand>,
}

#[derive(Debug, Subcommand)]
pub enum WorkflowSubcommand {
    /// Execute workflow manifest
    Run(run::RunArgs),
}

pub fn run(command: WorkflowSubcommand) -> i32 {
    match command {
        WorkflowSubcommand::Run(args) => run::run(args),
    }
}

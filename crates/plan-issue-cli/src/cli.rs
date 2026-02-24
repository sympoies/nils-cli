use clap::{Parser, ValueEnum};

use crate::ValidationError;
use crate::commands::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Parser)]
#[command(
    version,
    about = "Rust implementation of the plan-issue orchestration workflow.",
    after_help = "Usage paths:\n  - plan-issue: live GitHub-backed orchestration\n  - plan-issue-local: local-first rehearsal and dry-run flow\n\nBoth binaries share the same typed command contract.",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Pass-through repository target for GitHub operations.
    #[arg(long, global = true, value_name = "owner/repo")]
    pub repo: Option<String>,

    /// Print write actions without mutating GitHub state.
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Output machine-readable JSON (alias for --format json).
    #[arg(long, global = true)]
    pub json: bool,

    /// Output format.
    #[arg(long, global = true, value_enum)]
    pub format: Option<OutputFormat>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn resolve_output_format(&self) -> Result<OutputFormat, ValidationError> {
        if self.json && matches!(self.format, Some(OutputFormat::Text)) {
            return Err(ValidationError::new(
                "invalid-output-mode",
                "--json cannot be combined with --format text",
            ));
        }

        if self.json || matches!(self.format, Some(OutputFormat::Json)) {
            return Ok(OutputFormat::Json);
        }

        Ok(OutputFormat::Text)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        self.command.validate(self.dry_run)
    }
}

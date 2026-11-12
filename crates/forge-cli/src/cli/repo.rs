//! `repo freeze` arguments.
use clap::{Args, Subcommand};

/// `repo freeze` arguments.
#[derive(Args, Debug, Clone)]
pub struct RepoFreezeArgs {
    #[command(subcommand)]
    pub command: RepoFreezeCommand,
}

/// `repo freeze` subcommands.
#[derive(Subcommand, Debug, Clone)]
pub enum RepoFreezeCommand {
    /// Open a merge freeze. `pr merge` refuses to merge while it is open.
    Start {
        /// Why merges are frozen; becomes the freeze issue title.
        #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
        reason: String,
        /// Expected end of the freeze, recorded in the issue body.
        #[arg(long, value_parser = clap::builder::NonEmptyStringValueParser::new())]
        until: Option<String>,
    },
    /// End a merge freeze by closing its issue. Without `--issue`, ends the
    /// single active freeze.
    End {
        /// Freeze issue number to close.
        #[arg(long)]
        issue: Option<u64>,
    },
    /// Report the active merge freezes.
    Status,
}

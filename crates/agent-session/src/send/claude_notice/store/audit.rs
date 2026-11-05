//! Explicit full verification of retained notice provenance and its checkpoint.
use super::*;
use clap::{Args as ClapArgs, Subcommand};
use nils_common::cli_contract::OutputFormat;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Verify every retained archive and every reachable checkpoint page.
    Audit {
        /// Verification deadline in seconds; automatic first import may finish after it.
        #[arg(long, default_value_t = 3600, value_parser = clap::value_parser!(u64).range(1..=86400))]
        timeout_seconds: u64,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

impl Args {
    pub(crate) fn format(&self) -> OutputFormat {
        match self.command {
            Command::Audit { format, .. } => format,
        }
    }
}

pub(crate) fn run(context: &CliContext, args: Args) -> i32 {
    let Command::Audit {
        timeout_seconds,
        format,
    } = args.command;
    match lifecycle::checkpoint::audit(
        &context.state_dir,
        Instant::now() + Duration::from_secs(timeout_seconds),
    ) {
        Ok(()) => crate::render_single_success(
            "notice-store.audit",
            format,
            &serde_json::json!({"verified":true}),
            |_| "notice store verified\n".into(),
        ),
        Err(_) => crate::render_error(
            "notice-store.audit",
            format,
            CliError::runtime(
                "notice-store-audit-failed",
                "notice store verification failed; retained state was not repaired or discarded",
                None,
            ),
        ),
    }
}

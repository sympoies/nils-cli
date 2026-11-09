//! Local owner CLI for the same saved-draft discard operation as serve.
use crate::{CliContext, CliError, send_guard::drafts};
use clap::{Args as ClapArgs, Subcommand};
use nils_common::cli_contract::OutputFormat;

pub(crate) const COMMAND: &str = "drafts.discard";

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Permanently discard one saved draft without submitting it.
    Discard(Discard),
}

#[derive(Debug, ClapArgs)]
pub struct Discard {
    /// Session that owns the draft, independent of its current runtime.
    pub session_id: String,
    /// Canonical saved-draft UUID.
    pub draft_id: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

impl Args {
    pub(crate) fn format(&self) -> OutputFormat {
        match &self.command {
            Command::Discard(args) => args.format,
        }
    }
}

pub(crate) fn run(context: &CliContext, args: Args) -> i32 {
    let Command::Discard(args) = args.command;
    let result = (|| -> Result<serde_json::Value, CliError> {
        let _lock = crate::acquire_session_record_lock(context, &args.session_id)?;
        let record = crate::load_session_record(context, &args.session_id)?;
        let mut result = drafts::discard(context, &record, &args.draft_id)?;
        result["machine"] = serde_json::json!(crate::board::machine_identity(None, context));
        Ok(result)
    })();
    match result {
        Ok(value) => crate::render_single_success(COMMAND, args.format, &value, |value| {
            format!(
                "discarded {} ({} remaining)\n",
                value["discarded"].as_str().unwrap_or_default(),
                value["remaining"]
            )
        }),
        Err(error) => crate::render_error(COMMAND, args.format, error),
    }
}

//! Local consumer interface; it never sends input or contacts a provider.
use crate::{
    CliContext,
    auth_incident::alert_delivery::{self, Target},
};
use clap::{Args as ClapArgs, Subcommand};
use nils_common::cli_contract::OutputFormat;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List durable authentication alerts without the serve daemon.
    List(List),
    /// Record a consumer's confirmed transport delivery.
    Ack(Ack),
}

#[derive(Debug, ClapArgs)]
pub struct List {
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u16).range(1..=200))]
    pub limit: u16,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, ClapArgs)]
pub struct Ack {
    pub session_id: String,
    pub alert_id: String,
    #[arg(long, value_enum)]
    pub target: Target,
    /// SHA-256 digest of the external delivery receipt, never its payload.
    #[arg(long)]
    pub receipt: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

pub(crate) fn run(context: &CliContext, args: Args) -> i32 {
    let (command, format, result) = match args.command {
        Command::List(args) => (
            "auth-alert.list",
            args.format,
            alert_delivery::list(context, args.limit.into()),
        ),
        Command::Ack(args) => (
            "auth-alert.ack",
            args.format,
            alert_delivery::acknowledge(
                context,
                &args.session_id,
                &args.alert_id,
                args.target,
                &args.receipt,
            )
            .and_then(|alert| {
                serde_json::to_value(alert).map_err(|_| {
                    crate::CliError::runtime(
                        "auth-alert-encode-failed",
                        "authentication alert could not be encoded",
                        None,
                    )
                })
            }),
        ),
    };
    match result {
        Ok(value) => {
            crate::render_single_success(command, format, &value, |value| format!("{value}\n"))
        }
        Err(error) => crate::render_error(command, format, error),
    }
}

pub(crate) fn name(args: &Args) -> &'static str {
    match &args.command {
        Command::List(_) => "auth-alert.list",
        Command::Ack(_) => "auth-alert.ack",
    }
}

pub(crate) fn format(args: &Args) -> OutputFormat {
    match &args.command {
        Command::List(args) => args.format,
        Command::Ack(args) => args.format,
    }
}

//! Explicit repository-local remote selection; authentication stays with identity preparation.
use clap::{Args, Subcommand, ValueEnum};
use nils_common::{cli_contract::OutputFormat, forge_identity, git};
use serde::Serialize;

use crate::worktree::{CliError, emit_error, emit_success, ensure_inside_git_repo};

#[derive(Debug, Args)]
#[command(disable_help_subcommand = true)]
pub(crate) struct TransportArgs {
    #[command(subcommand)]
    command: Option<TransportCommand>,
}

#[derive(Debug, Subcommand)]
enum TransportCommand {
    #[command(about = "Select the repository-local remote URL transport")]
    Set {
        #[arg(value_enum)]
        transport: Transport,
        #[command(flatten)]
        options: Options,
    },
    #[command(about = "Read the stored transport without contacting the forge")]
    Status(Options),
    #[command(about = "Display help message for transport")]
    Help,
}

#[derive(Clone, Copy, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum Transport {
    Https,
    Ssh,
}

#[derive(Debug, Args)]
struct Options {
    #[arg(long, default_value = "origin")]
    remote: String,
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,
}

#[derive(Serialize)]
struct State {
    remote: String,
    transport: Transport,
    url: String,
    recorded_in: String,
}

pub(crate) fn command_model() -> clap::Command {
    TransportArgs::augment_args(
        clap::Command::new("transport").about("Select and inspect repository transport"),
    )
}

pub(crate) fn run(args: TransportArgs) -> i32 {
    let (operation, options, selection) = match args.command {
        Some(TransportCommand::Set { transport, options }) => {
            ("transport.set", options, Some(transport))
        }
        Some(TransportCommand::Status(options)) => ("transport.status", options, None),
        Some(TransportCommand::Help) | None => {
            if let Err(error) = command_model().print_help() {
                eprintln!("{error}");
                return 1;
            }
            println!();
            return 0;
        }
    };
    let format = if options.format == "json" {
        OutputFormat::Json
    } else {
        OutputFormat::Text
    };
    match execute(&options.remote, selection) {
        Ok(state) => emit_success(operation, format, &state, || {
            format!("{}: {} ({})", state.remote, state.url, state.recorded_in)
        }),
        Err(error) => emit_error(operation, format, error),
    }
}

fn identity_error(error: forge_identity::Error) -> CliError {
    CliError::data(error.code, error.to_string())
        .with_hint("resolve remote URL ambiguity or overrides through their configuration owner; do not embed credentials in URLs")
}

fn execute(remote: &str, selection: Option<Transport>) -> Result<State, CliError> {
    ensure_inside_git_repo()?;
    if remote.is_empty()
        || remote.starts_with('-')
        || !remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(CliError::usage(
            "transport-remote-invalid",
            "expected a named Git remote",
        ));
    }
    // Resolve through the existing guard before displaying or changing any URL.
    let (target, url) =
        forge_identity::target_for_remote(None, remote, false).map_err(identity_error)?;
    let push_urls = git::run_output(&["config", "--get-all", &format!("remote.{remote}.pushurl")])
        .map_err(|_| {
            CliError::runtime(
                "transport-config-unavailable",
                "cannot inspect push URL configuration",
            )
        })?;
    match push_urls.status.code() {
        Some(1) => {}
        Some(0) => return Err(CliError::data(
            "transport-pushurl-configured",
            "an explicit push URL prevents a single transport choice",
        )
        .with_hint(
            "resolve remote push URL configuration through its owner before selecting transport",
        )),
        _ => {
            return Err(CliError::runtime(
                "transport-config-unavailable",
                "cannot inspect push URL configuration",
            ));
        }
    }
    let recorded_in = format!("remote.{remote}.url");
    let (transport, url) = match selection {
        Some(transport) => {
            let url = match transport {
                Transport::Https => format!("https://{}/{}.git", target.host, target.repo),
                Transport::Ssh => format!("git@{}:{}.git", target.host, target.repo),
            };
            let output =
                git::run_output(&["config", "--local", "--replace-all", &recorded_in, &url])
                    .map_err(|_| {
                        CliError::runtime(
                            "transport-config-write-failed",
                            "cannot write repository-local remote URL",
                        )
                    })?;
            if !output.status.success() {
                return Err(CliError::runtime(
                    "transport-config-write-failed",
                    "cannot write repository-local remote URL",
                ));
            }
            (transport, url)
        }
        None => (
            if url.starts_with("https://") {
                Transport::Https
            } else {
                Transport::Ssh
            },
            url,
        ),
    };
    Ok(State {
        remote: remote.to_owned(),
        transport,
        url,
        recorded_in,
    })
}

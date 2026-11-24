use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueHint};
use nils_common::cli_contract::OutputFormat;
use serde_json::json;

use crate::{CliContext, CliError, coordination};

use super::{Refusal, SignedRecord, Verifier, cli_refusal, source, source_fence};

// Destination admission and its abort receipt must ship together.
const MOVE_AVAILABLE: bool = false;
const MAX_RECORD_BYTES: u64 =
    (6 * (super::MAX_PAYLOAD_BYTES + super::MAX_SIGNATURE_BYTES) + 256) as u64;

#[derive(Debug, Args)]
pub struct MoveArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect the exact portable source history without stopping the source.
    Plan {
        #[arg(long)]
        session: String,
        #[arg(long)]
        incarnation: String,
        #[arg(long)]
        destination: String,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Export a stopped source only when the complete move protocol is available.
    Export {
        #[arg(long)]
        session: String,
        #[arg(long)]
        incarnation: String,
        #[arg(long)]
        destination: String,
        #[arg(long, value_hint = ValueHint::FilePath)]
        ticket_file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Release a source move fence using a verified destination receipt.
    Recover {
        #[arg(long)]
        move_id: String,
        #[arg(long, value_hint = ValueHint::FilePath)]
        receipt_file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

impl MoveArgs {
    pub(crate) fn format(&self) -> OutputFormat {
        match self.command {
            Command::Plan { format, .. }
            | Command::Export { format, .. }
            | Command::Recover { format, .. } => format,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self.command {
            Command::Plan { .. } => "resume-move-plan",
            Command::Export { .. } => "resume-move-export",
            Command::Recover { .. } => "resume-move-recover",
        }
    }
}

pub(crate) fn run(context: &CliContext, args: MoveArgs) -> i32 {
    if !MOVE_AVAILABLE && !matches!(args.command, Command::Plan { .. }) {
        return crate::render_error(
            args.name(),
            args.format(),
            crate::start_resume::refusal("resume-move-unavailable"),
        );
    }
    match args.command {
        Command::Plan {
            session,
            incarnation,
            destination,
            format,
        } => {
            const COMMAND: &str = "resume-move-plan";
            match super::source::plan(context, &session, &incarnation, &destination) {
                Ok(plan) => crate::render_single_success(COMMAND, format, &plan, |_| {
                    "source move plan available\n".into()
                }),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
        Command::Export {
            session,
            incarnation,
            destination,
            ticket_file,
            format,
        } => {
            const COMMAND: &str = "resume-move-export";
            let result = (|| {
                let verifier = verifier()?;
                let ticket = read_record(&ticket_file)?;
                let key = configured_path("AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE")?;
                source_fence::export(
                    context,
                    &verifier,
                    source_fence::Export {
                        session: &session,
                        incarnation: &incarnation,
                        source_host: &source::local_host()?,
                        destination: &destination,
                        ticket: &ticket,
                        signing_key: &key,
                    },
                    &crate::resolve_tmux_bin(None),
                )
            })();
            match result {
                Ok(proof) => crate::render_single_success(COMMAND, format, &proof, |_| {
                    "stopped source proof available\n".into()
                }),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
        Command::Recover {
            move_id,
            receipt_file,
            format,
        } => {
            const COMMAND: &str = "resume-move-recover";
            let result = (|| {
                let verifier = verifier()?;
                let receipt = read_record(&receipt_file)?;
                source_fence::recover(
                    context,
                    &verifier,
                    &move_id,
                    &source::local_host()?,
                    &receipt,
                )
            })();
            match result {
                Ok(()) => crate::render_single_success(
                    COMMAND,
                    format,
                    &json!({"move_id": move_id, "source_fence": "released"}),
                    |_| "source fence released\n".into(),
                ),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
    }
}

fn verifier() -> Result<Verifier, CliError> {
    let ssh_keygen = crate::non_empty_env("AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN")
        .map(PathBuf::from)
        .ok_or_else(|| cli_refusal(Refusal::SshKeygenUnavailable))?;
    let allowlist = configured_path("AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE")?;
    Verifier::new(&ssh_keygen, &allowlist).map_err(cli_refusal)
}

fn configured_path(name: &str) -> Result<PathBuf, CliError> {
    crate::non_empty_env(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| cli_refusal(Refusal::TrustUnavailable))
}

fn read_record(path: &Path) -> Result<SignedRecord, CliError> {
    let bytes = coordination::read_private_file(path, MAX_RECORD_BYTES)
        .map_err(|_| cli_refusal(Refusal::ProofInvalid))?;
    serde_json::from_slice(&bytes).map_err(|_| cli_refusal(Refusal::ProofInvalid))
}

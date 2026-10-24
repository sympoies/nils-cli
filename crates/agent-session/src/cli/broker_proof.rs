//! Exact, non-executing broker evidence arguments.
use clap::{Args, ValueHint};
use nils_common::cli_contract::OutputFormat;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct PrepareAdmissionProofArgs {
    /// Owning session; defaults to AGENT_SESSION_ID inside its managed runtime.
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    #[arg(long)]
    pub admit_idempotency_key: String,
    #[arg(long)]
    pub claim: String,
    #[arg(long)]
    pub claim_revision: u64,
    #[arg(long)]
    pub operation: String,
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub targets_file: PathBuf,
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub execution_token_file: PathBuf,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct BrokerProofArgs {
    /// Managed session id whose exact broker evidence is inspected.
    #[arg(long)]
    pub session: String,
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    #[arg(long)]
    pub expected_incarnation: String,
    #[arg(long)]
    pub expected_generation: u64,
    /// Private JSON batch with 1 through 128 item-scoped selectors (at most 64 KiB).
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub batch_file: PathBuf,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

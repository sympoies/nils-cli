use std::path::PathBuf;

use clap::{Args, ValueHint};
use nils_common::cli_contract::OutputFormat;

#[derive(Debug, Args)]
pub struct ServiceMailboxCredential {
    /// Service ID explicitly admitted by the owning daemon.
    #[arg(long)]
    pub service: String,
    /// Exact admitted service generation.
    #[arg(long)]
    pub service_generation: String,
    /// Private service credential reference.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub credential_file: PathBuf,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Args)]
pub struct MessageServiceInboxArgs {
    #[command(flatten)]
    pub credential: ServiceMailboxCredential,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub cursor: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Args)]
pub struct MessageServiceShowArgs {
    #[command(flatten)]
    pub credential: ServiceMailboxCredential,
    #[arg(long)]
    pub message: String,
}

#[derive(Debug, Args)]
pub struct MessageServiceAckArgs {
    #[command(flatten)]
    pub credential: ServiceMailboxCredential,
    #[arg(long)]
    pub message: String,
    #[arg(long)]
    pub if_revision: u64,
    #[arg(long)]
    pub idempotency_key: String,
}

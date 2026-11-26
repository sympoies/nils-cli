use std::path::PathBuf;

use clap::{Args, ValueHint};
use nils_common::cli_contract::OutputFormat;

use super::MessageCategory;

#[derive(Debug, Args)]
pub struct MessageSendArgs {
    /// Authenticated managed session id sending the message.
    #[arg(long = "from")]
    pub from_session: String,
    /// Session destination. Omit for a reply to an authenticated service origin.
    #[arg(long = "to", required_unless_present = "reply_to")]
    pub to_session: Option<String>,
    /// Remote machine; omitted keeps local mailbox behavior.
    #[arg(long, requires = "to_session")]
    pub to_machine: Option<String>,
    /// Require a matching label on a local session destination before enqueueing.
    #[arg(long, value_name = "KEY=VALUE")]
    pub expect_label: Option<String>,
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub body_file: PathBuf,
    /// Private capability file; defaults to AGENT_SESSION_CAPABILITY_FILE.
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    #[arg(long)]
    pub idempotency_key: String,
    #[arg(long)]
    pub reply_to: Option<String>,
    /// Request a reply within 1 second to 7 days (for example 1h).
    #[arg(long, value_name = "DURATION")]
    pub reply_by: Option<String>,
    #[arg(long, value_name = "DURATION")]
    pub expires_in: Option<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
    /// Sender-declared category; omitted means uncategorized.
    #[arg(long, value_enum)]
    pub category: Option<MessageCategory>,
}

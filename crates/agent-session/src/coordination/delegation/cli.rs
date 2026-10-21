use super::{Record, digest_bytes, now_epoch};
use crate::{CliContext, CliError};
use clap::{Args as ClapArgs, Subcommand, ValueEnum, ValueHint};
use nils_common::{
    cli_contract::OutputFormat,
    fs::{SECRET_FILE_MODE, write_atomic},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    StartViaConsole,
    MessageSend,
}
impl Operation {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::StartViaConsole => "start-via-console",
            Self::MessageSend => "message-send",
        }
    }
}

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Mint a private expiring credential for a supervised worker.
    Mint(Mint),
    /// Revoke one delegation owned by the authenticated caller.
    Revoke(Revoke),
}
#[derive(Debug, ClapArgs)]
pub struct Mint {
    /// Caller session that issues and owns the delegation.
    #[arg(long)]
    pub session: String,
    /// Caller's runtime credential; defaults to AGENT_SESSION_CAPABILITY_FILE.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    /// Exact allowed operation (repeatable).
    #[arg(long = "operation", required = true, value_enum)]
    pub operations: Vec<Operation>,
    /// Exact MACHINE/SESSION_ID permitted for message-send (repeatable).
    #[arg(long = "recipient")]
    pub recipients: Vec<String>,
    /// Lifetime in seconds, from 1 to 86400.
    #[arg(long, default_value_t = 3600, value_parser = clap::value_parser!(u64).range(1..=86400))]
    pub expires_in: u64,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
#[derive(Debug, ClapArgs)]
pub struct Revoke {
    /// Caller session that owns the delegation.
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub id: String,
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub capability_file: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
pub(crate) fn format(args: &Args) -> OutputFormat {
    match &args.command {
        Command::Mint(args) => args.format,
        Command::Revoke(args) => args.format,
    }
}
pub(crate) fn name(args: &Args) -> &'static str {
    match &args.command {
        Command::Mint(_) => "delegation-mint",
        Command::Revoke(_) => "delegation-revoke",
    }
}
pub(crate) fn run(context: &CliContext, args: Args) -> i32 {
    let name = name(&args);
    let format = format(&args);
    let result = match args.command {
        Command::Mint(args) => mint(context, args),
        Command::Revoke(args) => revoke(context, args),
    };
    super::super::render_coordination(name, format, result)
}
fn invalid() -> CliError {
    CliError::usage(
        "delegation-invalid",
        "message-send requires exact MACHINE/SESSION_ID recipients; other operations do not accept recipients",
        None,
    )
}

pub(super) fn mint(context: &CliContext, mut args: Mint) -> Result<Value, CliError> {
    let token = super::super::capability_token_from_file(args.capability_file.as_deref())?;
    let (caller, incarnation) = super::super::authenticate_token(context, &args.session, &token)?;
    if args.operations.contains(&Operation::MessageSend) == args.recipients.is_empty()
        || args.recipients.len() > 128
        || args.recipients.iter().any(|recipient| {
            let Some((machine, session)) = recipient.split_once('/') else {
                return true;
            };
            machine.is_empty()
                || session.is_empty()
                || recipient.len() > 512
                || recipient
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || c == '*')
                || session.contains('/')
        })
    {
        return Err(invalid());
    }
    args.recipients.sort();
    args.recipients.dedup();
    let _session_lock = crate::acquire_session_record_lock(context, &caller.id)?;
    let mut locked = super::super::lock_registry(context)?;
    super::revalidate(
        context,
        &locked.registry,
        &caller.id,
        &incarnation,
        &token,
        super::Use::StartViaConsole,
    )?;
    let now = now_epoch();
    locked
        .registry
        .delegations
        .retain(|_, delegation| delegation.expires_at_epoch > now.saturating_sub(86400));
    if locked.registry.delegations.len() >= 4096
        || locked
            .registry
            .delegations
            .values()
            .filter(|record| {
                record.session_id == caller.id && !record.revoked && record.expires_at_epoch > now
            })
            .count()
            >= 128
    {
        return Err(CliError::data(
            "delegation-quota-exceeded",
            "the private delegation registry is at capacity",
            None,
        ));
    }
    let id = uuid::Uuid::new_v4().to_string();
    let credential = format!(
        "{}{id}.{}{}",
        super::PREFIX,
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let path =
        super::super::coordination_root(context)?.join(format!("delegation-{id}.credential"));
    let expires = now.saturating_add(args.expires_in as i64);
    let record = Record {
        id: id.clone(),
        session_id: caller.id.clone(),
        incarnation: incarnation.clone(),
        issuer_machine: crate::board::machine_identity(None, context),
        capability_digest: digest_bytes(token.as_bytes()),
        token_digest: digest_bytes(credential.as_bytes()),
        operations: args.operations,
        recipients: args.recipients,
        expires_at_epoch: expires,
        revoked: false,
    };
    write_atomic(&path, credential.as_bytes(), SECRET_FILE_MODE)
        .map_err(|_| super::super::store_unavailable())?;
    locked.registry.delegations.insert(id.clone(), record);
    locked.save()?;
    Ok(
        json!({"schema_version":"agent-session.delegation.v1", "id":id, "session_id":caller.id, "session_incarnation":incarnation, "credential_file":path, "expires_at_epoch":expires}),
    )
}
pub(super) fn revoke(context: &CliContext, args: Revoke) -> Result<Value, CliError> {
    let token = super::super::capability_token_from_file(args.capability_file.as_deref())?;
    let (caller, incarnation) = super::super::authenticate_token(context, &args.session, &token)?;
    let _session_lock = crate::acquire_session_record_lock(context, &caller.id)?;
    let mut locked = super::super::lock_registry(context)?;
    super::revalidate(
        context,
        &locked.registry,
        &caller.id,
        &incarnation,
        &token,
        super::Use::StartViaConsole,
    )?;
    let record = locked
        .registry
        .delegations
        .get_mut(&args.id)
        .filter(|record| record.session_id == caller.id && record.incarnation == incarnation)
        .ok_or_else(|| super::error("delegation-unauthorized"))?;
    record.revoked = true;
    locked.save()?;
    Ok(
        json!({"schema_version":"agent-session.delegation-revocation.v1", "id":args.id, "revoked":true}),
    )
}

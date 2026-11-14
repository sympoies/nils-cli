//! Explicit owner answers to one live Claude PermissionRequest hook.
mod transport;
mod version;
use crate::{CliContext, CliError, SessionRecord, activity};
use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use nils_common::cli_contract::OutputFormat;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List live, exactly correlated requests without tool input.
    List(List),
    /// Answer one request; both identifiers must match the live hook.
    Answer(Answer),
    /// Internal synchronous relay, invoked only by the managed hook dispatcher.
    #[command(hide = true)]
    Wait(Wait),
}
#[derive(Debug, ClapArgs)]
pub struct List {
    pub id: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
#[derive(Debug, ClapArgs)]
pub struct Answer {
    pub id: String,
    #[command(flatten)]
    pub request: AnswerRequest,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}
#[derive(Clone, Debug, ClapArgs, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    /// Fresh request UUID returned by permission list.
    #[arg(long)]
    pub request_id: String,
    /// Exact call identifier returned by permission list.
    #[arg(long)]
    pub call_id: String,
    /// Allow or deny this request once without saving permission rules.
    #[arg(long, value_enum)]
    pub decision: Decision,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Deny,
}
#[derive(Debug, ClapArgs)]
pub struct Wait {
    pub id: String,
    #[arg(long)]
    pub runtime_id: String,
    #[arg(long)]
    pub attention_id: String,
}

pub(crate) fn refusal(code: &str) -> CliError {
    CliError::runtime(
        code,
        "Claude permission request cannot be answered with these selectors",
        None,
    )
}

/// Short preflight; immediate pane writes retain the returned admission guard.
pub(crate) fn refuse_terminal(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    terminal_admission(context, record).map(drop)
}

/// Caller holds the record lock. Never retain this across hook acknowledgement
/// waits: provider ingress needs to proceed after the immediate terminal write.
pub(crate) fn terminal_admission(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Option<std::fs::File>, CliError> {
    if record.agent != "claude" {
        return Ok(None);
    }
    let ingress = activity::pending::probe_lock(context, &record.id)?
        .ok_or_else(|| refusal("permission-event-pending"))?;
    if activity::claude_attention::has_permission(context, record)? {
        return Err(refusal("claude-permission-hook-required"));
    }
    Ok(Some(ingress))
}

pub(crate) fn list(context: &CliContext, id: &str) -> Result<Value, CliError> {
    transport::list(context, id).map(|requests| json!({"requests": requests}))
}

pub(crate) fn answer(
    context: &CliContext,
    id: &str,
    request: AnswerRequest,
) -> Result<Value, CliError> {
    transport::answer(context, id, request)
}

pub(crate) fn run(context: &CliContext, args: Args) -> i32 {
    let (command, format, result) = match args.command {
        Command::List(args) => ("permission.list", args.format, list(context, &args.id)),
        Command::Answer(args) => (
            "permission.answer",
            args.format,
            answer(context, &args.id, args.request),
        ),
        Command::Wait(args) => {
            return match transport::wait(context, args) {
                Ok(()) => 0,
                Err(error) => {
                    eprintln!("agent-session: {}", error.code());
                    1
                }
            };
        }
    };
    match result {
        Ok(value) => crate::render_single_success(command, format, &value, |v| format!("{v}\n")),
        Err(error) => crate::render_error(command, format, error),
    }
}

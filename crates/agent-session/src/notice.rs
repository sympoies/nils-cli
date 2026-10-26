//! Versioned display-only session notices and guarded CLI delivery.
mod envelope;
pub use envelope::{NoticeEnvelope, NoticeError, ParsedNotice, parse_notice};

use std::path::PathBuf;

use clap::{Args, ValueHint};
use nils_common::cli_contract::OutputFormat;

use crate::{CliContext, CliError, cli, send_guard};

/// Explicit rollout gate; unset, empty and all values except `1` keep writers off.
pub const WRITERS_ENV: &str = "AGENT_SESSION_NOTICE_WRITERS";

#[derive(Debug, Args)]
pub struct NoticeArgs {
    /// Session id or unambiguous prefix.
    pub session: String,
    /// Generic event kind, for example schedule.tick.
    #[arg(long)]
    pub kind: String,
    /// Self-asserted producer label; never a verified identity.
    #[arg(long)]
    pub source: String,
    /// Display identifier; independent of the transport idempotency key.
    #[arg(long)]
    pub id: Option<String>,
    /// UTF-8 body file; bytes are retained exactly.
    #[arg(
        long,
        required_unless_present = "text_stdin",
        conflicts_with = "text_stdin",
        value_hint = ValueHint::FilePath
    )]
    pub body_file: Option<PathBuf>,
    #[arg(long, required_unless_present = "body_file")]
    pub text_stdin: bool,
    #[command(flatten)]
    pub guards: send_guard::GuardArgs,
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub tmux_bin: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub format: OutputFormat,
}

#[derive(Debug, Default, Args)]
pub struct StartNoticeArgs {
    /// Mark this initial prompt as a notice (requires the notice writer rollout gate).
    #[arg(long, requires = "notice_source")]
    pub notice_kind: Option<String>,
    #[arg(long, requires = "notice_kind")]
    pub notice_source: Option<String>,
    #[arg(long, requires = "notice_kind")]
    pub notice_id: Option<String>,
}

fn error(error: NoticeError) -> CliError {
    let code = match error {
        NoticeError::InvalidField(_) => "invalid-notice-field",
        NoticeError::EmptyBody => "empty-notice-body",
        NoticeError::DoubleHeader => "double-notice-header",
    };
    CliError::usage(code, error.to_string(), None)
}

fn require_enabled() -> Result<(), CliError> {
    if std::env::var(WRITERS_ENV).as_deref() == Ok("1") {
        return Ok(());
    }
    Err(CliError::runtime(
        "notice-writers-disabled",
        "notice writers are disabled; deploy compatible readers before setting AGENT_SESSION_NOTICE_WRITERS=1",
        None,
    ))
}

pub(crate) fn prepare_start(args: &mut cli::StartArgs) -> Result<(), CliError> {
    let Some(kind) = args.notice.notice_kind.as_deref() else {
        return Ok(());
    };
    let metadata = NoticeEnvelope::new(
        kind,
        args.notice.notice_source.as_deref().unwrap_or(""),
        args.notice.notice_id.as_deref(),
    )
    .map_err(error)?;
    require_enabled()?;
    let body = crate::read_prompt(&args.prompt, args.prompt_file.as_deref(), args.prompt_stdin)?;
    args.prompt = Some(
        metadata
            .encode(body.as_deref().unwrap_or(""))
            .map_err(error)?,
    );
    args.prompt_file = None;
    args.prompt_stdin = false;
    Ok(())
}

pub(crate) fn run(context: &CliContext, mut args: NoticeArgs) -> i32 {
    let format = args.format;
    let result = (|| {
        // Validate before reading stdin/body files, loading a record or claiming a key.
        let metadata =
            NoticeEnvelope::new(&args.kind, &args.source, args.id.as_deref()).map_err(error)?;
        require_enabled()?;
        let body = crate::read_prompt(&None, args.body_file.as_deref(), args.text_stdin)?;
        let text = metadata
            .encode(body.as_deref().unwrap_or(""))
            .map_err(error)?;
        let record = crate::load_session_record(context, &args.session)?;
        if !crate::codex_app_server::runtime_is_supported(&record) {
            return Err(CliError::runtime(
                "notice-transport-unsupported",
                "notice requires a supported Codex app-server runtime; terminal exact multiline verification is unavailable",
                Some(serde_json::json!({"pasted":false,"submitted":false,"stage":"admission"})),
            ));
        }
        args.guards.require_idle = true;
        args.guards.require_empty_composer = true;
        crate::send::send_to_session(
            context,
            cli::SendArgs {
                id: args.session,
                text: Some(text),
                text_stdin: false,
                keys: vec![cli::SpecialKey::Enter],
                allow_blocked: false,
                guards: args.guards,
                tmux_bin: args.tmux_bin,
                format,
            },
        )
    })();
    match result {
        Ok(result) => {
            crate::render_single_success("notice", format, &result, crate::render_send_text)
        }
        Err(error) => crate::render_error("notice", format, error),
    }
}

#[cfg(test)]
mod tests;

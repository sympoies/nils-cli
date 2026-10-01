//! Provider account switching for one session, shared by serve's
//! `PUT /sessions/{id}/account` and the `agent-session account` CLI so the two
//! surfaces cannot drift.

use std::path::Path;

use serde::Serialize;
use serde_json::{Value, json};

use crate::claude_account::ClaudeAccountView;
use crate::cli::{AccountArgs, AccountCommand};
use crate::codex_account::CodexAccountView;
use crate::{
    AgentKind, CliContext, CliError, SessionRecord, load_session_record, render_error,
    render_single_success, resolve_tmux_bin, write_session_record,
};

const SHOW_COMMAND: &str = "account-show";
const SWITCH_COMMAND: &str = "account-switch";

/// One session's provider account projection and the runtime it is fenced to.
#[derive(Debug, Serialize)]
pub(crate) struct SessionAccount {
    id: String,
    agent: String,
    session_incarnation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    codex_account: Option<CodexAccountView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    claude_account: Option<ClaudeAccountView>,
}

pub(crate) fn run_account(context: &CliContext, args: AccountArgs) -> i32 {
    match args.command {
        AccountCommand::Show(args) => match show(context, &args.id) {
            Ok(result) => render_single_success(SHOW_COMMAND, args.format, &result, render_text),
            Err(err) => render_error(SHOW_COMMAND, args.format, err),
        },
        AccountCommand::Switch(args) => {
            let format = args.format;
            let tmux_bin = resolve_tmux_bin(args.tmux_bin.as_deref());
            match switch(
                context,
                &args.id,
                &args.account,
                args.expected_incarnation.as_deref(),
                &tmux_bin,
            ) {
                Ok(result) => render_single_success(SWITCH_COMMAND, format, &result, render_text),
                Err(err) => render_error(SWITCH_COMMAND, format, err),
            }
        }
    }
}

/// The current account and any queued next account for `id`.
pub(crate) fn show(context: &CliContext, id: &str) -> Result<SessionAccount, CliError> {
    let record = load_session_record(context, id)?;
    let session_incarnation = launch_id(&record);
    Ok(session_account(&record, session_incarnation))
}

/// Switch `id` to `account`, fenced on `expected_incarnation` or, when omitted,
/// on the current runtime's launch id.
///
/// Claude takes the serve route's exact path. Codex binds live only through
/// the daemon's in-process control connection, so the CLI takes the serve
/// route's durable queue path: the account is bound for the next prompt, which
/// stays fenced until the daemon applies it at the idle boundary.
pub(crate) fn switch(
    context: &CliContext,
    id: &str,
    account: &str,
    expected_incarnation: Option<&str>,
    tmux_bin: &Path,
) -> Result<SessionAccount, CliError> {
    let record = load_session_record(context, id)?;
    let current = launch_id(&record);
    let expected = expected_incarnation
        .map(str::to_string)
        .or(current)
        .unwrap_or_default();
    if record.agent == AgentKind::Claude.as_str() {
        let (session_incarnation, view) =
            claude_switch_locked(context, &record.id, account, &expected, tmux_bin)?;
        let mut result = session_account(&record, session_incarnation);
        result.claude_account = view;
        return Ok(result);
    }
    let launch_id = codex_switch_precheck(&record, &expected, account)?;
    let view = codex_queue_switch(context, &record.id, &launch_id, account, true)?;
    let mut result = session_account(&record, Some(launch_id));
    result.codex_account = Some(view);
    Ok(result)
}

/// Claude has no live credential swap, so a switch is a durable next-account
/// intent applied by a restart plus `--resume` in the new account directory.
/// The restart happens now only when the running session is idle; otherwise
/// (busy, stopped, or unknown) the intent stays queued for the next resume.
pub(crate) fn claude_switch_locked(
    context: &CliContext,
    id: &str,
    account: &str,
    expected_session_incarnation: &str,
    tmux_bin: &Path,
) -> Result<(Option<String>, Option<ClaudeAccountView>), CliError> {
    let _record_lock = crate::acquire_session_record_lock(context, id)?;
    let mut record = load_session_record(context, id)?;
    let launch_id = launch_id(&record);
    if launch_id.as_deref() != Some(expected_session_incarnation) {
        return Err(CliError::data(
            "claude-account-session-incarnation-conflict",
            "session was replaced before its Claude account switch was applied",
            Some(json!({ "id": id })),
        ));
    }
    crate::claude_account::queue_next(&mut record, account)?;
    let idle = crate::claude_account::has_queued_next(&record)
        && crate::session_status(context, tmux_bin, &record) == "running"
        && crate::activity::state_for_view(context, &record)
            .is_some_and(|activity| activity.phase == crate::activity::TurnPhase::Waiting);
    if idle {
        // Prepare the new account before anything is stopped. A refusal
        // returns before the intent is written, so durable state (including
        // any previously queued intent) is left exactly as it was.
        crate::claude_account::preflight_next(&record)?;
    }
    let now = jiff::Timestamp::now().to_string();
    record.updated_at = now.clone();
    write_session_record(context, &record)?;
    if !idle {
        return Ok((launch_id, crate::claude_account::view_for_record(&record)));
    }
    crate::auto_resume::cancel_for_account_switch_locked(context, id, &now)?;
    crate::stop_session_runtime_locked(context, &mut record, tmux_bin)?;
    let stopped = load_session_record(context, id)?;
    let outcome = crate::resume_session_locked(context, stopped, tmux_bin)?;
    let resumed = load_session_record(context, id)?;
    Ok((
        outcome.session_incarnation,
        crate::claude_account::view_for_record(&resumed),
    ))
}

/// Checks every Codex switch passes before any binding or queued intent is
/// written: support, the exact runtime incarnation, and a broker-listed
/// account. Returns the runtime launch id.
pub(crate) fn codex_switch_precheck(
    record: &SessionRecord,
    expected_session_incarnation: &str,
    account: &str,
) -> Result<String, CliError> {
    let unsupported = || {
        CliError::data(
            "codex-account-unsupported",
            "this session does not support Codex account switching",
            None,
        )
    };
    if !crate::codex_account::view_for_record(record).supported {
        return Err(unsupported());
    }
    let launch_id = launch_id(record).ok_or_else(unsupported)?;
    if launch_id != expected_session_incarnation {
        return Err(CliError::data(
            "codex-account-session-incarnation-conflict",
            "session was replaced before its Codex account switch was applied",
            None,
        ));
    }
    // Like Claude, refuse an account the broker does not list before any
    // binding or queued intent is written. Re-selecting the current account
    // stays allowed so it can still cancel a queued switch.
    if crate::codex_account::selected_account(record).as_deref() != Some(account) {
        crate::codex_account::ensure_listed(account)?;
    }
    Ok(launch_id)
}

/// Durably queue `account` for the next prompt; the daemon applies it at the
/// idle boundary and any applied binding is left unchanged until then.
pub(crate) fn codex_queue_switch(
    context: &CliContext,
    id: &str,
    launch_id: &str,
    account: &str,
    supports_unbound_account_queue: bool,
) -> Result<CodexAccountView, CliError> {
    if supports_unbound_account_queue {
        crate::codex_account::queue_next_account_with_unbound(context, id, launch_id, account)
    } else {
        crate::codex_account::queue_next_account(context, id, launch_id, account)
    }
}

fn launch_id(record: &SessionRecord) -> Option<String> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
}

fn session_account(record: &SessionRecord, session_incarnation: Option<String>) -> SessionAccount {
    SessionAccount {
        id: record.id.clone(),
        agent: record.agent.clone(),
        session_incarnation,
        codex_account: (record.agent == AgentKind::Codex.as_str())
            .then(|| crate::codex_account::view_for_record(record)),
        claude_account: crate::claude_account::view_for_record(record),
    }
}

fn render_text(result: &SessionAccount) -> String {
    let mut text = format!("session {} ({})\n", result.id, result.agent);
    if let Some(incarnation) = &result.session_incarnation {
        text.push_str(&format!("incarnation: {incarnation}\n"));
    }
    let view = result
        .codex_account
        .as_ref()
        .and_then(|view| serde_json::to_value(view).ok())
        .or_else(|| {
            result
                .claude_account
                .as_ref()
                .and_then(|view| serde_json::to_value(view).ok())
        });
    let Some(view) = view else {
        text.push_str("account: unsupported\n");
        return text;
    };
    let field =
        |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_string);
    let state = field(&view, "state").unwrap_or_default();
    match field(&view, "selected_account") {
        Some(account) => text.push_str(&format!("account: {account} ({state})\n")),
        None => text.push_str(&format!("account: none ({state})\n")),
    }
    if let Some(next) = view.get("next") {
        let account = field(next, "account").unwrap_or_else(|| "unknown".to_string());
        let state = field(next, "state").unwrap_or_default();
        text.push_str(&format!("next: {account} ({state})\n"));
    }
    text
}

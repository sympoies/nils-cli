//! In-place restart of a managed session onto the installed provider binary.
//!
//! A restart keeps the session id, provider conversation, tmux runtime name,
//! mailbox and routing: it stops the provider at an idle safe point, retires
//! the stopped coordination incarnation, and resumes from the exact provider
//! resume metadata, so the relaunch picks up whatever binary is installed now.

use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use serde::Serialize;
use serde_json::json;

use crate::cli::{AgentKind, RestartArgs, SpecialKey};
use crate::send_guard::{Admission, GuardArgs};
use crate::{CliContext, CliError, SessionRecord, coordination, load_session_record};

const COMMAND: &str = "restart";
const SCHEMA_VERSION: &str = "agent-session.restart.v1";
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// A stopped incarnation's last heartbeat stays fresh for up to 30 seconds.
const PRIOR_LEASE_WAIT: Duration = Duration::from_secs(35);
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The provider version a restart observed for the runtime it launched.
const PROVIDER_VERSION_KEY: &str = "restart_provider_version";

#[derive(Debug, Default, Serialize)]
struct Incarnation {
    session_incarnation: Option<String>,
    generation: Option<u64>,
    pane_pid: Option<i64>,
    provider_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct Receipt {
    schema_version: &'static str,
    id: String,
    agent: String,
    tmux_session: String,
    provider_session_id: String,
    outcome: &'static str,
    stop_method: Option<&'static str>,
    old: Incarnation,
    new: Incarnation,
    started_at: String,
    stopped_at: Option<String>,
    relaunched_at: Option<String>,
    ready_at: Option<String>,
    finished_at: String,
}

pub(crate) fn run(context: &CliContext, args: RestartArgs) -> i32 {
    let format = args.format;
    match restart(context, &args) {
        Ok(receipt) => crate::render_single_success(COMMAND, format, &receipt, render_text),
        Err(err) => crate::render_error(COMMAND, format, err),
    }
}

fn render_text(receipt: &Receipt) -> String {
    let unknown = || "unknown".to_string();
    format!(
        "{}: restarted {} -> {} (provider {})\n",
        receipt.id,
        receipt
            .old
            .session_incarnation
            .clone()
            .unwrap_or_else(unknown),
        receipt
            .new
            .session_incarnation
            .clone()
            .unwrap_or_else(unknown),
        receipt.new.provider_version.clone().unwrap_or_else(unknown),
    )
}

/// The provider binary a resume of `record` launches.
pub(crate) fn resume_agent_bin(
    profile: Option<crate::DurableProfileResumeContext>,
    record: &SessionRecord,
    agent: AgentKind,
) -> PathBuf {
    profile
        .map(|profile| profile.agent_bin)
        .or_else(|| record.agent_bin.as_deref().map(PathBuf::from))
        .unwrap_or_else(|| crate::resolve_agent_bin(agent, None))
}

fn now() -> String {
    Timestamp::now().to_string()
}

fn launch_id(record: &SessionRecord) -> Option<String> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .filter(|launch_id| !launch_id.is_empty())
}

fn incarnation(record: &SessionRecord) -> Incarnation {
    let session_incarnation = launch_id(record);
    let provider_version = record
        .extra
        .get(PROVIDER_VERSION_KEY)
        .filter(|recorded| {
            session_incarnation.is_some()
                && recorded["launch_id"].as_str() == session_incarnation.as_deref()
        })
        .and_then(|recorded| recorded["version"].as_str())
        .map(str::to_string);
    Incarnation {
        session_incarnation,
        generation: record.runtime.as_ref().map(|runtime| runtime.generation),
        pane_pid: crate::persisted_tmux_runtime_identity(record)
            .ok()
            .flatten()
            .map(|identity| i64::from(identity.pane_pid)),
        provider_version,
    }
}

fn provider_version(agent_bin: &Path) -> Option<String> {
    let mut command = ProcessCommand::new(agent_bin);
    command.arg("--version").stdin(Stdio::null());
    let output = crate::run_output_with_timeout(command, VERSION_PROBE_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

fn refusal(code: &'static str, message: &'static str, record: &SessionRecord) -> CliError {
    CliError::runtime(code, message, Some(json!({ "id": record.id })))
}

/// A failure after the provider was stopped: the receipt says how far the
/// restart got and the printed command recovers the session.
fn failure(
    code: &'static str,
    message: &'static str,
    mut receipt: Receipt,
    outcome: &'static str,
    cause: Option<&str>,
    recovery: String,
) -> CliError {
    receipt.outcome = outcome;
    receipt.finished_at = now();
    let hint = format!("run `{recovery}` to recover the session");
    CliError::runtime(
        code,
        message,
        Some(json!({
            "id": receipt.id,
            "cause": cause,
            "recovery": recovery,
            "receipt": receipt,
        })),
    )
    .with_hint(hint)
}

fn restart(context: &CliContext, args: &RestartArgs) -> Result<Receipt, CliError> {
    let started_at = now();
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    let expected = load_session_record(context, &args.id)?;
    let (provider_resume, agent) = crate::validate_resume_metadata(&expected)?;
    let provider_session_id = provider_resume.session_id.clone();
    // Stopping the runtime that hosts this very process would kill the
    // restart between its stop and its relaunch.
    if std::env::var("AGENT_SESSION_ID").is_ok_and(|id| id == expected.id) {
        return Err(refusal(
            "restart-own-session",
            "a session cannot restart the runtime that hosts this command",
            &expected,
        ));
    }
    let old = incarnation(&expected);
    let Some(old_launch) = old.session_incarnation.clone() else {
        return Err(refusal(
            "session-not-running",
            "restart requires a running session; use resume for a stopped one",
            &expected,
        ));
    };
    let exit_requested =
        request_provider_exit(context, &expected, agent, &old_launch, &tmux, args.force)?;
    let mut receipt = Receipt {
        schema_version: SCHEMA_VERSION,
        id: expected.id.clone(),
        agent: expected.agent.clone(),
        tmux_session: expected.tmux_session.clone(),
        provider_session_id,
        outcome: "ready",
        stop_method: None,
        old,
        new: Incarnation::default(),
        started_at,
        stopped_at: None,
        relaunched_at: None,
        ready_at: None,
        finished_at: String::new(),
    };
    let stop_deadline = Instant::now() + Duration::from_secs(args.stop_timeout);
    while exit_requested
        && crate::session_status(context, &tmux, &expected) != "stopped"
        && Instant::now() < stop_deadline
    {
        thread::sleep(POLL_INTERVAL);
    }

    let record_lock = crate::acquire_session_record_lock(context, &expected.id)?;
    let mut record = load_session_record(context, &expected.id)?;
    crate::ensure_same_session_identity(&expected, &record)?;
    if launch_id(&record).as_deref() != Some(old_launch.as_str()) {
        return Err(refusal(
            "restart-superseded",
            "another operation relaunched the session during the restart",
            &record,
        ));
    }
    let exited = crate::session_status(context, &tmux, &record) == "stopped";
    crate::auto_resume::cancel_for_manual_input_locked(context, &record.id, &now())?;
    crate::stop_session_runtime_locked(context, &mut record, &tmux)?;
    receipt.stop_method = Some(if exit_requested && exited {
        "provider-exit"
    } else {
        "terminated"
    });
    receipt.stopped_at = Some(now());
    let recovery = format!("agent-session resume {}", record.id);
    if let Err(cause) = coordination::retire_after_verified_stop(context, &record, &tmux)
        .and_then(|()| await_prior_lease(context, &record))
    {
        return Err(failure(
            "restart-prior-incarnation-live",
            "the session stopped but its prior coordination incarnation was not released",
            receipt,
            "relaunch-failed",
            Some(cause.code()),
            recovery,
        ));
    }
    receipt.relaunched_at = Some(now());
    let resumed = match crate::resume_session_locked(context, record, &tmux) {
        Ok(resumed) => resumed,
        Err(cause) => {
            return Err(failure(
                "restart-relaunch-failed",
                "the session stopped but its provider could not be relaunched",
                receipt,
                "relaunch-failed",
                Some(cause.code()),
                recovery,
            ));
        }
    };
    let new_launch = resumed.session_incarnation.unwrap_or_default();
    let mut relaunched = load_session_record(context, &expected.id)?;
    let profile = crate::validate_durable_profile_resume_context(&relaunched)?;
    let version = provider_version(&resume_agent_bin(profile, &relaunched, agent));
    relaunched.extra.insert(
        PROVIDER_VERSION_KEY.to_string(),
        json!({ "launch_id": new_launch, "version": version }),
    );
    crate::write_session_record(context, &relaunched)?;
    receipt.new = incarnation(&relaunched);
    drop(record_lock);

    let ready_deadline = Instant::now() + Duration::from_secs(args.ready_timeout);
    match await_ready(context, &relaunched, &new_launch, &tmux, ready_deadline) {
        Ok(()) => {
            receipt.ready_at = Some(now());
            receipt.finished_at = now();
            Ok(receipt)
        }
        Err(cause) => {
            let exited = cause == "provider-exited";
            let recovery = if exited {
                format!("agent-session resume {}", expected.id)
            } else {
                format!("agent-session glance {}", expected.id)
            };
            Err(failure(
                "restart-not-ready",
                "the session was relaunched but its provider did not reach a ready prompt in time",
                receipt,
                if exited {
                    "relaunch-exited"
                } else {
                    "not-ready"
                },
                Some(&cause),
                recovery,
            ))
        }
    }
}

/// Type the provider's own exit command into a verified idle, empty composer.
/// Returns whether it was sent. Without `force` an unverified session is
/// refused before anything is stopped; with it, the runtime is terminated.
fn request_provider_exit(
    context: &CliContext,
    expected: &SessionRecord,
    agent: AgentKind,
    launch: &str,
    tmux: &Path,
    force: bool,
) -> Result<bool, CliError> {
    let _record_lock = crate::acquire_session_record_lock(context, &expected.id)?;
    let mut current = load_session_record(context, &expected.id)?;
    crate::ensure_same_session_identity(expected, &current)?;
    if crate::session_status(context, tmux, &current) != "running" {
        return Err(refusal(
            "session-not-running",
            "restart requires a running session; use resume for a stopped one",
            &current,
        ));
    }
    let _activity_lock = crate::activity::acquire_coordination_activity_lock(context, &current.id)?;
    let guards = GuardArgs {
        expect_runtime: Some(launch.to_string()),
        require_idle: true,
        require_empty_composer: true,
        ..GuardArgs::default()
    };
    let admission = Admission {
        context,
        record: &current,
        args: &guards,
        tmux_bin: tmux,
        verify_composer: false,
        deadline: Instant::now() + crate::send_submit::HOLD_BUDGET,
    };
    if let Err(cause) = admission.check(None, "admission") {
        if force {
            return Ok(false);
        }
        return Err(CliError::runtime(
            "restart-session-not-idle",
            "restart requires a verified idle session with an empty composer",
            Some(json!({ "id": current.id, "cause": cause.code() })),
        )
        .with_hint("retry once the session is idle, or pass --force to stop it now"));
    }
    // Admission already refuses every provider without an observable prompt.
    let command = match agent {
        AgentKind::Claude => "/exit",
        AgentKind::Codex => "/quit",
        _ => return Ok(false),
    };
    crate::codex_account::authorize_terminal_input_locked(context, &mut current)?;
    crate::auto_resume::cancel_for_manual_input_locked(context, &current.id, &now())?;
    crate::send_input_unlocked(
        context,
        &current,
        Some(command),
        &[SpecialKey::Enter],
        tmux,
        None,
        crate::PasteMode::Raw,
    )?;
    Ok(true)
}

/// Bounded wait for the stopped incarnation's lease. Retiring it normally
/// releases it at once; a heartbeat that cannot be proven stale is waited out
/// instead of refusing the relaunch as a live prior incarnation.
fn await_prior_lease(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let incarnation = coordination::incarnation(record)?;
    let deadline = Instant::now() + PRIOR_LEASE_WAIT;
    while coordination::broker::heartbeat_fresh(context, &record.id, &incarnation, 0) {
        if Instant::now() >= deadline {
            return Err(refusal(
                "session-incarnation-conflict",
                "the prior coordination incarnation is still live",
                record,
            ));
        }
        thread::sleep(POLL_INTERVAL);
    }
    Ok(())
}

/// Bounded wait until the relaunched provider reports an idle prompt through
/// the same admission a guarded idle send uses. Returns the last refusal code.
fn await_ready(
    context: &CliContext,
    record: &SessionRecord,
    launch: &str,
    tmux: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let guards = GuardArgs {
        expect_runtime: Some(launch.to_string()),
        require_idle: true,
        ..GuardArgs::default()
    };
    loop {
        if crate::session_status(context, tmux, record) == "stopped" {
            return Err("provider-exited".to_string());
        }
        let admission = Admission {
            context,
            record,
            args: &guards,
            tmux_bin: tmux,
            verify_composer: false,
            deadline: Instant::now() + crate::send_submit::HOLD_BUDGET,
        };
        let cause = match admission.check(None, "readiness") {
            Ok(()) => return Ok(()),
            Err(cause) => cause.code().to_string(),
        };
        if Instant::now() >= deadline {
            return Err(cause);
        }
        thread::sleep(POLL_INTERVAL);
    }
}

#[cfg(test)]
#[path = "restart_tests.rs"]
mod tests;

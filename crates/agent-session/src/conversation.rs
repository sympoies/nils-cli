//! Provider-confirmed conversation transitions, independent of managed runtime identity.
mod clear;
pub(crate) use clear::{ensure_input_available, input_fenced};

use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    CliContext, CliError, ProviderResume, SessionRecord, acquire_session_record_lock, activity,
    canonical_provider_resume_args,
    cli::{AgentKind, ClearOptions, ConversationArgs, SpecialKey},
    codex_app_server, coordination, ensure_same_session_identity, load_session_record, send_guard,
    session_dir, write_private_file, write_session_record,
};

const LIVE_FILE: &str = "provider-conversation.json";

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LiveConversation {
    schema_version: String,
    runtime_id: String,
    runtime_generation: u64,
    provider: String,
    session_id: String,
}

#[derive(Debug, Serialize)]
struct ResultView {
    id: String,
    provider: String,
    old_provider_session_id: Option<String>,
    new_provider_session_id: String,
    changed: bool,
    support: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt: Option<clear::Receipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    receipt_path: Option<String>,
}

fn error(code: &'static str, message: &'static str, record: &SessionRecord) -> CliError {
    CliError::runtime(code, message, Some(json!({"id": record.id})))
}

fn provider(record: &SessionRecord) -> Result<AgentKind, CliError> {
    match AgentKind::from_name(&record.agent) {
        Some(agent @ (AgentKind::Codex | AgentKind::Claude)) if record.mode == "interactive" => {
            Ok(agent)
        }
        _ => Err(error(
            "conversation-provider-unsupported",
            "conversation clearing and rebinding require interactive Codex or Claude Code",
            record,
        )),
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && !id.chars().any(char::is_control)
        && !id.starts_with("local:v1:")
}

fn live(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<Option<LiveConversation>, CliError> {
    let mut bytes = Vec::new();
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(session_dir(context, &record.id).join(LIVE_FILE))
    {
        Ok(file) => {
            if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
                return Err(error(
                    "conversation-observation-invalid",
                    "live conversation observation is invalid",
                    record,
                ));
            }
            file.take(16 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| {
                    error(
                        "conversation-observation-unavailable",
                        "live conversation observation is unreadable",
                        record,
                    )
                })?;
            if bytes.len() > 16 * 1024 {
                return Err(error(
                    "conversation-observation-invalid",
                    "live conversation observation is invalid",
                    record,
                ));
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(error(
                "conversation-observation-unavailable",
                "live conversation observation is unreadable",
                record,
            ));
        }
    };
    let value: LiveConversation = serde_json::from_slice(&bytes).map_err(|_| {
        error(
            "conversation-observation-invalid",
            "live conversation observation is invalid",
            record,
        )
    })?;
    let runtime = record.runtime.as_ref().ok_or_else(|| {
        error(
            "conversation-runtime-missing",
            "session has no live runtime identity",
            record,
        )
    })?;
    if value.schema_version != "agent-session.provider-conversation.v1"
        || value.provider != record.agent
        || !valid_id(&value.session_id)
    {
        return Err(error(
            "conversation-observation-mismatch",
            "live conversation observation does not match this runtime",
            record,
        ));
    }
    if value.runtime_id != runtime.launch_id || value.runtime_generation != runtime.generation {
        // A stopped runtime's receipt cannot authorize this runtime, but must
        // not prevent it from observing its own future native transition.
        return Ok(None);
    }
    Ok(Some(value))
}

/// Read-only consistency check; a metrics GET cannot repair a pending native transition.
pub(crate) fn binding_is_current(context: &CliContext, record: &SessionRecord) -> bool {
    live(context, record).is_ok_and(|live| {
        live.is_none_or(|observed| {
            record.provider_resume.as_ref().is_some_and(|resume| {
                resume.provider == observed.provider && resume.session_id == observed.session_id
            })
        })
    })
}

/// Called only for a primary native thread/start response or Claude SessionStart(clear).
/// Retain the observation before mutation so a failed persistence has a supported recovery path.
pub(crate) fn observe_native(
    context: &CliContext,
    expected: &SessionRecord,
    session_id: &str,
) -> Result<(), CliError> {
    provider(expected)?;
    if !valid_id(session_id) {
        return Err(error(
            "conversation-identity-invalid",
            "provider returned an invalid conversation identity",
            expected,
        ));
    }
    let _lock = acquire_session_record_lock(context, &expected.id)?;
    let current = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(expected, &current)?;
    retain_observation(context, &current, session_id)?;
    persist_locked(context, &current, session_id, false)
}

/// Preserve exact runtime-bound evidence without admitting a turn or changing
/// a binding. A rejected Codex turn may reveal the TUI's live thread.
pub(crate) fn retain_observation(
    context: &CliContext,
    expected: &SessionRecord,
    session_id: &str,
) -> Result<(), CliError> {
    provider(expected)?;
    if !valid_id(session_id) {
        return Err(error(
            "conversation-identity-invalid",
            "provider conversation identity is invalid",
            expected,
        ));
    }
    let current = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(expected, &current)?;
    let runtime = current.runtime.as_ref().ok_or_else(|| {
        error(
            "conversation-runtime-missing",
            "session has no live runtime identity",
            &current,
        )
    })?;
    let observation = LiveConversation {
        schema_version: "agent-session.provider-conversation.v1".into(),
        runtime_id: runtime.launch_id.clone(),
        runtime_generation: runtime.generation,
        provider: current.agent.clone(),
        session_id: session_id.into(),
    };
    write_private_file(
        &session_dir(context, &current.id).join(LIVE_FILE),
        &serde_json::to_vec(&observation).expect("conversation serialization"),
    )
}

fn persist_locked(
    context: &CliContext,
    current: &SessionRecord,
    session_id: &str,
    recover: bool,
) -> Result<(), CliError> {
    let agent = provider(current)?;
    let changed = current
        .provider_resume
        .as_ref()
        .is_none_or(|resume| resume.session_id != session_id);
    let mut next = current.clone();
    if changed {
        let now = Timestamp::now().to_string();
        next.provider_resume = Some(ProviderResume {
            provider: agent.as_str().into(),
            session_id: session_id.into(),
            captured_at: now.clone(),
            capture_method: "provider-native-conversation".into(),
            resume_args: canonical_provider_resume_args(agent, &next.cwd, session_id)
                .expect("supported provider"),
            extra: BTreeMap::new(),
        });
        next.updated_at = now;
    }
    let attached = codex_app_server::thread_attached_path(current);
    let previous_binding = attached
        .map(|path| match fs::read(path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        })
        .transpose()
        .map(Option::flatten)
        .map_err(|_| {
            error(
                "conversation-binding-unavailable",
                "provider runtime binding is unreadable",
                current,
            )
        })?;
    activity::with_conversation_rebind(context, &next, changed || recover, || {
        let result = (|| {
            if agent == AgentKind::Codex {
                codex_app_server::replace_thread_binding(&next, session_id)?;
            }
            if changed {
                write_session_record(context, &next)?;
            }
            Ok(())
        })();
        if result.is_err() {
            if let Some(path) = attached {
                if let Some(bytes) = previous_binding.as_ref() {
                    write_private_file(path, bytes)?;
                } else if let Err(err) = fs::remove_file(path)
                    && err.kind() != std::io::ErrorKind::NotFound
                {
                    return Err(error(
                        "conversation-binding-rollback-failed",
                        "provider binding rollback failed; use rebind after verifying idle",
                        current,
                    ));
                }
            }
            write_session_record(context, current)?;
        }
        result
    })
}

fn require_idle(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    match activity::state_for_view(context, record) {
        Some(turn) if turn.phase == activity::TurnPhase::Waiting && turn.current_turn.is_none() => {
            Ok(())
        }
        _ => Err(error(
            "conversation-session-not-idle",
            "conversation operation requires a verified idle session",
            record,
        )),
    }
}

fn require_claude_recovery_idle(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    if require_idle(context, record).is_ok() {
        return Ok(());
    }
    // A crash after projection commit but before record commit leaves exactly
    // this runtime's new idle projection. Require both the native receipt and
    // the matching projection; a working or unverified state stays refused.
    let observation = live(context, record)?.ok_or_else(|| {
        error(
            "conversation-observation-unavailable",
            "Claude rebind requires a native clear hook observation",
            record,
        )
    })?;
    let mut projected = record.clone();
    if let Some(resume) = projected.provider_resume.as_mut() {
        resume.session_id = observation.session_id;
    } else {
        return require_idle(context, record);
    }
    require_idle(context, &projected)
}

pub(crate) enum Operation {
    Clear(ClearOptions),
    Rebind,
}

pub(crate) fn run(context: &CliContext, args: ConversationArgs, request: Operation) -> i32 {
    let command = match request {
        Operation::Clear(_) => "clear",
        Operation::Rebind => "rebind",
    };
    let format = args.format;
    match operation(context, args, request) {
        Ok(view) => crate::render_single_success(command, format, &view, |view| {
            format!(
                "{}: {} -> {}\n",
                view.id,
                view.old_provider_session_id.as_deref().unwrap_or("unknown"),
                view.new_provider_session_id
            )
        }),
        Err(err) => crate::render_error(command, format, err),
    }
}

fn operation_guard(context: &CliContext, record: &SessionRecord) -> Result<fs::File, CliError> {
    let path = session_dir(context, &record.id).join("conversation-operation.lock");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| {
            error(
                "conversation-operation-unavailable",
                "conversation operation fence is unavailable",
                record,
            )
        })?;
    // SAFETY: flock borrows the valid descriptor owned by file; dropping it
    // releases the lease even if this command exits while awaiting a receipt.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(error(
            "conversation-operation-busy",
            "another conversation operation is in progress",
            record,
        ));
    }
    Ok(file)
}

fn operation(
    context: &CliContext,
    args: ConversationArgs,
    request: Operation,
) -> Result<ResultView, CliError> {
    let expected = load_session_record(context, &args.id)?;
    let agent = provider(&expected)?;
    let _operation_guard = operation_guard(context, &expected)?;
    match request {
        Operation::Clear(options) => {
            let mut attempt = clear::Attempt::start(context, &expected, &options)?;
            let result = transition(
                context,
                args,
                &expected,
                agent,
                Some((&options, &mut attempt)),
            );
            attempt.finish(result)
        }
        Operation::Rebind => transition(context, args, &expected, agent, None),
    }
}

fn transition(
    context: &CliContext,
    args: ConversationArgs,
    expected: &SessionRecord,
    agent: AgentKind,
    mut request: Option<(&ClearOptions, &mut clear::Attempt)>,
) -> Result<ResultView, CliError> {
    let clear = request.is_some();
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    let old = expected
        .provider_resume
        .as_ref()
        .map(|resume| resume.session_id.clone());
    if let Some((options, attempt)) = request.as_mut() {
        let deadline = options
            .wait_idle
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        loop {
            let result = (|| {
                let _record_lock = match deadline {
                    Some(deadline) => crate::acquire_session_record_lock_timed(
                        context,
                        &expected.id,
                        deadline.saturating_duration_since(Instant::now()),
                    )?,
                    None => acquire_session_record_lock(context, &expected.id)?,
                };
                let current = load_session_record(context, &expected.id)?;
                check_runtime(context, expected, &current, &tmux, deadline)?;
                admit_clear(context, &current, &tmux, deadline, options, attempt)
            })();
            match result {
                Err(err) if deadline.is_some() && waitable(&err) => {
                    let remaining = deadline
                        .expect("wait requested")
                        .saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(idle_timeout(expected));
                    }
                    thread::sleep(remaining.min(Duration::from_millis(100)));
                }
                result => {
                    result?;
                    break;
                }
            }
        }
    } else {
        let _record_lock = acquire_session_record_lock(context, &expected.id)?;
        let current = load_session_record(context, &expected.id)?;
        check_runtime(context, expected, &current, &tmux, None)?;
        if agent == AgentKind::Claude {
            require_claude_recovery_idle(context, &current)?;
        }
    }
    #[cfg(all(test, target_os = "linux"))]
    if clear && let Some(runtime) = &expected.runtime {
        crate::restart::test_control::fire(
            context,
            crate::restart::test_control::Phase::ClearSubmitted,
            &runtime.launch_id,
        );
    }
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    let session_id = if clear {
        loop {
            if Instant::now() >= deadline {
                return Err(clear_timeout(expected));
            }
            let current = load_session_record(context, &expected.id)?;
            ensure_same_session_identity(expected, &current)?;
            if let Some(observation) = live(context, &current)?
                && Some(&observation.session_id) != old.as_ref()
                && current
                    .provider_resume
                    .as_ref()
                    .is_some_and(|resume| resume.session_id == observation.session_id)
            {
                break observation.session_id;
            }
            thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(100)),
            );
        }
    } else if agent == AgentKind::Codex {
        let candidate = live(context, expected)?.map(|value| value.session_id);
        codex_app_server::probe_idle_conversation(context, expected, candidate.as_deref())?
    } else {
        let observation = live(context, expected)?
            .ok_or_else(|| {
                error(
                    "conversation-observation-unavailable",
                    "Claude rebind requires a native clear hook observation",
                    expected,
                )
            })?
            .session_id;
        if Some(&observation) == old.as_ref() {
            return Err(error(
                "conversation-observation-stale",
                "Claude rebind requires a newly observed native conversation identity",
                expected,
            ));
        }
        observation
    };
    let lock = if clear {
        crate::acquire_session_record_lock_timed(
            context,
            &expected.id,
            deadline.saturating_duration_since(Instant::now()),
        )
        .map_err(|err| {
            if err.code() == "session-record-lock-timeout" {
                clear_timeout(expected)
            } else {
                err
            }
        })?
    } else {
        acquire_session_record_lock(context, &expected.id)?
    };
    let current = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(expected, &current)?;
    let gate = if agent == AgentKind::Codex {
        Some(codex_app_server::acquire_account_mutation_gate(
            context,
            &current.id,
        )?)
    } else {
        None
    };
    if !clear && agent == AgentKind::Codex {
        // Re-probe while holding both admission fences. A stale unknown/busy
        // snapshot cannot authorize recovery; the live provider must prove idle.
        codex_app_server::probe_idle_conversation(context, &current, Some(&session_id))?;
    } else if !clear && agent == AgentKind::Claude {
        require_claude_recovery_idle(context, &current)?;
    } else {
        require_idle(context, &current)?;
    }
    if !clear {
        persist_locked(context, &current, &session_id, true)?;
    }
    let mut confirmed = load_session_record(context, &expected.id)?;
    if confirmed
        .provider_resume
        .as_ref()
        .map(|resume| resume.session_id.as_str())
        != Some(session_id.as_str())
    {
        return Err(error(
            "conversation-rebind-unconfirmed",
            "provider identity persistence did not confirm the observed conversation",
            &confirmed,
        ));
    }
    if let Some((_, attempt)) = request {
        attempt.confirmed_clear(&session_id)?;
        crate::provider_continuity::append(
            expected,
            &mut confirmed,
            crate::provider_continuity::Kind::Clear,
        );
        write_session_record(context, &confirmed)?;
        // Codex's TUI must acquire this gate for the single turn/start.
        // The record lock and the clear input fence still exclude managed input.
        drop(gate);
        attempt.submit_prompt(context, &confirmed, &tmux, lock)?;
    }
    Ok(ResultView {
        id: expected.id.clone(),
        provider: agent.as_str().into(),
        changed: old.as_deref() != Some(&session_id),
        old_provider_session_id: old,
        new_provider_session_id: session_id,
        support: if agent == AgentKind::Codex {
            "app-server"
        } else {
            "native-hook"
        },
        receipt: None,
        receipt_path: None,
    })
}

fn check_runtime(
    context: &CliContext,
    expected: &SessionRecord,
    current: &SessionRecord,
    tmux: &Path,
    deadline: Option<Instant>,
) -> Result<(), CliError> {
    ensure_same_session_identity(expected, current)?;
    if current
        .provider_resume
        .as_ref()
        .map(|resume| &resume.session_id)
        != expected
            .provider_resume
            .as_ref()
            .map(|resume| &resume.session_id)
    {
        return Err(error(
            "conversation-state-changed",
            "provider conversation changed before admission; retry from current state",
            current,
        ));
    }
    let status = if crate::recorded_runtime_is_from_prior_boot(current) {
        "stopped".into()
    } else if let Some(deadline) = deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(idle_timeout(current));
        }
        crate::live_status_with_timeout(
            tmux,
            &current.tmux_session,
            remaining.min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
        )
    } else {
        crate::session_status(context, tmux, current)
    };
    if status != "running" {
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(idle_timeout(current));
        }
        return Err(error(
            "session-not-running",
            "conversation operation requires a running session",
            current,
        ));
    }
    if current.agent == "codex"
        && (!codex_app_server::runtime_is_supported(current)
            || !codex_app_server::live_conversation_capability(context, current))
    {
        return Err(error(
            "conversation-runtime-unsupported",
            "Codex conversation operations require a live proxy with conversation-rebind support; stop and resume with the upgraded runtime first",
            current,
        ));
    }
    Ok(())
}

fn admit_clear(
    context: &CliContext,
    current: &SessionRecord,
    tmux: &Path,
    deadline: Option<Instant>,
    options: &ClearOptions,
    attempt: &mut clear::Attempt,
) -> Result<(), CliError> {
    let enhanced = options.wait_idle.is_some() || options.initial_prompt_file.is_some();
    let codex = current.agent == "codex";
    let shared_claude_admission = enhanced && !codex;
    let _gate = if codex {
        Some(codex_app_server::acquire_account_mutation_gate(
            context,
            &current.id,
        )?)
    } else {
        None
    };
    let candidate = current
        .provider_resume
        .as_ref()
        .map(|resume| resume.session_id.as_str());
    if let Some(observation) = live(context, current)?
        && Some(observation.session_id.as_str()) != candidate
    {
        return Err(error(
            "conversation-rebind-required",
            "a pending native conversation transition requires rebind before another clear",
            current,
        ));
    }
    let _activity_lock = activity::acquire_coordination_activity_lock(context, &current.id)?;
    activity::refuse_if_blocked(
        context,
        current,
        "resolve the provider prompt before retrying clear",
    )?;
    if !shared_claude_admission {
        require_idle(context, current)?;
    }
    let ingress = activity::pending::probe_lock(context, &current.id)?.ok_or_else(|| {
        error(
            "send-provider-event-pending",
            "provider event ingestion is pending",
            current,
        )
    })?;
    let _ingress = activity::pending::hold(context, &current.id, ingress);
    if codex {
        codex_app_server::idle_probe::probe_idle_conversation_until(
            context, current, candidate, deadline,
        )?;
    }
    coordination::unread::check(
        context,
        &current.id,
        options.allow_unread_mail || enhanced,
        "retry_conversation_clear",
    )?;
    let composer_budget = Instant::now() + crate::send::guarded_hold_budget(current, false);
    let composer_deadline = deadline.map_or(composer_budget, |d| d.min(composer_budget));
    let check_composer = || check_composer(context, current, tmux, composer_deadline);
    if shared_claude_admission {
        crate::delete_recovery::check_claude_idle_and_composer(
            context,
            current,
            deadline,
            check_composer,
        )?;
    } else {
        check_composer()?;
    }
    if enhanced {
        attempt.fence_input(context, current)?;
    }
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(idle_timeout(current));
    }
    crate::codex_account::authorize_terminal_input_locked(context, &mut current.clone())?;
    crate::auto_resume::cancel_for_manual_input_locked(
        context,
        &current.id,
        &Timestamp::now().to_string(),
    )?;
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(idle_timeout(current));
    }
    attempt.submitted_clear()?;
    // The provider observer needs the record lock to commit the new identity.
    // The enhanced clear's input fence remains held while that lock is released.
    crate::send::input::send_conversation_input_unlocked(
        context, current, Some(if codex { "/new" } else { "/clear" }),
        &[SpecialKey::Enter], tmux, None, crate::PasteMode::Raw,
    )
    .map_err(|err| {
        if enhanced {
            error("conversation-clear-submission-unconfirmed",
                  "native clear submission is unconfirmed; inspect the provider and use rebind before retrying clear", current)
        } else { err }
    })
}

fn check_composer(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
) -> Result<(), CliError> {
    send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &send_guard::GuardArgs {
            require_empty_composer: true,
            ..Default::default()
        },
        tmux_bin: tmux,
        verify_composer: false,
        deadline,
    }
    .check_preserving_draft("admission")
}

fn waitable(err: &CliError) -> bool {
    matches!(
        err.code(),
        "conversation-session-not-idle"
            | "conversation-live-identity-unverified"
            | "send-provider-event-pending"
            | "send-guard-observation-timeout"
            | "session-record-lock-timeout"
            | "codex-account-session-busy"
            | "coordination-notification-submission-in-progress"
            | "session-status-unknown"
    ) || (err.code() == "session-state-mismatch"
        && matches!(
            err.details()
                .and_then(|details| details.get("state"))
                .and_then(serde_json::Value::as_str),
            Some("working" | "provider-tool-pending" | "provider-event-pending")
        ))
}

fn idle_timeout(record: &SessionRecord) -> CliError {
    error(
        "conversation-idle-timeout",
        "session did not become verifiably idle before the wait expired",
        record,
    )
}

fn clear_timeout(record: &SessionRecord) -> CliError {
    error(
        "conversation-clear-unconfirmed",
        "native clear was submitted but no new provider identity was confirmed; inspect the provider and use rebind before retrying clear",
        record,
    )
}

#[cfg(test)]
#[path = "conversation/clear_admission_tests.rs"]
mod clear_admission_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    const CLEAR: Operation = Operation::Clear(ClearOptions {
        allow_unread_mail: false,
        wait_idle: None,
        initial_prompt_file: None,
    });
    pub(super) fn fixture(
        tmp: &tempfile::TempDir,
        agent: AgentKind,
    ) -> (CliContext, SessionRecord, std::path::PathBuf) {
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let cwd = tmp.path().join("repo");
        fs::create_dir_all(&cwd).unwrap();
        let mut created = crate::create_record(crate::RecordRequest {
            context: &context,
            agent,
            mode: "interactive",
            coordination_mode: crate::cli::CoordinationMode::Advisory,
            title: Some("Retained title"),
            title_state: None,
            explicit_id: Some("conversation-test"),
            cwd: &cwd,
            prompt: None,
            log_file_name: None,
            provider_resume: Some(ProviderResume {
                provider: agent.as_str().into(),
                session_id: "old-conversation".into(),
                captured_at: "2026-01-01T00:00:00Z".into(),
                capture_method: "test".into(),
                resume_args: canonical_provider_resume_args(
                    agent,
                    &cwd.to_string_lossy(),
                    "old-conversation",
                )
                .unwrap(),
                extra: BTreeMap::new(),
            }),
            agent_args: vec![],
            agent_bin: None,
        })
        .unwrap();
        created.release_lifecycle_lock();
        activity::activate_runtime(&context, &created.record).unwrap();
        activity::with_conversation_rebind(&context, &created.record, true, || Ok(())).unwrap();
        let tmux = tmp.path().join("tmux");
        let state_dir = crate::display_path(&context.state_dir);
        let runtime_id = &created.record.runtime.as_ref().unwrap().launch_id;
        fs::write(
            &tmux,
            format!(
                r#"#!/bin/sh
printf '%s\n' "$@" >> {calls}
case "$1" in
  capture-pane) printf '%s\n' '────────────────────' '❯ ' '────────────────────' ;;
  show-environment)
    case "$4" in
      AGENT_SESSION_ID) printf '%s\n' AGENT_SESSION_ID={id} ;;
      AGENT_SESSION_STATE_DIR) printf '%s\n' AGENT_SESSION_STATE_DIR={state_dir} ;;
      AGENT_SESSION_RUNTIME_ID) printf '%s\n' AGENT_SESSION_RUNTIME_ID={runtime_id} ;;
    esac ;;
esac
"#,
                calls = shell_words::quote(&tmp.path().join("tmux.calls").to_string_lossy()),
                id = shell_words::quote(&created.record.id),
                state_dir = shell_words::quote(&state_dir),
                runtime_id = shell_words::quote(runtime_id),
            ),
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o755)).unwrap();
        (context, created.record, tmux)
    }

    #[test]
    fn native_transition_preserves_runtime_and_updates_exact_resume_arguments() {
        let lock = nils_test_support::GlobalStateLock::new();
        let _policy = nils_test_support::EnvGuard::remove(&lock, crate::launch_env::ALLOWLIST_ENV);
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _) = fixture(&tmp, AgentKind::Claude);
        record.extra.insert(
            "launch_env".into(),
            json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"}),
        );
        crate::write_session_record(&context, &record).unwrap();
        observe_native(&context, &record, "new-conversation").unwrap();
        let new = load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            serde_json::to_value(&new.runtime).unwrap(),
            serde_json::to_value(&record.runtime).unwrap()
        );
        assert_eq!(new.title, record.title);
        assert_eq!(new.extra["launch_env"], record.extra["launch_env"]);
        assert_eq!(
            new.provider_resume.as_ref().unwrap().resume_args,
            vec!["--resume", "new-conversation"]
        );
        assert_eq!(
            live(&context, &new).unwrap().unwrap().session_id,
            "new-conversation"
        );
        assert_eq!(
            activity::state_for_view(&context, &new).unwrap().phase,
            activity::TurnPhase::Waiting
        );
    }

    #[test]
    fn clear_refuses_unknown_and_working_snapshots_without_terminal_input() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, tmux) = fixture(&tmp, AgentKind::Claude);
        for phase in ["starting", "working", "unknown"] {
            let _ = fs::remove_file(tmp.path().join("tmux.calls"));
            let path = session_dir(&context, &record.id).join("activity.json");
            let mut document: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            document["state"]["phase"] = json!(phase);
            write_private_file(&path, &serde_json::to_vec(&document).unwrap()).unwrap();
            let args = ConversationArgs {
                id: record.id.clone(),
                expect_idle: false,
                timeout: 1,
                tmux_bin: Some(tmux.clone()),
                format: crate::OutputFormat::Json,
            };
            assert_eq!(
                operation(&context, args, CLEAR).unwrap_err().code(),
                "conversation-session-not-idle"
            );
            assert_eq!(
                load_session_record(&context, &record.id)
                    .unwrap()
                    .provider_resume
                    .as_ref()
                    .unwrap()
                    .session_id,
                "old-conversation"
            );
            let calls = fs::read_to_string(tmp.path().join("tmux.calls")).unwrap();
            assert!(!calls.contains("paste-buffer"));
            assert!(!calls.contains("send-keys"));
        }
    }

    #[test]
    fn failed_record_commit_restores_activity_and_keeps_recovery_observation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = fixture(&tmp, AgentKind::Claude);
        let before = activity::state_for_view(&context, &record).unwrap();
        crate::fail_session_record_write_on_nth_call(1);
        assert!(observe_native(&context, &record, "new-conversation").is_err());
        let current = load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            current.provider_resume.as_ref().unwrap().session_id,
            "old-conversation"
        );
        assert_eq!(
            activity::state_for_view(&context, &current).unwrap(),
            before
        );
        assert_eq!(
            live(&context, &current).unwrap().unwrap().session_id,
            "new-conversation"
        );
        let args = ConversationArgs {
            id: record.id.clone(),
            expect_idle: true,
            timeout: 1,
            tmux_bin: Some(tmp.path().join("tmux")),
            format: crate::OutputFormat::Json,
        };
        let recovered = operation(&context, args, Operation::Rebind).unwrap();
        assert_eq!(recovered.new_provider_session_id, "new-conversation");
        assert_eq!(
            load_session_record(&context, &record.id)
                .unwrap()
                .provider_resume
                .as_ref()
                .unwrap()
                .session_id,
            "new-conversation"
        );
    }

    #[test]
    fn native_command_delivery_without_new_identity_never_reports_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, tmux) = fixture(&tmp, AgentKind::Claude);
        let args = ConversationArgs {
            id: record.id.clone(),
            expect_idle: true,
            timeout: 1,
            tmux_bin: Some(tmux),
            format: crate::OutputFormat::Json,
        };
        assert_eq!(
            operation(&context, args, CLEAR).unwrap_err().code(),
            "conversation-clear-unconfirmed"
        );
        assert_eq!(
            load_session_record(&context, &record.id)
                .unwrap()
                .provider_resume
                .as_ref()
                .unwrap()
                .session_id,
            "old-conversation"
        );
    }

    #[test]
    fn clear_command_confirms_ids_and_preserves_exact_resume_identity() {
        let lock = nils_test_support::GlobalStateLock::new();
        let _policy = nils_test_support::EnvGuard::remove(&lock, crate::launch_env::ALLOWLIST_ENV);
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, tmux) = fixture(&tmp, AgentKind::Claude);
        record.extra.insert(
            "launch_env".into(),
            json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"}),
        );
        crate::write_session_record(&context, &record).unwrap();
        let observer_context = context.clone();
        let observer_record = record.clone();
        let calls = tmp.path().join("tmux.calls");
        let observer = thread::spawn(move || {
            for _ in 0..100 {
                if fs::read_to_string(&calls)
                    .unwrap_or_default()
                    .contains("send-keys")
                {
                    observe_native(&observer_context, &observer_record, "new-conversation")
                        .unwrap();
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("native clear was not delivered");
        });
        let view = operation(
            &context,
            ConversationArgs {
                id: record.id.clone(),
                expect_idle: true,
                timeout: 3,
                tmux_bin: Some(tmux),
                format: crate::OutputFormat::Json,
            },
            CLEAR,
        )
        .unwrap();
        observer.join().unwrap();
        let result = serde_json::to_value(&view).unwrap();
        assert_eq!(result["old_provider_session_id"], "old-conversation");
        assert_eq!(result["new_provider_session_id"], "new-conversation");
        assert_eq!(result["support"], "native-hook");
        assert_eq!(result["changed"], true);
        let current = load_session_record(&context, &record.id).unwrap();
        assert_eq!(current.extra["launch_env"], record.extra["launch_env"]);
        assert_eq!(
            current.provider_resume.unwrap().resume_args,
            vec!["--resume", "new-conversation"]
        );
    }

    #[test]
    fn claude_rebind_recovers_projection_commit_interrupted_before_record_commit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, tmux) = fixture(&tmp, AgentKind::Claude);
        retain_observation(&context, &record, "new-conversation").unwrap();
        let mut projected = record.clone();
        projected.provider_resume.as_mut().unwrap().session_id = "new-conversation".into();
        activity::with_conversation_rebind(&context, &projected, true, || Ok(())).unwrap();
        assert_eq!(
            activity::state_for_view(&context, &record).unwrap().phase,
            activity::TurnPhase::Unknown
        );
        let view = operation(
            &context,
            ConversationArgs {
                id: record.id.clone(),
                expect_idle: true,
                timeout: 1,
                tmux_bin: Some(tmux),
                format: crate::OutputFormat::Json,
            },
            Operation::Rebind,
        )
        .unwrap();
        assert_eq!(view.new_provider_session_id, "new-conversation");
        let current = load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            activity::state_for_view(&context, &current).unwrap().phase,
            activity::TurnPhase::Waiting
        );
    }

    #[test]
    fn concurrent_conversation_commands_are_refused_without_another_clear() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, tmux) = fixture(&tmp, AgentKind::Claude);
        let guard = operation_guard(&context, &record).unwrap();
        let args = ConversationArgs {
            id: record.id.clone(),
            expect_idle: true,
            timeout: 1,
            tmux_bin: Some(tmux),
            format: crate::OutputFormat::Json,
        };
        assert_eq!(
            operation(&context, args, CLEAR).unwrap_err().code(),
            "conversation-operation-busy"
        );
        drop(guard);
        assert!(operation_guard(&context, &record).is_ok());
    }

    #[test]
    fn stale_claude_receipt_and_pending_transition_cannot_report_clear_success() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, tmux) = fixture(&tmp, AgentKind::Claude);
        let args = || ConversationArgs {
            id: record.id.clone(),
            expect_idle: true,
            timeout: 1,
            tmux_bin: Some(tmux.clone()),
            format: crate::OutputFormat::Json,
        };
        retain_observation(&context, &record, "old-conversation").unwrap();
        assert_eq!(
            operation(&context, args(), Operation::Rebind)
                .unwrap_err()
                .code(),
            "conversation-observation-stale"
        );
        retain_observation(&context, &record, "new-conversation").unwrap();
        assert_eq!(
            operation(&context, args(), CLEAR).unwrap_err().code(),
            "conversation-rebind-required"
        );
        let calls = fs::read_to_string(tmp.path().join("tmux.calls")).unwrap();
        assert!(!calls.contains("send-keys"));
    }

    #[test]
    fn retired_runtime_receipt_does_not_block_future_native_clear_observation() {
        for agent in [AgentKind::Codex, AgentKind::Claude] {
            let tmp = tempfile::TempDir::new().unwrap();
            let (context, record, _) = fixture(&tmp, agent);
            retain_observation(&context, &record, "new-conversation").unwrap();
            let mut resumed = record.clone();
            let runtime = resumed.runtime.as_mut().unwrap();
            runtime.launch_id = "resumed-runtime".into();
            runtime.generation += 1;
            write_session_record(&context, &resumed).unwrap();
            assert!(live(&context, &resumed).unwrap().is_none());
            retain_observation(&context, &resumed, "next-conversation").unwrap();
            assert_eq!(
                live(&context, &resumed).unwrap().unwrap().session_id,
                "next-conversation"
            );
        }
    }

    #[test]
    fn unknown_provider_is_refused_before_runtime_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, tmux) = fixture(&tmp, AgentKind::Claude);
        record.agent = "unsupported-agent".into();
        write_session_record(&context, &record).unwrap();
        let args = ConversationArgs {
            id: record.id.clone(),
            expect_idle: true,
            timeout: 1,
            tmux_bin: Some(tmux),
            format: crate::OutputFormat::Json,
        };
        assert_eq!(
            operation(&context, args, CLEAR).unwrap_err().code(),
            "conversation-provider-unsupported"
        );
        assert!(live(&context, &record).unwrap().is_none());
    }

    #[test]
    fn stale_runtime_and_invalid_identity_are_refused() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = fixture(&tmp, AgentKind::Claude);
        for id in ["", "bad\nid", "local:v1:projected"] {
            assert_eq!(
                observe_native(&context, &record, id).unwrap_err().code(),
                "conversation-identity-invalid"
            );
        }
        let mut stale = record.clone();
        stale.runtime.as_mut().unwrap().launch_id = "retired-runtime".into();
        assert!(observe_native(&context, &stale, "new-conversation").is_err());
        assert!(live(&context, &record).unwrap().is_none());
    }
}

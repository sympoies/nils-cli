//! Provider-confirmed conversation transitions, independent of managed runtime identity.
use std::{
    collections::BTreeMap,
    fs, thread,
    time::{Duration, Instant},
};

use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    CliContext, CliError, ProviderResume, SessionRecord, acquire_session_record_lock, activity,
    canonical_provider_resume_args,
    cli::{AgentKind, ConversationArgs, SpecialKey},
    codex_app_server, ensure_same_session_identity, load_session_record, session_dir,
    write_private_file, write_session_record,
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
    let bytes = match fs::read(session_dir(context, &record.id).join(LIVE_FILE)) {
        Ok(bytes) => bytes,
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

pub(crate) fn run(context: &CliContext, args: ConversationArgs, clear: bool) -> i32 {
    let command = if clear { "clear" } else { "rebind" };
    let format = args.format;
    match operation(context, args, clear) {
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
    clear: bool,
) -> Result<ResultView, CliError> {
    let expected = load_session_record(context, &args.id)?;
    let agent = provider(&expected)?;
    let _operation_guard = operation_guard(context, &expected)?;
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    let old = expected
        .provider_resume
        .as_ref()
        .map(|resume| resume.session_id.clone());
    {
        let _record_lock = acquire_session_record_lock(context, &expected.id)?;
        let current = load_session_record(context, &expected.id)?;
        ensure_same_session_identity(&expected, &current)?;
        if current
            .provider_resume
            .as_ref()
            .map(|resume| &resume.session_id)
            != old.as_ref()
        {
            return Err(error(
                "conversation-state-changed",
                "provider conversation changed before admission; retry from current state",
                &current,
            ));
        }
        if crate::session_status(context, &tmux, &current) != "running" {
            return Err(error(
                "session-not-running",
                "conversation operation requires a running session",
                &current,
            ));
        }
        if agent == AgentKind::Codex
            && (!codex_app_server::runtime_is_supported(&current)
                || !codex_app_server::live_conversation_capability(context, &current))
        {
            return Err(error(
                "conversation-runtime-unsupported",
                "Codex conversation operations require a live proxy with conversation-rebind support; stop and resume with the upgraded runtime first",
                &current,
            ));
        }
        let _gate = if agent == AgentKind::Codex {
            Some(codex_app_server::acquire_account_mutation_gate(
                context,
                &current.id,
            )?)
        } else {
            None
        };
        if clear {
            if let Some(observation) = live(context, &current)?
                && Some(&observation.session_id) != old.as_ref()
            {
                return Err(error(
                    "conversation-rebind-required",
                    "a pending native conversation transition requires rebind before another clear",
                    &current,
                ));
            }
            let _activity_lock =
                activity::acquire_coordination_activity_lock(context, &current.id)?;
            require_idle(context, &current)?;
            if agent == AgentKind::Codex {
                let candidate = current
                    .provider_resume
                    .as_ref()
                    .map(|resume| resume.session_id.as_str());
                codex_app_server::probe_idle_conversation(context, &current, candidate)?;
            }
            crate::codex_account::authorize_terminal_input_locked(context, &mut current.clone())?;
            crate::auto_resume::cancel_for_manual_input_locked(
                context,
                &current.id,
                &Timestamp::now().to_string(),
            )?;
            // Native slash commands do not start a turn. Release the lifecycle
            // lock promptly so the provider can commit its observed transition.
            crate::send_input_unlocked(
                context,
                &current,
                Some(if agent == AgentKind::Codex {
                    "/new"
                } else {
                    "/clear"
                }),
                &[SpecialKey::Enter],
                &tmux,
                None,
                crate::PasteMode::Raw,
            )?;
        } else if agent == AgentKind::Claude {
            require_claude_recovery_idle(context, &current)?;
        }
    }
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    let session_id = if clear {
        loop {
            let current = load_session_record(context, &expected.id)?;
            ensure_same_session_identity(&expected, &current)?;
            if let Some(observation) = live(context, &current)?
                && Some(&observation.session_id) != old.as_ref()
            {
                break observation.session_id;
            }
            if Instant::now() >= deadline {
                return Err(error(
                    "conversation-clear-unconfirmed",
                    "native clear was submitted but no new provider identity was confirmed; inspect the provider and use rebind before retrying clear",
                    &current,
                ));
            }
            thread::sleep(Duration::from_millis(100));
        }
    } else if agent == AgentKind::Codex {
        let candidate = live(context, &expected)?.map(|value| value.session_id);
        codex_app_server::probe_idle_conversation(context, &expected, candidate.as_deref())?
    } else {
        let observation = live(context, &expected)?
            .ok_or_else(|| {
                error(
                    "conversation-observation-unavailable",
                    "Claude rebind requires a native clear hook observation",
                    &expected,
                )
            })?
            .session_id;
        if Some(&observation) == old.as_ref() {
            return Err(error(
                "conversation-observation-stale",
                "Claude rebind requires a newly observed native conversation identity",
                &expected,
            ));
        }
        observation
    };
    let _lock = acquire_session_record_lock(context, &expected.id)?;
    let current = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(&expected, &current)?;
    let _gate = if agent == AgentKind::Codex {
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
    let confirmed = load_session_record(context, &expected.id)?;
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
    Ok(ResultView {
        id: expected.id,
        provider: agent.as_str().into(),
        changed: old.as_deref() != Some(&session_id),
        old_provider_session_id: old,
        new_provider_session_id: session_id,
        support: if agent == AgentKind::Codex {
            "app-server"
        } else {
            "native-hook"
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::PermissionsExt;

    fn fixture(
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
        fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$@\" >> {}\nexit 0\n",
                shell_words::quote(&tmp.path().join("tmux.calls").to_string_lossy())
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
        for phase in ["starting", "working", "needs_input", "unknown"] {
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
                operation(&context, args, true).unwrap_err().code(),
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
        let recovered = operation(&context, args, false).unwrap();
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
            operation(&context, args, true).unwrap_err().code(),
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
            true,
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
            false,
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
            operation(&context, args, true).unwrap_err().code(),
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
            operation(&context, args(), false).unwrap_err().code(),
            "conversation-observation-stale"
        );
        retain_observation(&context, &record, "new-conversation").unwrap();
        assert_eq!(
            operation(&context, args(), true).unwrap_err().code(),
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
            operation(&context, args, true).unwrap_err().code(),
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

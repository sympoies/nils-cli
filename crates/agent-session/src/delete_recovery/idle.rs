use super::*;
use crate::activity::{Confidence, SourceKind, TurnPhase};
use crate::codex_app_server::{self, guarded};

mod claude;

pub(super) fn delete_if_idle(
    context: &CliContext,
    args: &cli::DeleteArgs,
) -> Result<DeleteResult, CliError> {
    let machine = board::machine_identity(None, context);
    let children = lineage::guard_children(context, &machine, &args.id, args.orphan_children)?;
    let observed = load_session_record(context, &args.id)?;
    let _record_lock = acquire_session_record_lock(context, &observed.id)?;
    let record = load_session_record(context, &observed.id)?;
    let expected = args.expected_incarnation.as_deref().unwrap_or_default();
    let actual = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|incarnation| !incarnation.is_empty());
    if actual != Some(expected) || ensure_same_session_identity(&observed, &record).is_err() {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "session runtime changed before conditional delete",
            Some(json!({
                "id": record.id,
                "expected_session_incarnation": expected,
                "actual_session_incarnation": actual,
            })),
        ));
    }

    let tmux = resolve_tmux_bin(args.tmux_bin.as_deref());
    match session_status(context, &tmux, &record).as_str() {
        "running" => {}
        "stopped" => return Err(active(&record, "stopped")),
        _ => return Err(unknown(&record)),
    }
    let claude =
        record.agent == "claude" && record.runtime.as_ref().is_some_and(|r| r.kind == "tmux");
    if record.mode != "interactive"
        || !(claude || codex_app_server::retirement::supported(context, &record))
    {
        return Err(CliError::data(
            "session-idle-delete-unsupported",
            if record.agent == "codex" {
                "conditional idle deletion requires a live managed Codex proxy"
            } else {
                "conditional idle deletion requires a managed Claude terminal or live managed Codex proxy"
            },
            Some(json!({"id": record.id})),
        ));
    }
    let session_dir = resolve_session_record_path(context, &record.id)?.session_dir;
    if claude {
        // We hold the record lock, so a pause receipt here belongs to a run that
        // died before restoring input. Restore that exact pane before the checks.
        pane_input::restore_interrupted(&tmux, &record, &session_dir)?;
    }
    if persisted_tmux_termination_state(&record)
        .map_err(|_| unknown(&record))?
        .is_some()
    {
        return Err(CliError::runtime(
            "session-termination-in-progress",
            "conditional deletion must recover interrupted termination before removing session state",
            Some(json!({"id": record.id, "action": "retry-delete", "retryable": true})),
        ));
    }

    // The receiver's lifecycle lock serializes both local and relayed message
    // admission, including after the registry snapshot is closed for teardown.
    if args.if_unread_empty {
        coordination::unread::check(context, &record.id, false, "retry_conditional_delete")?;
    }
    ensure_session_lifecycle_mutation_allowed(context, &record)?;

    // The proxy and hook writers can acquire their gates before lifecycle.
    // Probe both of those gates without waiting while holding lifecycle.
    let _provider = if claude {
        None
    } else {
        Some(
            codex_app_server::acquire_account_mutation_gate(context, &record.id).map_err(
                |error| {
                    if error.code() == "codex-account-session-busy" {
                        active(&record, "working")
                    } else {
                        unknown(&record)
                    }
                },
            )?,
        )
    };
    let _requests = if claude {
        None
    } else {
        Some(
            codex_app_server::retirement::probe_requests(context, &record)
                .map_err(|_| unknown(&record))?
                .ok_or_else(|| active(&record, "working"))?,
        )
    };
    let _activity = activity::acquire_coordination_activity_lock(context, &record.id)
        .map_err(|_| unknown(&record))?;
    let _ingress = activity::pending::probe_lock(context, &record.id)
        .map_err(|_| unknown(&record))?
        .ok_or_else(|| active(&record, "provider-event-pending"))?;

    if activity::pending::unsettled(context, &record).ok_or_else(|| unknown(&record))? {
        return Err(active(&record, "provider-event-pending"));
    }
    if claude {
        claude::require_waiting(context, &record)?;
        claude::require_native_idle(&record)?;
    } else {
        require_waiting_activity(context, &record)?;
    }
    let paused_input = if claude {
        Some(
            pane_input::PausedPaneInput::pause(&tmux, &record, &session_dir)
                .ok_or_else(|| unknown(&record))?,
        )
    } else {
        None
    };
    let deadline = Instant::now() + crate::send::guarded_hold_budget(&record, false);
    if !claude {
        require_codex_idle(&record, deadline)?;
    }
    let composer_guard = send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record: &record,
        args: &send_guard::GuardArgs {
            require_empty_composer: true,
            ..Default::default()
        },
        tmux_bin: &tmux,
        verify_composer: false,
        deadline,
    };
    let check_composer = || {
        composer_guard
            .check_preserving_draft("admission")
            .map_err(|error| match error.code() {
                "send-composer-not-empty" | "send-composer-unobservable" => error,
                _ => unknown(&record),
            })
    };
    check_composer()?;

    if claude {
        // Refuse native work that appeared during capture, then check for late
        // rendering of input accepted before the pane was paused.
        claude::require_native_idle(&record)?;
        check_composer()?;
    }

    let mut options = DeleteSessionOptions::regular(
        PANE_INPUT_COMMAND_TIMEOUT,
        DELETE_TERMINATION_VERIFY_TIMEOUT,
        board::CloseReason::Deleted,
    );
    options.paused_pane = paused_input.as_ref().map(pane_input::PausedPaneInput::pane);
    let (_, mut deleted) = delete_session_locked_with_timeouts_and_prepare(
        context,
        record,
        session_dir,
        &tmux,
        options,
        |_| Ok(()),
    )?;
    if let Some(input) = paused_input {
        input.disarm();
    }
    deleted.children = Some(children);
    Ok(deleted)
}

fn require_codex_idle(record: &SessionRecord, deadline: Instant) -> Result<(), CliError> {
    let thread = guarded::Client::connect(record, deadline)
        .and_then(|mut client| client.idle_thread(false))
        .map_err(|error| match error {
            guarded::Error::NotIdle => active(record, "working"),
            _ => unknown(record),
        })?;
    // Guarded sends also admit a loaded thread in systemError. Retirement
    // requires the provider's explicit idle status.
    if thread
        .pointer("/thread/status/type")
        .and_then(Value::as_str)
        != Some("idle")
    {
        return Err(unknown(record));
    }
    Ok(())
}

fn require_waiting_activity(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let state =
        activity::controller_state_for_record(context, record).ok_or_else(|| unknown(record))?;
    match state.phase {
        TurnPhase::Waiting
            if state.current_turn.is_none()
                && state.source.kind == SourceKind::ProviderHook
                && state.source.confidence == Confidence::Authoritative =>
        {
            Ok(())
        }
        TurnPhase::Working | TurnPhase::NeedsInput | TurnPhase::Starting => {
            Err(active(record, "working"))
        }
        _ => Err(unknown(record)),
    }
}

fn active(record: &SessionRecord, state: &str) -> CliError {
    CliError::data(
        "session-state-mismatch",
        "session is not a running, idle runtime",
        Some(json!({"id": record.id, "state": state})),
    )
}

fn unknown(record: &SessionRecord) -> CliError {
    CliError::runtime(
        "session-status-unknown",
        "running session idleness could not be verified",
        Some(json!({"id": record.id, "state": "unknown"})),
    )
}

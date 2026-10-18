//! Terminal mailbox wakes preserve observed operator drafts and never retry Enter.
use super::*;

pub(super) fn start_terminal_coordination_notification(
    context: &CliContext,
    tmux_bin: &Path,
    candidate: &crate::coordination::NotificationCandidate,
    prompt: &str,
    tail: ProviderPromptTail,
    expected_agent: AgentKind,
) -> Result<ProviderPromptTail, TerminalNotificationStartError> {
    let _record_lock = crate::acquire_session_record_lock(context, &candidate.target_session_id)
        .map_err(|_| TerminalNotificationStartError::Known("recipient-incarnation-replaced"))?;
    let current = load_session_record(context, &candidate.target_session_id)
        .map_err(|_| TerminalNotificationStartError::Known("recipient-incarnation-replaced"))?;
    if current.agent != expected_agent.as_str() {
        return Err(TerminalNotificationStartError::Known(
            "provider-unsupported",
        ));
    }
    if current.coordination_mode == cli::CoordinationMode::Off {
        return Err(TerminalNotificationStartError::Known(
            "coordination-disabled",
        ));
    }
    if current
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
        return Err(TerminalNotificationStartError::Known(
            "recipient-incarnation-replaced",
        ));
    }
    if !terminal_notification_waiting(context, &current, expected_agent) {
        return Err(TerminalNotificationStartError::Known("recipient-working"));
    }
    if crate::live_status(tmux_bin, &current.tmux_session) != "running" {
        return Err(TerminalNotificationStartError::Known("provider-not-ready"));
    }
    match tmux_session_attached(tmux_bin, &current.tmux_session) {
        Ok(false) => {}
        Ok(true) => {
            return Err(TerminalNotificationStartError::Known("recipient-attached"));
        }
        Err(()) => {
            return Err(TerminalNotificationStartError::Known("provider-not-ready"));
        }
    }

    let deadline = Instant::now() + crate::send_submit::HOLD_BUDGET;
    check_composer(context, tmux_bin, &current, expected_agent, "", deadline)
        .map_err(TerminalNotificationStartError::Known)?;

    let current = load_session_record(context, &candidate.target_session_id)
        .map_err(|_| TerminalNotificationStartError::Known("recipient-incarnation-replaced"))?;
    if current
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
        return Err(TerminalNotificationStartError::Known(
            "recipient-incarnation-replaced",
        ));
    }
    if current.agent != expected_agent.as_str()
        || !terminal_notification_waiting(context, &current, expected_agent)
        || crate::live_status(tmux_bin, &current.tmux_session) != "running"
    {
        return Err(TerminalNotificationStartError::Known("recipient-working"));
    }
    // Quiescence validation and the durable `attempting` transition occur
    // under one short registry lock. Claim and operation admission reject that
    // exact per-session submission fence while terminal I/O runs outside the
    // global coordination lock.
    let mut quiescence = crate::coordination::lock_session_quiescence(
        context,
        &candidate.target_session_id,
        &candidate.target_incarnation,
    )
    .map_err(|_| TerminalNotificationStartError::Known("provider-not-ready"))?;
    if !quiescence.broker_authoritative
        || quiescence.active_claim
        || quiescence.active_operation
        || quiescence.uncertain_operation
    {
        return Err(TerminalNotificationStartError::Known(
            "recipient-coordination-not-quiescent",
        ));
    }
    if !quiescence
        .begin_notification_attempt(candidate)
        .map_err(|_| TerminalNotificationStartError::Known("provider-not-ready"))?
    {
        return Err(TerminalNotificationStartError::Known("provider-not-ready"));
    }
    drop(quiescence);
    if !terminal_notification_waiting(context, &current, expected_agent) {
        if !crate::coordination::mark_notification_known_failure(
            context,
            candidate,
            "recipient-working",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .map_err(|_| TerminalNotificationStartError::Unknown)?
        {
            return Err(TerminalNotificationStartError::Unknown);
        }
        return Err(TerminalNotificationStartError::Known("recipient-working"));
    }
    if crate::live_status(tmux_bin, &current.tmux_session) != "running" {
        if !crate::coordination::mark_notification_known_failure(
            context,
            candidate,
            "provider-not-ready",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .map_err(|_| TerminalNotificationStartError::Unknown)?
        {
            return Err(TerminalNotificationStartError::Unknown);
        }
        return Err(TerminalNotificationStartError::Known("provider-not-ready"));
    }
    match tmux_session_attached(tmux_bin, &current.tmux_session) {
        Ok(false) => {}
        Ok(true) => {
            if !crate::coordination::mark_notification_known_failure(
                context,
                candidate,
                "recipient-attached",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .map_err(|_| TerminalNotificationStartError::Unknown)?
            {
                return Err(TerminalNotificationStartError::Unknown);
            }
            return Err(TerminalNotificationStartError::Known("recipient-attached"));
        }
        Err(()) => {
            if !crate::coordination::mark_notification_known_failure(
                context,
                candidate,
                "provider-not-ready",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .map_err(|_| TerminalNotificationStartError::Unknown)?
            {
                return Err(TerminalNotificationStartError::Unknown);
            }
            return Err(TerminalNotificationStartError::Known("provider-not-ready"));
        }
    }
    guarded_input(
        context,
        tmux_bin,
        &current,
        candidate,
        prompt,
        expected_agent,
        deadline,
    )?;
    Ok(tail)
}

fn check_composer(
    context: &CliContext,
    tmux_bin: &Path,
    record: &SessionRecord,
    expected_agent: AgentKind,
    expected_text: &str,
    deadline: Instant,
) -> Result<(), &'static str> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("recipient-composer-unobservable");
    }
    let target = format!("{}:0.0", record.tmux_session);
    let mut command = ProcessCommand::new(tmux_bin);
    // Join terminal soft wraps without trimming occupied spaces. The same exact
    // styled-composer parser as guarded send refuses hidden or ambiguous input.
    command.args(["capture-pane", "-p", "-e", "-J", "-t", &target]);
    let pane = crate::run_output_with_timeout_and_strict_cap(
        command,
        remaining.min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
        crate::PANE_OBSERVATION_MAX_OUTPUT_BYTES,
    )
    .ok()
    .filter(|output| output.status.success())
    .and_then(|output| String::from_utf8(output.stdout).ok())
    .and_then(|pane| crate::send_submit::exact_composer_input(&record.agent, &pane))
    .ok_or("recipient-composer-unobservable")?;
    if pane != expected_text {
        return Err("recipient-composer-not-empty");
    }
    // Provider events can be waiting behind our lifecycle lock while a pane
    // read runs. A pending ingest is not an idle input boundary.
    if !terminal_notification_waiting(context, record, expected_agent)
        || crate::activity::pending::event_pending(context, &record.id)
            .map_err(|_| "provider-not-ready")?
    {
        return Err("recipient-working");
    }
    match tmux_session_attached(tmux_bin, &record.tmux_session) {
        Ok(false) => Ok(()),
        Ok(true) => Err("recipient-attached"),
        Err(()) => Err("provider-not-ready"),
    }
}

fn guarded_input(
    context: &CliContext,
    tmux_bin: &Path,
    record: &SessionRecord,
    candidate: &crate::coordination::NotificationCandidate,
    prompt: &str,
    expected_agent: AgentKind,
    deadline: Instant,
) -> Result<(), TerminalNotificationStartError> {
    let nonce = uuid::Uuid::new_v4();
    let buffer = format!("{}-notification-{nonce}", record.id);
    let temp = session_dir(context, &record.id).join(format!("notification-input-{nonce}"));
    crate::write_private_file(&temp, prompt.as_bytes())
        .map_err(|_| TerminalNotificationStartError::Unknown)?;
    let target = format!("{}:0.0", record.tmux_session);
    let mut refused = None;
    let result = crate::send::load_and_paste_guarded(
        tmux_bin,
        &buffer,
        &target,
        &temp,
        crate::PANE_INPUT_COMMAND_TIMEOUT,
        crate::PasteMode::Raw,
        || {
            check_composer(context, tmux_bin, record, expected_agent, "", deadline).map_err(
                |reason| {
                    refused = Some(reason);
                    CliError::unavailable(reason, "mailbox wake input is unavailable", None)
                },
            )
        },
    );
    let _ = fs::remove_file(temp);
    if let Some(reason) = refused {
        let known = crate::coordination::mark_notification_known_failure(
            context,
            candidate,
            reason,
            COORDINATION_NOTIFICATION_RETRY,
        )
        .map_err(|_| TerminalNotificationStartError::Unknown)?;
        return Err(if known {
            TerminalNotificationStartError::Known(reason)
        } else {
            TerminalNotificationStartError::Unknown
        });
    }
    result.map_err(|_| TerminalNotificationStartError::Unknown)?;
    std::thread::sleep(crate::POST_PASTE_KEY_SETTLE_DELAY);
    // Preserve the existing post-paste race guard before the initial Enter.
    check_composer(context, tmux_bin, record, expected_agent, prompt, deadline)
        .map_err(|_| TerminalNotificationStartError::Unknown)?;
    // Confirm that the wake left the composer. A swallowed Enter is retried
    // only while the exact prompt remains visible in the recognized composer.
    let probe = crate::send_submit::probe(prompt).ok_or(TerminalNotificationStartError::Unknown)?;
    let report = crate::send_submit::submit_and_confirm(
        record.agent.as_str(),
        &probe,
        deadline,
        |timeout| {
            let mut command = ProcessCommand::new(tmux_bin);
            command.args(["capture-pane", "-p", "-e", "-J", "-t", &target]);
            crate::run_output_with_timeout_and_strict_cap(
                command,
                timeout.min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
                crate::PANE_OBSERVATION_MAX_OUTPUT_BYTES,
            )
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
        },
        || {
            terminal_notification_waiting(context, record, expected_agent)
                && check_composer(context, tmux_bin, record, expected_agent, prompt, deadline)
                    .is_ok()
        },
        || crate::send_tmux_key(tmux_bin, &target, SpecialKey::Enter),
    )
    .map_err(|_| TerminalNotificationStartError::Unknown)?;
    match report.outcome {
        crate::send_submit::SubmitOutcome::Submitted
        | crate::send_submit::SubmitOutcome::Queued => Ok(()),
        crate::send_submit::SubmitOutcome::Stuck => {
            let known = crate::coordination::mark_notification_known_failure(
                context,
                candidate,
                "submission-not-confirmed",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .map_err(|_| TerminalNotificationStartError::Unknown)?;
            Err(if known {
                TerminalNotificationStartError::Known("submission-not-confirmed")
            } else {
                TerminalNotificationStartError::Unknown
            })
        }
        crate::send_submit::SubmitOutcome::Unverified => {
            Err(TerminalNotificationStartError::Unknown)
        }
    }
}

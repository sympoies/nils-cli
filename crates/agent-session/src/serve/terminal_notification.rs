//! Terminal mailbox wakes preserve observed operator drafts and retry a swallowed Enter
//! only while the exact wake remains visible.
use super::*;

#[derive(Debug)]
pub(super) enum TerminalNotificationStartError {
    Known(&'static str),
    Unknown,
    UnknownReason(&'static str),
}

pub(super) async fn mark_unknown(
    state: &Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
    reason: &'static str,
) {
    let context = state.context.clone();
    let _ = tokio::task::spawn_blocking(move || {
        crate::coordination::mark_notification_unknown(&context, &candidate, reason)
    })
    .await;
}

pub(super) async fn mark_unknown_after_paste(
    state: &Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
    reason: &'static str,
) {
    let context = state.context.clone();
    let _ =
        tokio::task::spawn_blocking(move || candidate.mark_unknown_after_paste(&context, reason))
            .await;
}

/// Test-only override of the wake hold budget, in milliseconds; 0 means none.
/// A process-wide atomic because the wake runs on blocking-pool threads.
#[cfg(test)]
static HOLD_BUDGET_OVERRIDE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Test-only: widens the wake hold budget until dropped. The retry cadence
/// stays real; only the lock-hold deadline stops depending on how fast a loaded
/// host starts the fake tmux interpreter. A test that waits for the budget to
/// expire must not hold it.
#[cfg(test)]
pub(super) struct HoldBudgetGuard;

#[cfg(test)]
impl HoldBudgetGuard {
    pub(super) fn widen(budget: Duration) -> Self {
        HOLD_BUDGET_OVERRIDE_MS.store(
            budget.as_millis() as u64,
            std::sync::atomic::Ordering::SeqCst,
        );
        Self
    }
}

#[cfg(test)]
impl Drop for HoldBudgetGuard {
    fn drop(&mut self) {
        HOLD_BUDGET_OVERRIDE_MS.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

fn hold_budget() -> Duration {
    #[cfg(test)]
    match HOLD_BUDGET_OVERRIDE_MS.load(std::sync::atomic::Ordering::SeqCst) {
        0 => {}
        millis => return Duration::from_millis(millis),
    }
    crate::send_submit::HOLD_BUDGET
}

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

    let deadline = Instant::now() + hold_budget();
    let recovery_prompt = candidate
        .recovery_prompt(context)
        .map_err(|_| TerminalNotificationStartError::Unknown)?;
    let mut recovery_failure = None;
    if let Some(recovery_prompt) = recovery_prompt.as_deref() {
        if check_composer(
            context,
            tmux_bin,
            &current,
            expected_agent,
            recovery_prompt,
            deadline,
        )
        .is_ok()
        {
            if clear_owned_notification(context, tmux_bin, &current, recovery_prompt) {
                let _ = candidate.clear_recovery_prompt(context, recovery_prompt);
            } else {
                recovery_failure = Some("notification-residual-changed");
            }
        } else if check_composer(context, tmux_bin, &current, expected_agent, "", deadline).is_ok()
        {
            // A prior cleanup may have succeeded just before its final cursor
            // confirmation became unavailable. Empty input is safe to retire.
            let _ = candidate.clear_recovery_prompt(context, recovery_prompt);
        } else {
            recovery_failure = Some("notification-residual-unobservable-or-changed");
        }
    }
    if recovery_failure.is_none() {
        check_composer(context, tmux_bin, &current, expected_agent, "", deadline)
            .map_err(TerminalNotificationStartError::Known)?;
    }

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
    if let Some(reason) = recovery_failure {
        return Err(TerminalNotificationStartError::UnknownReason(reason));
    }
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
    if let Err(reason) = check_composer(context, tmux_bin, record, expected_agent, prompt, deadline)
    {
        if !clear_owned_notification(context, tmux_bin, record, prompt) {
            return Err(TerminalNotificationStartError::UnknownReason(reason));
        }
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
        |timeout| {
            crate::live_status_with_timeout(tmux_bin, &record.tmux_session, timeout) == "stopped"
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
            if !clear_owned_notification(context, tmux_bin, record, prompt) {
                return Err(TerminalNotificationStartError::UnknownReason(
                    "submission-not-confirmed",
                ));
            }
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
            // An empty composer is also what a successful Enter looks like, so
            // only a reminder still visible proves that the wake failed.
            match observe_composer_input(tmux_bin, record) {
                Some(input) if input.is_empty() => {
                    return Err(TerminalNotificationStartError::Unknown);
                }
                None => {
                    return Err(TerminalNotificationStartError::UnknownReason(
                        "submission-not-confirmed",
                    ));
                }
                Some(_) => {}
            }
            if !clear_owned_notification(context, tmux_bin, record, prompt) {
                return Err(TerminalNotificationStartError::UnknownReason(
                    "submission-not-confirmed",
                ));
            }
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
    }
}

/// Read the recognized composer, retrying briefly while the pane layout is
/// transiently unreadable. `None` means the composer stayed unobservable.
fn observe_composer_input(tmux_bin: &Path, record: &SessionRecord) -> Option<String> {
    let target = format!("{}:0.0", record.tmux_session);
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut command = ProcessCommand::new(tmux_bin);
        command.args(["capture-pane", "-p", "-e", "-J", "-t", &target]);
        let input = crate::run_output_with_timeout_and_strict_cap(
            command,
            remaining.min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
            crate::PANE_OBSERVATION_MAX_OUTPUT_BYTES,
        )
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|pane| crate::send_submit::exact_composer_input(&record.agent, &pane));
        if input.is_some() {
            return input;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Clear only the exact notification text this dispatch pasted. The fresh
/// composer read is the ownership check; a human draft or any mixed text is
/// left untouched, and this path never presses Enter.
fn clear_owned_notification(
    context: &CliContext,
    tmux_bin: &Path,
    record: &SessionRecord,
    prompt: &str,
) -> bool {
    let args = crate::send_guard::GuardArgs {
        require_empty_composer: true,
        ..Default::default()
    };
    let admission = crate::send_guard::Admission {
        context,
        record,
        args: &args,
        tmux_bin,
        verify_composer: true,
        deadline: Instant::now() + hold_budget(),
    };
    let error = crate::CliError::unavailable(
        "notification-submission-unconfirmed",
        "mailbox notification submission could not be confirmed",
        None,
    );
    let error = admission.report_pasted_input(error, prompt, true);
    error
        .details()
        .and_then(|details| details.get("cleared"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

mod mailbox_dispatch;
mod native_notice;
pub(super) use mailbox_dispatch::dispatch_coordination_notification;
pub(crate) use native_notice::begin_mailbox_notice;

#[cfg(test)]
pub(super) use mailbox_dispatch::dispatch_terminal_coordination_notification;

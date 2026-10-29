//! Guarded app-server sends never write to the TUI composer.
use super::*;

#[derive(Debug, Serialize)]
pub(crate) struct Receipt {
    pub(crate) transport: &'static str,
    pub(crate) confirmation: &'static str,
}

pub(super) struct Attempt<'a> {
    pub(super) started: Instant,
    pub(super) acknowledged: &'a mut dyn FnMut(&SessionRecord, &str),
    pub(super) terminal_command: bool,
}

pub(super) fn deliver(
    context: &CliContext,
    args: cli::SendArgs,
    text: String,
    mut record: SessionRecord,
    tmux_bin: &Path,
    manual_input: &mut ManualInputSection,
    attempt: Attempt<'_>,
) -> Result<SendResult, CliError> {
    let observed = record.clone();
    let admission = send_guard::Admission {
        context,
        record: &observed,
        args: &args.guards,
        tmux_bin,
        verify_composer: true,
        deadline: attempt.started + send_submit::HOLD_BUDGET,
    };
    // This shared gate serializes both proxy turn/start and account mutation.
    // It is nonblocking while the session record lock is owned.
    if text.trim_start().starts_with('/') && text.trim() != "/compact" {
        return Err(refusal(
            &record,
            "send-native-command-unsupported",
            "admission",
            false,
            false,
        ));
    }
    let _account_gate = codex_app_server::acquire_account_mutation_gate(context, &record.id)?;
    codex_account::ensure_proxy_input_allowed(&record)?;
    let mut client = codex_app_server::guarded::Client::connect(&record, admission.deadline)
        .map_err(|code| refusal(&record, code, "admission", false, false))?;
    admission.check_key()?;
    admission.check(None, "admission")?;
    client
        .idle()
        .map_err(|code| refusal(&record, code, "admission", false, false))?;
    admission.claim_key()?;
    admission.check(None, "claimed")?;
    client
        .idle()
        .map_err(|code| refusal(&record, code, "claimed", false, false))?;
    let mut prepared = false;
    prepare_guarded_input(context, &mut record, true, &mut prepared)?;
    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
    let confirmation = client
        .dispatch(&text, |turn_id| {
            (attempt.acknowledged)(&record, turn_id);
            // Acknowledged input may wait for synchronous provider ingress
            // before entering history. Ingress needs this record lock. Keep
            // the account gate and durable claim, but perform no further
            // record writes after releasing the lock.
            drop(manual_input.record_lock.take());
        })
        .map_err(|error| {
            refusal(
                &record,
                "send-native-unconfirmed",
                "submit",
                true,
                error.acknowledged,
            )
        })?;
    if manual_input.record_lock.is_none() {
        let unconfirmed = |_| refusal(&record, "send-native-unconfirmed", "submit", true, true);
        let lock = crate::acquire_session_record_lock_timed(
            context,
            &record.id,
            admission.deadline.saturating_duration_since(Instant::now()),
        )
        .map_err(unconfirmed)?;
        let current = load_session_record(context, &record.id).map_err(unconfirmed)?;
        if crate::ensure_same_session_identity(&record, &current).is_err()
            || current
                .provider_resume
                .as_ref()
                .map(|r| (&r.provider, &r.session_id))
                != record
                    .provider_resume
                    .as_ref()
                    .map(|r| (&r.provider, &r.session_id))
        {
            return Err(refusal(
                &record,
                "send-native-unconfirmed",
                "submit",
                true,
                true,
            ));
        }
        manual_input.record_lock = Some(lock);
    }
    Ok(SendResult {
        id: record.id,
        tmux_session: record.tmux_session,
        sent_text: true,
        keys: args
            .keys
            .iter()
            .map(|key| key.as_str().to_owned())
            .collect(),
        submission: Some(send_submit::SubmitReport {
            outcome: send_submit::SubmitOutcome::Submitted,
            enter_presses: 0,
        }),
        guards: Some(send_guard::GuardReport { atomic: false }),
        native: Some(Receipt {
            transport: "codex_app_server",
            confirmation,
        }),
        command_dispatched: false,
    })
}

fn refusal(
    record: &SessionRecord,
    code: &str,
    stage: &str,
    dispatched: bool,
    acknowledged: bool,
) -> CliError {
    let submitted = if !dispatched {
        Some(false)
    } else if acknowledged {
        Some(true)
    } else {
        None
    };
    CliError::runtime(
        code,
        "native guarded input could not be verified; no terminal input was written",
        Some(json!({"id":record.id,"stage":stage,
            "atomic":false,"pasted":false,"submitted":submitted,"dispatched":dispatched,"acknowledged":acknowledged})),
    )
}

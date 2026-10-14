//! Guarded app-server sends never write to the TUI composer.
use super::*;

#[derive(Debug, Serialize)]
pub(crate) struct Receipt {
    pub(crate) transport: &'static str,
    pub(crate) confirmation: &'static str,
}

pub(super) fn deliver(
    context: &CliContext,
    args: cli::SendArgs,
    text: String,
    mut record: SessionRecord,
    tmux_bin: &Path,
    started: Instant,
) -> Result<SendResult, CliError> {
    let observed = record.clone();
    let admission = send_guard::Admission {
        context,
        record: &observed,
        args: &args.guards,
        tmux_bin,
        verify_composer: true,
        deadline: started + send_submit::HOLD_BUDGET,
    };
    // This shared gate serializes both proxy turn/start and account mutation.
    // It is nonblocking while the session record lock is owned.
    if text.trim_start().starts_with('/') && text.trim() != "/compact" {
        return Err(refusal(
            &record,
            "send-native-command-unsupported",
            false,
            false,
        ));
    }
    let _account_gate = codex_app_server::acquire_account_mutation_gate(context, &record.id)?;
    codex_account::ensure_proxy_input_allowed(&record)?;
    let mut client = codex_app_server::guarded::Client::connect(&record, admission.deadline)
        .map_err(|code| refusal(&record, code, false, false))?;
    admission.check_key()?;
    admission.check(None, "admission")?;
    client
        .idle()
        .map_err(|code| refusal(&record, code, false, false))?;
    admission.claim_key()?;
    admission.check(None, "admission")?;
    client
        .idle()
        .map_err(|code| refusal(&record, code, false, false))?;
    let mut prepared = false;
    prepare_guarded_input(context, &mut record, true, &mut prepared)?;
    let confirmation = client
        .dispatch(&text)
        .map_err(|error| refusal(&record, "send-native-unconfirmed", true, error.acknowledged))?;
    record.updated_at = Zoned::now().timestamp().to_string();
    write_session_record(context, &record)?;
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
    })
}

fn refusal(record: &SessionRecord, code: &str, dispatched: bool, acknowledged: bool) -> CliError {
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
        Some(
            json!({"id":record.id,"stage":if dispatched {"submit"} else {"admission"},
            "atomic":false,"pasted":false,"submitted":submitted,"dispatched":dispatched,"acknowledged":acknowledged}),
        ),
    )
}

//! Scheduler-owned continuations never enter the manual-input path.
use super::*;

fn unsupported() -> CliError {
    CliError::runtime(
        "notice-transport-unsupported",
        "this runtime has no supported daemon notice transport",
        None,
    )
}

fn envelope(text: &str) -> Result<String, CliError> {
    if !crate::notice::writers_enabled() {
        return Err(CliError::runtime(
            "notice-writers-disabled",
            "notice writers are disabled",
            None,
        ));
    }
    crate::notice::NoticeEnvelope::new("runtime.resume", "agent-session", None)
        .and_then(|metadata| metadata.encode(&crate::notice::generated::body(text)))
        .map_err(|_| unsupported())
}

pub(crate) fn send_auto_resume_input(
    context: &CliContext,
    expected: &SessionRecord,
    record_lock: crate::SessionRecordLock,
    text: &str,
    tmux: &Path,
) -> Result<(), CliError> {
    let mut record = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(expected, &record)?;
    crate::auth_incident::recovery::guard_input(
        context,
        &record,
        Some(text),
        &[SpecialKey::Enter],
    )?;
    let text = envelope(text)?;
    let capability = claude_notice::runtime::readiness(context, &record).ok_or_else(unsupported)?;
    let recovery = auto_resume::notice::claimed(context, &record)?;
    codex_account::authorize_input_locked(context, &mut record)?;
    let mut args = unguarded_args(
        record.id.clone(),
        Some(text.clone()),
        vec![SpecialKey::Enter],
        false,
        tmux.to_owned(),
    );
    args.guards = send_guard::GuardArgs {
        notice_envelope: true,
        require_idle: true,
        require_empty_composer: true,
        expect_runtime: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone()),
        expect_provider_session: record
            .provider_resume
            .as_ref()
            .map(|provider| provider.session_id.clone()),
        idempotency_key: Some(format!("auto-resume-{}", recovery.generation)),
        ..Default::default()
    };
    let started = Instant::now();
    let deadline = started + guarded_hold_budget(&record, false);
    let mut manual = ManualInputSection::new(record_lock);
    claude_notice::deliver(
        context,
        args,
        text,
        record,
        tmux,
        &mut manual,
        native::Attempt {
            started,
            acknowledged: &mut |_, _| {},
            terminal_command: false,
            recovery: Some(recovery),
            notification: None,
        },
        deadline,
        capability,
    )
    .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_codex(
    context: &CliContext,
    expected: &SessionRecord,
    record_lock: crate::SessionRecordLock,
    text: &str,
    tmux: &Path,
    runtime: &tokio::runtime::Handle,
    handle: &codex_app_server::ControlHandle,
) -> Result<(), CliError> {
    let started = Instant::now();
    let deadline = started + codex_app_server::CONTINUATION_BUDGET;
    let record = load_session_record(context, &expected.id)?;
    ensure_same_session_identity(expected, &record)?;
    crate::auth_incident::recovery::guard_input(
        context,
        &record,
        Some(text),
        &[SpecialKey::Enter],
    )?;
    let text = envelope(text)?;
    let recovery = auto_resume::notice::claimed(context, &record)?;
    let _account = codex_app_server::acquire_account_mutation_gate(context, &record.id)?;
    codex_account::ensure_proxy_input_allowed(&record)?;
    let guards = send_guard::GuardArgs {
        notice_envelope: true,
        require_empty_composer: true,
        expect_runtime: record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.clone()),
        expect_provider_session: record
            .provider_resume
            .as_ref()
            .map(|provider| provider.session_id.clone()),
        ..Default::default()
    };
    let admission = send_guard::Admission {
        context,
        record: &record,
        args: &guards,
        tmux_bin: tmux,
        verify_composer: true,
        deadline,
    };
    admission.check_preserving_draft("admission")?;
    let prepared = codex_notice::Prepared::new(
        context,
        &record,
        &text,
        started,
        deadline,
        Some(recovery.clone()),
        || Ok(()),
    )?;
    admission.check_preserving_draft("claimed")?;
    let submitted = runtime.block_on(handle.submit_notice(
        &text,
        prepared.client_id(),
        &record,
        record_lock,
        tmux,
        recovery,
        deadline,
    ));
    match submitted {
        Ok(turn) => prepared.confirm(context, &text, &turn),
        Err(_) => Err(prepared.unknown(context)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn generated_continuation_body_is_formatted_before_shared_encoding() {
        let lock = GlobalStateLock::new();
        let _writers = EnvGuard::set(&lock, crate::notice::WRITERS_ENV, "1");
        let encoded = envelope("  Continue \r\nnext \n").unwrap();
        let notice = crate::notice::parse_notice(&encoded).unwrap();
        assert_eq!(notice.metadata.kind(), "runtime.resume");
        assert_eq!(notice.metadata.source(), "agent-session");
        assert_eq!(notice.body, "  Continue\r\nnext\n");
    }
}

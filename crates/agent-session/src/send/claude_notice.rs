//! Notice-only native input; terminal containment remains the unsupported route.
use crate::{
    CliContext, CliError, ManualInputSection, SessionRecord, cli, send_guard, send_submit,
};
use crate::provider_prompt::native_notice::CaptureError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::cell::Cell;
use std::path::Path;
use std::time::Instant;

mod bridge;
#[cfg(test)]
mod module_contract;
pub(crate) mod runtime;
pub(crate) mod store;

// External Claude Code 2.1.296's generated module contract and native probes
// establish prompt.submit(asUser), prompt.read, timers and HTTP Unix sockets.
pub(crate) const PROVIDER_VERSION: &str = "2.1.296";
pub(crate) const PLUGIN: &str = "agent-session-claude-notice";

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Launch {
    pub(crate) session_id: String,
    pub(crate) incarnation: String,
    pub(crate) generation: u64,
    pub(crate) profile: Option<String>,
}

impl Launch {
    pub(crate) fn from_record(record: &crate::SessionRecord) -> Result<Self, crate::CliError> {
        Ok(Self {
            session_id: record.id.clone(),
            incarnation: crate::coordination::incarnation(record)?,
            generation: record
                .runtime
                .as_ref()
                .ok_or_else(runtime::unavailable)?
                .generation,
            profile: crate::session_agent_profile(record).map(str::to_string),
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    pub(crate) launch: Launch,
    pub(crate) provider: String,
    pub(crate) provider_session_id: String,
    pub(crate) provider_version: String,
    pub(crate) module_sha256: String,
}

fn unconfirmed(record: &SessionRecord, stage: &str) -> CliError {
    CliError::runtime(
        "notice-native-unconfirmed",
        "native notice input is unconfirmed; do not replay it",
        Some(
            json!({"id":record.id,"guard":"transport","stage":stage,"atomic":false,
            "pasted":false,"submitted":null}),
        ),
    )
}

// Pass the original guarded attempt and lock through; no second send context.
#[allow(clippy::too_many_arguments)]
pub(super) fn deliver(
    context: &CliContext,
    args: cli::SendArgs,
    text: String,
    mut record: SessionRecord,
    tmux: &Path,
    manual: &mut ManualInputSection,
    attempt: super::native::Attempt<'_>,
    deadline: Instant,
    (bootstrap, ready): (runtime::Bootstrap, runtime::Ready),
) -> Result<super::SendResult, CliError> {
    let observed = record.clone();
    let admission = send_guard::Admission {
        context,
        record: &observed,
        args: &args.guards,
        tmux_bin: tmux,
        verify_composer: true,
        deadline,
    };
    if text.len() > crate::provider_prompt::MAX_PROVIDER_PROMPT_BYTES
        || text.contains('\r')
        || crate::notice::parse_notice(&text).is_none()
    {
        return Err(CliError::runtime(
            "notice-native-text-unsupported",
            "native notice requires a valid LF-only text envelope of at most 16 KiB",
            Some(
                json!({"id":record.id,"guard":"text","stage":"admission","atomic":false,"pasted":false,"submitted":false}),
            ),
        ));
    }
    let mut baseline = crate::provider_prompt::native_notice::Baseline::capture(&record)
        .map_err(|error| match error {
            CaptureError::NoTranscript => runtime::baseline_unavailable(&record),
            CaptureError::Unavailable => runtime::unavailable(),
        })?;
    let dir = runtime::module_dir(&context.state_dir, &record)?;
    let op = uuid::Uuid::new_v4().to_string();
    let listener = bridge::Listener::open(dir, &bootstrap, &ready.binding, op.clone()).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            CliError::runtime("send-session-not-idle", "another native notice is in progress",
                Some(json!({"id":record.id,"guard":"idle","stage":"admission","atomic":false,"pasted":false,"submitted":false})))
        } else { runtime::unavailable() }
    })?;
    let consumed = Cell::new(false);
    let mut token = None;
    let mut stage = "admission";
    let mut failure_reason = store::Reason::Guard;
    let result = (|| {
        let connection = listener
            .receive(deadline)
            .map_err(|_| runtime::unavailable())?;
        let request: bridge::Authorize = connection.parse().map_err(|_| runtime::unavailable())?;
        if connection.path != "/authorize"
            || request.schema_version != "agent-session.claude-notice-authorize.v1"
            || request.op != op
            || request.binding != ready.binding
            || request.nonce != bootstrap.nonce
            || !request.terminal
            || !request.box_empty
            || request.cursor != 0
        {
            return Err(runtime::unavailable());
        }
        // A live, exact native request is necessary before consuming any key.
        admission.check_key()?;
        admission.check(None, "admission")?;
        if runtime::readiness(context, &record)
            .is_none_or(|(boot, now)| boot.nonce != bootstrap.nonce || now.binding != ready.binding)
        {
            return Err(runtime::unavailable());
        }
        let claim = store::Claim::new(
            op.clone(),
            ready.binding.clone(),
            &text,
            args.guards.idempotency_key.as_deref(),
            baseline.stamp.clone(),
            deadline.saturating_duration_since(attempt.started),
        );
        token = Some(store::consume(context, claim, deadline, || {
            consumed.set(true);
            admission.claim_key()
        })?);
        stage = "claimed";
        admission.check(None, "claimed")?;
        let mut prepared = false;
        super::prepare_guarded_input(context, &mut record, true, &mut prepared)?;
        record.updated_at = jiff::Zoned::now().timestamp().to_string();
        crate::write_session_record(context, &record)?;
        admission.check(None, "claimed")?;
        bridge::remaining(deadline).map_err(|_| runtime::unavailable())?;
        // Required synchronous provider hooks must be able to acquire this
        // record lock BEFORE the module enters prompt.submit.
        drop(manual.record_lock.take());
        stage = "submit";
        connection
            .respond(
                &json!({"schema_version":"agent-session.claude-notice-authorized.v1",
            "op":op,"binding":ready.binding,"envelope_bytes":text.len(),
            "envelope_sha256":sha256(text.as_bytes()),"text":text}),
                deadline,
            )
            .map_err(|_| runtime::unavailable())?;
        let connection = listener
            .receive(deadline)
            .map_err(|_| runtime::unavailable())?;
        let report: bridge::Report = connection.parse().map_err(|_| runtime::unavailable())?;
        if connection.path != "/result"
            || report.schema_version != "agent-session.claude-notice-result.v1"
            || report.op != op
            || report.binding != ready.binding
            || report.nonce != bootstrap.nonce
        {
            return Err(runtime::unavailable());
        }
        let accepted = report.outcome == "accepted"
            && report.after_empty
            && report.returned_text.as_deref() == Some(&text)
            && report.origin.as_ref().is_some_and(|origin| {
                origin.kind == "plugin" && origin.name == PLUGIN && origin.as_user
            });
        if !accepted {
            failure_reason = if report.outcome == "dropped"
                && report.returned_text.is_none()
                && report.origin.is_none()
            {
                store::Reason::Drop
            } else {
                store::Reason::ApiResultMismatch
            };
            connection
                .respond(&json!({"received":true}), deadline)
                .map_err(|_| runtime::unavailable())?;
            return Err(runtime::unavailable());
        }
        connection
            .respond(&json!({"received":true}), deadline)
            .map_err(|_| runtime::unavailable())?;
        stage = "confirmation";
        failure_reason = store::Reason::HistoryUnavailable;
        let proof = baseline
            .confirm(&ready.binding, &text, deadline)
            .ok_or_else(runtime::unavailable)?;
        let lock = crate::acquire_session_record_lock_timed(
            context,
            &record.id,
            bridge::remaining(deadline).map_err(|_| runtime::unavailable())?,
        )
        .map_err(|_| runtime::unavailable())?;
        failure_reason = store::Reason::IdentityChanged;
        admission.identity("confirmation")?;
        let current = crate::load_session_record(context, &record.id)?;
        if Launch::from_record(&current)? != ready.binding.launch
            || current.provider_resume.as_ref().is_none_or(|provider| {
                provider.provider != "claude"
                    || provider.session_id != ready.binding.provider_session_id
            })
        {
            return Err(runtime::unavailable());
        }
        manual.record_lock = Some(lock);
        bridge::remaining(deadline).map_err(|_| runtime::unavailable())?;
        failure_reason = store::Reason::Storage;
        store::finish(
            context,
            token.as_ref().ok_or_else(runtime::unavailable)?,
            Some(&proof),
            store::Reason::Guard,
            attempt.started.elapsed(),
            deadline,
        )?;
        bridge::remaining(deadline).map_err(|_| runtime::unavailable())?;
        Ok(super::SendResult {
            id: record.id.clone(),
            tmux_session: record.tmux_session.clone(),
            sent_text: true,
            keys: args
                .keys
                .iter()
                .map(|key| key.as_str().to_string())
                .collect(),
            submission: Some(send_submit::SubmitReport {
                outcome: send_submit::SubmitOutcome::Submitted,
                enter_presses: 0,
            }),
            guards: Some(send_guard::GuardReport { atomic: false }),
            native: Some(super::native::Receipt {
                transport: "claude_mod",
                confirmation: "notice_row_observed",
            }),
            command_dispatched: false,
        })
    })();
    match result {
        Ok(value) => Ok(value),
        Err(error) if error.code() == "send-idempotency-key-used" && token.is_none() => Err(error),
        Err(_) if consumed.get() => {
            if let Some(token) = &token {
                let _ = store::finish(
                    context,
                    token,
                    None,
                    failure_reason,
                    attempt.started.elapsed(),
                    deadline,
                );
            }
            Err(unconfirmed(&record, stage))
        }
        Err(error) => Err(error),
    }
}

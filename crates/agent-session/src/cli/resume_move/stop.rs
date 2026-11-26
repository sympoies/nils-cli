//! Owner-local graceful stop. Only fresh stopped evidence can authorize export.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::activity::{self, Confidence, SourceKind, TurnPhase};
use crate::codex_app_server::{self, guarded};
use crate::restart::{self, RestartFence, ResumeStatus};
use crate::send_guard::{Admission, GuardArgs};
use crate::{CliContext, CliError, SessionRecord, start_resume};

use super::{Provider, Refusal, canonical_uuid, source, source_fence};

#[derive(PartialEq, Eq, Serialize)]
pub(super) struct SourceIdentity {
    pub(super) session_id: String,
    pub(super) session_incarnation: String,
    pub(super) session_generation: u64,
    pub(super) provider: Provider,
    pub(super) provider_session_id: String,
    pub(super) account: String,
    pub(super) account_binding_revision: u64,
    #[serde(skip)]
    pub(super) provider_root: PathBuf,
}

#[derive(Serialize)]
pub(super) struct VerifiedStoppedSource {
    outcome: &'static str,
    #[serde(flatten)]
    pub(super) identity: SourceIdentity,
}

impl VerifiedStoppedSource {
    pub(super) fn observe(
        context: &CliContext,
        record: &SessionRecord,
        tmux: &Path,
    ) -> Result<Self, CliError> {
        start_resume::prove_stopped(context, record, tmux)?;
        Ok(Self {
            outcome: "stopped",
            identity: source_identity(record)?,
        })
    }
}

fn source_identity(record: &SessionRecord) -> Result<SourceIdentity, CliError> {
    let runtime = record
        .runtime
        .as_ref()
        .filter(|runtime| canonical_uuid(&runtime.launch_id) && runtime.generation > 0)
        .ok_or_else(|| unknown("runtime-identity"))?;
    let conversation = record
        .provider_resume
        .as_ref()
        .filter(|resume| resume.provider == record.agent && canonical_uuid(&resume.session_id))
        .ok_or_else(|| unknown("provider-conversation"))?;
    let (provider, account, revision, provider_root) = source::binding(record)?;
    Ok(SourceIdentity {
        session_id: record.id.clone(),
        session_incarnation: runtime.launch_id.clone(),
        session_generation: runtime.generation,
        provider,
        provider_session_id: conversation.session_id.clone(),
        account,
        account_binding_revision: revision,
        provider_root,
    })
}

pub(super) fn stop(
    context: &CliContext,
    id: &str,
    incarnation: &str,
) -> Result<VerifiedStoppedSource, CliError> {
    if crate::validate_id(id).is_err() || !canonical_uuid(incarnation) {
        return Err(mismatch());
    }
    let tmux = crate::resolve_tmux_bin(None);
    let expected = read_exact(context, id, incarnation)?;
    let expected_source = source_identity(&expected).map_err(|_| unknown("source-binding"))?;
    if std::env::var("AGENT_SESSION_ID").is_ok_and(|own| own == id) {
        return Err(busy("own-runtime"));
    }
    // Match export's conversation-before-record order. Never keep that lock
    // or the record, ingress, or provider locks while exit hooks are running.
    let (_fence, _recovery) = {
        let _conversation = start_resume::raw_conversation_lock(
            context,
            expected_source.provider.agent(),
            &expected_source.provider_session_id,
        )
        .map_err(|_| unknown("conversation-lock"))?;
        let _record = crate::try_acquire_session_record_lock(context, id)
            .map_err(|_| unknown("record-lock"))?
            .ok_or_else(|| busy("lifecycle"))?;
        let current = read_exact(context, id, incarnation)?;
        revalidate(&expected, &expected_source, &current)?;
        restart::ensure_not_fenced(context, &current).map_err(|_| busy("lifecycle"))?;
        if current.mode != "interactive" {
            return Err(unknown("source-mode"));
        }
        if crate::runtime_environment::boot_recovery::shutdown_mark_armed(&current) {
            return Err(busy("boot-recovery"));
        }
        match restart::resume_status(&tmux, &current) {
            ResumeStatus::Stopped => {
                let receipt = VerifiedStoppedSource::observe(context, &current, &tmux)
                    .map_err(|_| unknown("stopped-proof"))?;
                return Err(CliError::data(
                    Refusal::SourceAlreadyStopped.code(),
                    "the exact source is already stopped",
                    Some(json!({"next_action":"export","receipt":receipt})),
                ));
            }
            ResumeStatus::Unknown { .. } => return Err(unknown("tmux-status")),
            ResumeStatus::Running => {}
        }
        source_fence::ensure_unfenced(
            context,
            expected_source.provider.agent(),
            &expected_source.provider_session_id,
        )
        .map_err(|_| unknown("move-fence"))?;
        // Export requires the exact coordination owner to close on exit. If
        // it was never registered, stopping the pane cannot create that proof.
        match crate::coordination::auth_effects::resume_source_closed(context, &current) {
            Err(error) if error.code() == "resume-source-live" => {}
            _ => return Err(unknown("source-ownership")),
        }
        if !matches!(crate::persisted_tmux_termination_state(&current), Ok(None)) {
            return Err(unknown("termination-state"));
        }
        crate::auth_incident::recovery::ensure_input_allowed(context, &current)
            .map_err(|_| unknown("auth-recovery"))?;
        source::find_history(
            expected_source.provider,
            &expected_source.provider_root,
            &expected_source.provider_session_id,
        )
        .map_err(|_| unknown("provider-history"))?;

        let recovery = crate::auto_resume::notice::try_attempt(context, id)
            .map_err(|_| unknown("recovery-lock"))?
            .ok_or_else(|| busy("auto-resume"))?;
        let fence = admit_and_exit(context, &current, &expected_source, &tmux)?;
        (fence, recovery)
    };

    let wait = restart::await_retained_exit(context, &expected, &tmux);
    let _record = crate::try_acquire_session_record_lock(context, id)
        .map_err(|_| unknown("record-lock"))?
        .ok_or_else(|| unknown("record-lock"))?;
    let current = read_exact(context, id, incarnation)?;
    revalidate(&expected, &expected_source, &current)?;
    if let Some(cause) = wait {
        return Err(if cause == "claude-background-work" {
            busy(cause)
        } else {
            unknown(cause)
        });
    }
    VerifiedStoppedSource::observe(context, &current, &tmux).map_err(|_| unknown("stopped-proof"))
}

fn read_exact(
    context: &CliContext,
    id: &str,
    incarnation: &str,
) -> Result<SessionRecord, CliError> {
    let record = crate::load_session_record(context, id).map_err(|_| unknown("source-record"))?;
    if record.id != id
        || record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.launch_id != incarnation)
    {
        Err(mismatch())
    } else {
        Ok(record)
    }
}

fn revalidate(
    expected: &SessionRecord,
    expected_source: &SourceIdentity,
    current: &SessionRecord,
) -> Result<(), CliError> {
    crate::ensure_same_session_identity(expected, current).map_err(|_| mismatch())?;
    if source_identity(current).map_err(|_| unknown("source-binding"))? != *expected_source {
        return Err(mismatch());
    }
    Ok(())
}

fn admit_and_exit(
    context: &CliContext,
    record: &SessionRecord,
    expected: &SourceIdentity,
    tmux: &Path,
) -> Result<RestartFence, CliError> {
    let native = codex_app_server::runtime_is_supported(record);
    let (_provider, _requests) = if native {
        if !codex_app_server::retirement::supported(context, record) {
            return Err(unknown("provider-request-fence"));
        }
        crate::codex_account::ensure_proxy_input_allowed(record)
            .map_err(|_| unknown("source-binding"))?;
        let provider = codex_app_server::acquire_account_mutation_gate(context, &record.id)
            .map_err(|error| {
                if error.code() == "codex-account-session-busy" {
                    busy("provider-turn")
                } else {
                    unknown("provider-turn-fence")
                }
            })?;
        let requests = codex_app_server::retirement::probe_requests(context, record)
            .map_err(|_| unknown("provider-request-fence"))?
            .ok_or_else(|| busy("provider-request"))?;
        (Some(provider), Some(requests))
    } else {
        (None, None)
    };

    let _activity = activity::acquire_coordination_activity_lock(context, &record.id)
        .map_err(|_| unknown("activity-lock"))?;
    let _ingress = activity::pending::probe_lock(context, &record.id)
        .map_err(|_| unknown("provider-ingress"))?
        .ok_or_else(|| busy("provider-ingress"))?;
    if activity::pending::unsettled(context, record).ok_or_else(|| unknown("provider-ingress"))? {
        return Err(busy("provider-ingress"));
    }
    require_waiting(context, record)?;

    let deadline = Instant::now() + crate::send::guarded_hold_budget(record, false);
    if native {
        let thread = guarded::Client::connect(record, deadline)
            .and_then(|mut client| client.idle_thread(false))
            .map_err(|error| match error {
                guarded::Error::NotIdle => busy("provider-turn"),
                _ => unknown("provider-idle"),
            })?;
        if thread
            .pointer("/thread/status/type")
            .and_then(Value::as_str)
            != Some("idle")
        {
            return Err(unknown("provider-idle"));
        }
    }
    // A missing persisted process identity would leave no way to prove a
    // stopped source. Check it against the current pane before issuing exit.
    let saved = crate::persisted_tmux_runtime_identity(record)
        .ok()
        .flatten()
        .ok_or_else(|| unknown("persisted-process-identity"))?;
    match crate::capture_tmux_runtime_identity(context, record, tmux, Duration::from_secs(1)) {
        Ok(crate::TmuxRuntimeProbe::Running(live))
            if saved.same_runtime_target(&live) && saved.same_process_identity(&live) => {}
        _ => return Err(unknown("live-process-identity")),
    }
    let guard = Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &GuardArgs {
            expect_runtime: Some(expected.session_incarnation.clone()),
            expect_provider_session: Some(expected.provider_session_id.clone()),
            require_empty_composer: true,
            ..Default::default()
        },
        tmux_bin: tmux,
        verify_composer: false,
        deadline,
    };
    guard
        .check_preserving_draft("admission")
        .map_err(|error| match error.code() {
            "send-composer-not-empty" => busy("composer-draft"),
            _ => unknown("composer-or-runtime"),
        })?;
    if expected.provider == Provider::Claude {
        restart::check_exit_background_work(tmux, record).map_err(|error| {
            if error
                .details()
                .and_then(|details| details["cause"].as_str())
                == Some("claude-background-work")
            {
                busy("claude-background-work")
            } else {
                unknown("background-work")
            }
        })?;
    }
    let fence =
        RestartFence::acquire(context, record, &expected.session_incarnation).map_err(|error| {
            match error.code() {
                "restart-in-progress" | "coordination-notification-submission-in-progress" => {
                    busy("lifecycle-or-notification")
                }
                _ => unknown("input-fence"),
            }
        })?;
    restart::send_provider_exit(context, record, tmux, Some(&saved))
        .map_err(|_| unknown("exit-submission"))?;
    Ok(fence)
}

fn require_waiting(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let state = activity::state_view::bound_provider_state(context, record)
        .ok_or_else(|| unknown("provider-activity"))?;
    if matches!(
        state.phase,
        TurnPhase::Working | TurnPhase::NeedsInput | TurnPhase::Starting
    ) || state.current_turn.is_some()
    {
        return Err(busy("provider-activity"));
    }
    if state.phase == TurnPhase::Waiting
        && state.source.kind == SourceKind::ProviderHook
        && state.source.confidence == Confidence::Authoritative
    {
        return Ok(());
    }
    Err(unknown("provider-activity"))
}

fn busy(cause: &'static str) -> CliError {
    CliError::data(
        Refusal::SourceBusy.code(),
        "the source cannot stop while input, recovery, or provider work is active",
        Some(json!({"cause":cause,"next_action":"wait-for-idle-or-resolve-input"})),
    )
}

fn mismatch() -> CliError {
    CliError::data(
        Refusal::SessionMismatch.code(),
        "the source does not match the exact session and incarnation selector",
        Some(json!({"next_action":"refresh-source-selector"})),
    )
}

fn unknown(cause: &'static str) -> CliError {
    CliError::data(
        "resume-source-unknown",
        "the exact source could not be verified safe to stop or proven stopped",
        Some(json!({"cause":cause,"next_action":"inspect-source-state"})),
    )
}

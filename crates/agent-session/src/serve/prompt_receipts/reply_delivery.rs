//! Observe and consume one idle-ready boundary at the guarded input fence.

use super::idle_boundary::{Boundary, IdleObservation, Store};
use crate::codex_app_server::{self, guarded};
use crate::send_guard::{Admission, GuardArgs};
use crate::{CliContext, CliError, SessionRecord, SessionRecordLock, SpecialKey, activity};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub(in crate::serve) const CAPABILITY: &str = "agent-session.reply-delivery.idle-boundary.v1";

const OBSERVATION_BUDGET: Duration = Duration::from_secs(1);
const MAX_OBSERVATIONS: usize = 4;

#[derive(Default)]
pub(in crate::serve) struct ListObserver {
    running: AtomicBool,
    next_session: AtomicUsize,
}

impl ListObserver {
    /// The request reads only durable snapshots. One detached batch owns a
    /// shared deadline, with independent blocking probes for its sessions.
    pub(in crate::serve) fn schedule(
        self: &Arc<Self>,
        context: CliContext,
        tmux: PathBuf,
        candidates: Vec<(String, String)>,
    ) {
        if candidates.is_empty() || self.running.swap(true, Ordering::AcqRel) {
            return;
        }
        let limit = candidates.len().min(MAX_OBSERVATIONS);
        let start = self.next_session.fetch_add(limit, Ordering::Relaxed) % candidates.len();
        let observer = self.clone();
        let deadline = Instant::now() + OBSERVATION_BUDGET;
        tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            for offset in 0..limit {
                let (id, incarnation) = candidates[(start + offset) % candidates.len()].clone();
                let context = context.clone();
                let tmux = tmux.clone();
                tasks.spawn_blocking(move || {
                    observe(&context, &tmux, &id, &incarnation, deadline);
                });
            }
            while tasks.join_next().await.is_some() {}
            observer.running.store(false, Ordering::Release);
        });
    }
}

pub(super) fn valid_token(token: &str) -> bool {
    uuid::Uuid::parse_str(token).ok().is_some_and(|id| {
        id.get_version() == Some(uuid::Version::Random) && id.hyphenated().to_string() == token
    })
}

pub(in crate::serve) fn projection(
    context: &CliContext,
    record: &SessionRecord,
) -> Option<Option<Boundary>> {
    let incarnation = &record.runtime.as_ref()?.launch_id;
    let binding = identity(context, record)?.binding;
    Some(
        super::idle_boundary::projected(context, &record.id, incarnation, &binding)
            .ok()
            .flatten(),
    )
}

struct VerifiedIdentity {
    binding: String,
    conversation: String,
}

fn identity(context: &CliContext, record: &SessionRecord) -> Option<VerifiedIdentity> {
    use crate::codex_account::BindingSnapshot;
    let runtime = record
        .runtime
        .as_ref()
        .filter(|runtime| !runtime.launch_id.is_empty())?;
    let provider = record
        .provider_resume
        .as_ref()
        .filter(|provider| provider.provider == record.agent && !provider.session_id.is_empty())?;
    let conversation = super::store::digest(&(&provider.provider, &provider.session_id));
    if codex_app_server::runtime_is_supported(record) {
        if !matches!(
            crate::codex_account::binding_snapshot(record),
            BindingSnapshot::Bound { .. }
        ) {
            return None;
        }
        let control = guarded::reply_control_token(context, record)?;
        let identity = super::native_identity(record)?;
        return Some(VerifiedIdentity {
            binding: super::store::digest(&("codex", identity, control)),
            conversation,
        });
    }
    if !crate::claude_structured_prompt_supported(record) {
        return None;
    }
    let account = crate::claude_account::applied_identity(record)?;
    Some(VerifiedIdentity {
        binding: super::store::digest(&(
            "claude",
            &record.id,
            &record.created_at,
            &runtime.launch_id,
            runtime.generation,
            &runtime.kind,
            &runtime.tmux_session,
            &record.tmux_session,
            &provider.session_id,
            account,
        )),
        conversation,
    })
}

fn observe(context: &CliContext, tmux: &Path, id: &str, expected: &str, deadline: Instant) {
    if Instant::now() >= deadline {
        return;
    }
    let Ok(Some(lock)) = crate::try_acquire_session_record_lock(context, id) else {
        return;
    };
    let Ok(record) = crate::load_session_record(context, id) else {
        return;
    };
    let Some(runtime) = &record.runtime else {
        return;
    };
    if runtime.launch_id != expected {
        return;
    }
    let store = Store::new(context, &record.id, expected, &lock);
    match probe(context, &record, tmux, deadline) {
        Ok(observation) if Instant::now() < deadline => {
            let _ = store.observe(observation);
        }
        _ => {
            let _ = store.invalidate();
        }
    }
}

fn probe(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
) -> Result<IdleObservation, CliError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero()
        || crate::live_status_with_timeout(tmux, &record.tmux_session, remaining) != "running"
    {
        return Err(stale_identity());
    }
    crate::restart::ensure_not_fenced(context, record)?;
    crate::auth_incident::recovery::ensure_input_allowed(context, record)?;
    let args = GuardArgs {
        require_idle: true,
        require_empty_composer: true,
        ..Default::default()
    };
    let admission = Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &args,
        tmux_bin: tmux,
        verify_composer: true,
        deadline,
    };
    if codex_app_server::runtime_is_supported(record) {
        let _account = codex_app_server::acquire_account_mutation_gate(context, &record.id)?;
        crate::codex_account::ensure_proxy_input_allowed(record)?;
        let identity = identity(context, record).ok_or_else(stale_identity)?;
        admission.check_preserving_draft("admission")?;
        let mut client = guarded::Client::connect(record, deadline).map_err(native_error)?;
        return native_observation(identity, &mut client);
    }
    admission.check_preserving_draft("admission")?;
    crate::cli::permission::refuse_terminal(context, record, Some("prompt"), &[SpecialKey::Enter])?;
    terminal_observation(
        context,
        record,
        identity(context, record).ok_or_else(stale_identity)?,
        admission.stopped_idle.borrow().is_some(),
    )
}

fn native_observation(
    identity: VerifiedIdentity,
    client: &mut guarded::Client,
) -> Result<IdleObservation, CliError> {
    let (position, turn) = client
        .idle_watermark()
        .map_err(native_error)?
        .ok_or_else(unobservable)?;
    Ok(IdleObservation {
        binding: identity.binding,
        conversation: identity.conversation,
        position,
        turn: super::store::digest(&turn),
    })
}

fn terminal_observation(
    context: &CliContext,
    record: &SessionRecord,
    identity: VerifiedIdentity,
    admitted_stop: bool,
) -> Result<IdleObservation, CliError> {
    let state = activity::bound_claude_state(context, record).ok_or_else(unobservable)?;
    if !(state.phase == activity::TurnPhase::Waiting && state.current_turn.is_none()
        || admitted_stop && state.phase == activity::TurnPhase::Working)
    {
        return Err(unobservable());
    }
    let (time, turn) = if let Some(current) = &state.current_turn {
        (
            Some(current.started_at.as_str()),
            current.provider_turn_id.as_deref(),
        )
    } else if let Some(last) = &state.last_turn {
        (
            last.started_at.as_deref().or(Some(&last.completed_at)),
            last.provider_turn_id.as_deref(),
        )
    } else {
        (None, None)
    };
    let position = match (time, turn) {
        (None, None) => 0,
        (Some(time), Some(_)) => time
            .parse::<jiff::Timestamp>()
            .ok()
            .and_then(|time| u64::try_from(time.as_nanosecond()).ok())
            .ok_or_else(unobservable)?,
        _ => return Err(unobservable()),
    };
    Ok(IdleObservation {
        binding: identity.binding,
        conversation: identity.conversation,
        position,
        turn: super::store::digest(&turn),
    })
}

pub(crate) fn check(
    context: &CliContext,
    record: &SessionRecord,
    args: &GuardArgs,
    lock: &SessionRecordLock,
) -> Result<(), CliError> {
    let Some(token) = args.expected_idle_ready_boundary.as_deref() else {
        return Ok(());
    };
    let store = request_store(context, record, args, lock)?;
    store.check(token)?;
    if identity(context, record).is_none() {
        store.invalidate()?;
        return Err(stale_identity());
    }
    Ok(())
}

/// All callers reach this before pane or provider input. Preserve the existing
/// stage rules for ordinary guarded attempts.
pub(crate) fn checked<T>(
    context: &CliContext,
    record: &SessionRecord,
    args: &GuardArgs,
    lock: &SessionRecordLock,
    result: Result<T, CliError>,
) -> Result<T, CliError> {
    if args.expected_idle_ready_boundary.is_some() && result.is_err() {
        request_store(context, record, args, lock)?.invalidate()?;
        return result.map_err(admission_error);
    }
    result
}

pub(crate) enum AdmittedInput<'a> {
    Native(&'a mut guarded::Client),
    Terminal {
        tmux: &'a Path,
        text: &'a str,
        stopped: bool,
    },
}

pub(crate) fn consume(
    context: &CliContext,
    record: &SessionRecord,
    args: &GuardArgs,
    lock: &SessionRecordLock,
    input: AdmittedInput<'_>,
) -> Result<Option<std::fs::File>, CliError> {
    // Terminal permission admission acquires the exclusive ingress probe.
    // Return it to the paste caller so the probe spans consumption and input.
    let ingress = match &input {
        AdmittedInput::Terminal { tmux, text, .. } => checked(
            context,
            record,
            args,
            lock,
            crate::cli::permission::input_admission(context, record, tmux, Some(text), &[]),
        )?,
        AdmittedInput::Native(_) => None,
    };
    let Some(token) = args.expected_idle_ready_boundary.as_deref() else {
        return Ok(ingress);
    };
    let store = request_store(context, record, args, lock)?;
    let observed = identity(context, record)
        .ok_or_else(stale_identity)
        .and_then(|identity| match input {
            AdmittedInput::Native(client) => {
                let observation = native_observation(identity, client)?;
                if activity::pending::event_pending(context, &record.id).map_err(admission_error)? {
                    return Err(refused(
                        "send-provider-event-pending",
                        "provider event ingestion is pending",
                    ));
                }
                Ok(observation)
            }
            AdmittedInput::Terminal { stopped, .. } => {
                terminal_observation(context, record, identity, stopped)
            }
        });
    match observed {
        Ok(observation) => store.consume(token, &observation).map(|()| ingress),
        Err(error) => {
            store.invalidate()?;
            Err(error)
        }
    }
}

fn request_store<'a>(
    context: &CliContext,
    record: &SessionRecord,
    args: &'a GuardArgs,
    lock: &'a SessionRecordLock,
) -> Result<Store<'a>, CliError> {
    let incarnation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str());
    let expected = args.expect_runtime.as_deref();
    match incarnation.zip(expected) {
        Some((incarnation, expected)) if incarnation == expected => {
            Ok(Store::new(context, &record.id, expected, lock))
        }
        _ => Err(refused(
            "session-incarnation-conflict",
            "the expected session incarnation has changed",
        )),
    }
}

fn native_error(error: guarded::Error) -> CliError {
    error.annotate(refused(
        error.code(),
        "native idle readiness could not be verified",
    ))
}

fn stale_identity() -> CliError {
    refused(
        "stale-idle-ready-boundary",
        "the boundary's runtime identity is unavailable",
    )
}

fn unobservable() -> CliError {
    refused(
        "send-idle-unobservable",
        "the provider turn watermark could not be verified",
    )
}

fn refused(code: &str, message: &str) -> CliError {
    admission_error(CliError::runtime(code, message, None))
}

fn admission_error(error: CliError) -> CliError {
    let mut details = error.details().cloned().unwrap_or_else(|| json!({}));
    details["stage"] = json!("admission");
    details["pasted"] = json!(false);
    details["submitted"] = json!(false);
    details["dispatched"] = json!(false);
    CliError::runtime(error.code(), error.message(), Some(details))
}

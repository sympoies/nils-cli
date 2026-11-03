//! Passive response observation for existing guarded terminal key actions.
use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
use std::{
    fs,
    path::{Path, PathBuf},
};

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{capture_styled, drafts, send_submit};
use crate::{CliContext, CliError, SessionRecord, activity, session_dir};

impl super::Admission<'_> {
    pub(crate) fn send_key(&self, key: crate::SpecialKey) -> Result<(), CliError> {
        let target = format!("{}:0.0", self.record.tmux_session);
        let action = || crate::send_tmux_key(self.tmux_bin, &target, key);
        if key == crate::SpecialKey::Enter {
            watch(
                self.context,
                self.record,
                self.tmux_bin,
                Operation::Submit,
                action,
            )
        } else {
            action()
        }
    }
}

pub(crate) const BOUND_ENV: &str = "AGENT_SESSION_UNRESPONSIVE_SECONDS";
pub(crate) const DEFAULT_BOUND: u64 = 4;
const OBSERVATION: &str = "pane-response.json";
const EVENT: &str = "pane-response-event.json";

pub(crate) fn parse_bound(raw: &str) -> Result<u64, CliError> {
    raw.parse::<u64>()
        .ok()
        .filter(|seconds| *seconds > 0 && raw.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| {
            CliError::usage(
                "invalid-unresponsive-bound",
                "unresponsive bound must be positive integer seconds",
                None,
            )
        })
}

fn bound() -> Result<u64, CliError> {
    match std::env::var(BOUND_ENV) {
        Ok(raw) => parse_bound(&raw),
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_BOUND),
        Err(_) => parse_bound(""),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Operation {
    Submit,
    Clear,
}

#[derive(Serialize, Deserialize)]
struct Observation {
    identity: String,
    pane: String,
    // Keep the reader selected by this delivery, including an explicit CLI
    // binary selection. This private path is never projected to consumers.
    tmux_bin: PathBuf,
    started_at: String,
    bound_seconds: u64,
    operation: Operation,
    observed_at: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    schema_version: String,
    event: String,
    session_id: String,
    session_incarnation: String,
    started_at: String,
    observed_at: String,
    bound_seconds: u64,
    operation: Operation,
}

fn identity(record: &SessionRecord) -> Option<String> {
    let runtime = record.runtime.as_ref().filter(|r| r.kind == "tmux")?;
    Some(digest(
        &serde_json::to_vec(&(
            &runtime.launch_id,
            runtime.generation,
            &record.agent,
            record
                .provider_resume
                .as_ref()
                .map(|r| (&r.provider, &r.session_id)),
        ))
        .ok()?,
    ))
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn pane(tmux_bin: &Path, record: &SessionRecord) -> Option<(String, bool)> {
    let target = format!("{}:0.0", record.tmux_session);
    let pane = capture_styled(tmux_bin, &target, crate::PANE_OBSERVATION_COMMAND_TIMEOUT)?;
    let input = send_submit::exact_composer_input(&record.agent, &pane)?;
    Some((digest(pane.as_bytes()), input.is_empty()))
}

fn idle(context: &CliContext, record: &SessionRecord, state: &activity::TurnState) -> bool {
    state.phase == activity::TurnPhase::Waiting
        && state.current_turn.is_none()
        && matches!(
            activity::pending::event_pending(context, &record.id),
            Ok(false)
        )
}

// A separate nonblocking lock fences view readers against an in-flight key
// observation without taking the record/activity locks held by those callers.
fn lock(context: &CliContext, record: &SessionRecord) -> Option<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(session_dir(context, &record.id).join("pane-response.lock"))
        .ok()?;
    // SAFETY: file owns the descriptor; dropping it releases the flock.
    (unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0).then_some(file)
}

/// Called only after existing admission and immediately before an existing key.
/// Observation failure never invents pane evidence or adds a recovery action.
pub(crate) fn watch<T>(
    context: &CliContext,
    record: &SessionRecord,
    tmux_bin: &Path,
    operation: Operation,
    action: impl FnOnce() -> Result<T, CliError>,
) -> Result<T, CliError> {
    let seconds = bound()?;
    let Some(_lock) = lock(context, record) else {
        return action();
    };
    let path = session_dir(context, &record.id).join(OBSERVATION);
    let state = activity::controller_state_for_record(context, record);
    let evidence = state
        .as_ref()
        .filter(|s| idle(context, record, s))
        .and_then(|_| Some((identity(record)?, pane(tmux_bin, record)?)));
    if let Some((identity, (pane, false))) = evidence {
        let previous = drafts::read_json::<Observation>(&path, 4096)
            .ok()
            .filter(|p| p.identity == identity && p.pane == pane);
        let observation = Observation {
            identity,
            pane,
            tmux_bin: tmux_bin.to_owned(),
            started_at: previous
                .as_ref()
                .map(|p| p.started_at.clone())
                .unwrap_or_else(|| Timestamp::now().to_string()),
            bound_seconds: seconds,
            operation,
            observed_at: previous.and_then(|p| p.observed_at),
        };
        drafts::persist(&path, &observation)?;
    } else {
        let _ = fs::remove_file(path);
    }
    action()
}

pub(crate) fn project(
    context: &CliContext,
    record: &SessionRecord,
    state: &mut activity::TurnState,
) {
    let path = session_dir(context, &record.id).join(OBSERVATION);
    if !path.exists() {
        return;
    }
    let Some(_lock) = lock(context, record) else {
        return;
    };
    let Ok(mut observation) = drafts::read_json::<Observation>(&path, 4096) else {
        return;
    };
    let unchanged = identity(record).as_deref() == Some(&observation.identity)
        && idle(context, record, state)
        && pane(&observation.tmux_bin, record).is_some_and(|(pane, _)| pane == observation.pane);
    if !unchanged {
        let _ = fs::remove_file(path);
        return;
    }
    let Ok(started) = observation.started_at.parse::<Timestamp>() else {
        return;
    };
    let now = Timestamp::now();
    if now.as_nanosecond() - started.as_nanosecond()
        < i128::from(observation.bound_seconds) * 1_000_000_000
    {
        return;
    }
    let newly_observed = observation.observed_at.is_none();
    let observed_at = observation
        .observed_at
        .get_or_insert_with(|| now.to_string())
        .clone();
    let event = Event {
        schema_version: "agent-session.pane-response.v1".into(),
        event: "pane_unresponsive".into(),
        session_id: record.id.clone(),
        session_incarnation: record
            .runtime
            .as_ref()
            .map(|r| r.launch_id.clone())
            .unwrap_or_default(),
        started_at: observation.started_at.clone(),
        observed_at: observed_at.clone(),
        bound_seconds: observation.bound_seconds,
        operation: observation.operation,
    };
    if newly_observed
        && (drafts::persist(&session_dir(context, &record.id).join(EVENT), &event).is_err()
            || drafts::persist(&path, &observation).is_err())
    {
        return;
    }
    state.phase = activity::TurnPhase::Unresponsive;
    state.phase_changed_at = observed_at;
    state.source.kind = activity::SourceKind::TerminalHeuristic;
    state.source.confidence = activity::Confidence::Observed;
}

/// Share the saved-draft board event attachment boundary and private writer.
pub(crate) fn project_events(context: &CliContext, id: &str, projected: &mut Value) {
    if let Ok(Some(event)) = drafts::latest(context, id) {
        projected["stale_draft"] = event;
    }
    if let Ok(event) = drafts::read_json::<Event>(&session_dir(context, id).join(EVENT), 2048)
        && event.schema_version == "agent-session.pane-response.v1"
        && event.event == "pane_unresponsive"
        && event.session_id == id
        && event.bound_seconds > 0
        && event.started_at.parse::<Timestamp>().is_ok()
        && event.observed_at.parse::<Timestamp>().is_ok()
        && let Ok(value) = serde_json::to_value(event)
    {
        projected["pane_response"] = value;
    }
}

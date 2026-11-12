//! Structured Claude session state from `claude agents --json`.
//!
//! The installed Claude CLI lists its active sessions with a `state` of
//! `working`, `blocked`, `done`, `failed` or `stopped` when the provider
//! reports one, and otherwise with a `status` of `idle`, `busy` or `waiting`
//! (plus `waitingFor`). The shadow sampler prefers
//! this provider-owned signal over screen rules and falls through to the
//! screen only when it is unavailable, recording why. One call per provider
//! config directory serves every session within a short window, because each
//! config directory has its own session registry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::{ShadowObservationView, TurnPhase};
use crate::SessionRecord;
use crate::cli::AgentKind;

pub(super) const OBSERVER_VERSION: &str = "claude-agents.v1";
/// Key in the observation of a screen-sampled Claude session: why the
/// structured source was not used.
pub(super) const UNAVAILABLE_KEY: &str = "claude_agents_unavailable";

const FETCH_TIMEOUT: Duration = Duration::from_millis(1500);
const OUTPUT_LIMIT: usize = 1024 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(2);
/// The child receives only these inherited variables. API-key style
/// credentials and endpoint overrides are never inherited; the only provider
/// variable is the session's own `CLAUDE_CONFIG_DIR`.
const INHERITED_ENV: [&str; 3] = ["HOME", "PATH", "TMPDIR"];
/// The `waitingFor` value that names an approval (provider documentation;
/// also observed in the provider spike on Claude Code 2.1.317). Any other value is reported as `other`, never copied.
const PERMISSION_PROMPT: &str = "permission prompt";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WaitingFor {
    PermissionPrompt,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AgentState {
    Idle,
    Busy,
    Waiting(WaitingFor),
}

/// Why the structured source gave no state for a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unavailable {
    BinaryMissing,
    Timeout,
    Failed,
    Malformed,
    NoProviderSession,
    Unmatched,
    UnknownState,
    ConflictingEntries,
}

impl Unavailable {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::BinaryMissing => "binary_missing",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::Malformed => "malformed",
            Self::NoProviderSession => "no_provider_session",
            Self::Unmatched => "unmatched",
            Self::UnknownState => "unknown_state",
            Self::ConflictingEntries => "conflicting_entries",
        }
    }
}

#[derive(Debug)]
pub(super) struct Snapshot(HashMap<String, Result<AgentState, Unavailable>>);

impl Snapshot {
    pub(super) fn state(&self, provider_session_id: &str) -> Result<AgentState, Unavailable> {
        self.0
            .get(provider_session_id)
            .copied()
            .unwrap_or(Err(Unavailable::Unmatched))
    }
}

/// The state of one listed session. `state` (background sessions and newer
/// Claude Code versions) decides when present: `status` then only says
/// whether the process is busy. `status` decides only when `state` is absent.
fn entry_state(entry: &serde_json::Map<String, Value>) -> Result<AgentState, Unavailable> {
    let waiting = || {
        AgentState::Waiting(match entry.get("waitingFor").and_then(Value::as_str) {
            Some(PERMISSION_PROMPT) => WaitingFor::PermissionPrompt,
            _ => WaitingFor::Other,
        })
    };
    match entry.get("state") {
        Some(state) => match state.as_str() {
            Some("working") => Ok(AgentState::Busy),
            Some("blocked") => Ok(waiting()),
            Some("done" | "failed" | "stopped") => Ok(AgentState::Idle),
            _ => Err(Unavailable::UnknownState),
        },
        None => match entry.get("status").and_then(Value::as_str) {
            Some("idle") => Ok(AgentState::Idle),
            Some("busy") => Ok(AgentState::Busy),
            Some("waiting") => Ok(waiting()),
            _ => Err(Unavailable::UnknownState),
        },
    }
}

pub(super) fn parse(output: &[u8]) -> Result<Snapshot, Unavailable> {
    let value: Value = serde_json::from_slice(output).map_err(|_| Unavailable::Malformed)?;
    let entries = value.as_array().ok_or(Unavailable::Malformed)?;
    let mut sessions = HashMap::new();
    for entry in entries {
        let entry = entry.as_object().ok_or(Unavailable::Malformed)?;
        let Some(session_id) = entry.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let state = entry_state(entry);
        // Two entries for one id may only agree; the list order decides nothing.
        let state = match sessions.get(session_id) {
            Some(previous) if *previous != state => Err(Unavailable::ConflictingEntries),
            _ => state,
        };
        sessions.insert(session_id.to_string(), state);
    }
    Ok(Snapshot(sessions))
}

pub(super) fn fetch(
    bin: &Path,
    config_dir: Option<&Path>,
    timeout: Duration,
) -> Result<Snapshot, Unavailable> {
    let mut command = ProcessCommand::new(bin);
    command.args(["agents", "--json"]).env_clear();
    for name in INHERITED_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    if let Some(config_dir) = config_dir {
        command.env("CLAUDE_CONFIG_DIR", config_dir);
    }
    let output = crate::run_output_with_timeout_and_strict_cap(command, timeout, OUTPUT_LIMIT)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => Unavailable::BinaryMissing,
            std::io::ErrorKind::TimedOut => Unavailable::Timeout,
            _ => Unavailable::Failed,
        })?;
    if !output.status.success() {
        return Err(Unavailable::Failed);
    }
    parse(&output.stdout)
}

type CacheKey = (PathBuf, Option<PathBuf>);
type Fetched = Result<Arc<Snapshot>, Unavailable>;

struct CacheEntry {
    fetched_at: Instant,
    result: Fetched,
}

type CacheSlot = Arc<Mutex<Option<CacheEntry>>>;

static CACHE: Mutex<Option<HashMap<CacheKey, CacheSlot>>> = Mutex::new(None);

/// One call per provider config directory within [`CACHE_TTL`]; concurrent
/// callers of the same key wait for the call in flight instead of spawning.
fn snapshot(bin: PathBuf, config_dir: Option<PathBuf>) -> Fetched {
    let slot = {
        let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        cache
            .get_or_insert_with(HashMap::new)
            .entry((bin.clone(), config_dir.clone()))
            .or_default()
            .clone()
    };
    let mut slot = slot.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(entry) = slot
        .as_ref()
        .filter(|entry| entry.fetched_at.elapsed() < CACHE_TTL)
    {
        return entry.result.clone();
    }
    let result = fetch(&bin, config_dir.as_deref(), FETCH_TIMEOUT).map(Arc::new);
    *slot = Some(CacheEntry {
        fetched_at: Instant::now(),
        result: result.clone(),
    });
    result
}

/// Structured state of one Claude session, matched by provider session id.
pub(super) fn state_for(record: &SessionRecord) -> Result<AgentState, Unavailable> {
    let provider_session_id = record
        .provider_resume
        .as_ref()
        .filter(|resume| resume.provider == "claude")
        .map(|resume| resume.session_id.as_str())
        .ok_or(Unavailable::NoProviderSession)?;
    // A session without its own config directory runs in the ambient one.
    let config_dir = crate::session_effective_provider_config_dir(record)
        .or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from));
    let bin = crate::agent_bin::resolve_agent_bin(AgentKind::Claude, None);
    snapshot(bin, config_dir)?.state(provider_session_id)
}

pub(super) fn observation(state: AgentState, phase: &TurnPhase) -> ShadowObservationView {
    let (projection, rule_id) = match state {
        AgentState::Busy => ("working", "claude_agents_busy"),
        AgentState::Idle => ("waiting", "claude_agents_idle"),
        AgentState::Waiting(WaitingFor::PermissionPrompt) => {
            ("needs_input", "claude_agents_waiting_permission_prompt")
        }
        AgentState::Waiting(WaitingFor::Other) => ("needs_input", "claude_agents_waiting_other"),
    };
    ShadowObservationView {
        observer_version: OBSERVER_VERSION.to_string(),
        rule_id: rule_id.to_string(),
        observed_at: super::super::now(),
        projection: projection.to_string(),
        disagrees: super::disagrees(phase, projection),
        extra: serde_json::Map::new(),
    }
}

#[cfg(test)]
mod tests;

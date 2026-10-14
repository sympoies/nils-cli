//! Opt-in native-send diagnostics contain typed metadata, never RPC values.
use std::cell::RefCell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Instant;

use serde::Serialize;

use crate::{SessionRecord, SpecialKey, cli, codex_app_server};

const MAX_EVENTS: usize = 256;
const CONTROL: &str = "AGENT_SESSION_GUARDED_TRACE_FILE";
thread_local! { static ACTIVE: RefCell<Option<State>> = const { RefCell::new(None) }; }

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Method {
    Connect,
    Initialize,
    LoadedThreads,
    ThreadRead,
    TurnStart,
    CompactStart,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Outcome {
    Ok,
    Timeout,
    Unavailable,
    Rejected,
    DeadlineExpired,
    Cancelled,
    Observed,
    NotObserved,
    Invalid,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    AdmissionStarted,
    RecordLockWaiting,
    RecordLockAcquired,
    RecordLockFailed,
    BudgetStarted,
    RpcRequest,
    RpcResponse,
    InputObservation,
    TurnAcknowledged,
    TraceTruncated,
}

// No field accepts a caller-controlled string or a provider JSON value.
#[derive(Serialize)]
struct Event {
    schema_version: &'static str,
    stage: Stage,
    elapsed_us: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<Method>,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<Outcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remaining_us: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    include_turns: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_matches: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_matches: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text_matches: Option<bool>,
}

impl Event {
    fn new(stage: Stage) -> Self {
        Self {
            schema_version: "agent-session.guarded-trace.v1",
            stage,
            elapsed_us: 0,
            request: None,
            method: None,
            outcome: None,
            remaining_us: None,
            include_turns: None,
            thread_matches: None,
            turn_matches: None,
            text_matches: None,
        }
    }
}

struct State {
    file: File,
    started: Instant,
    events: Vec<Event>,
    next_request: u32,
}

fn record(mut event: Event) {
    ACTIVE.with(|active| {
        if let Some(state) = active.borrow_mut().as_mut() {
            event.elapsed_us = micros(state.started.elapsed());
            if state.events.len() < MAX_EVENTS - 1 {
                state.events.push(event);
            } else if state.events.len() == MAX_EVENTS - 1 {
                let mut truncated = Event::new(Stage::TraceTruncated);
                truncated.elapsed_us = event.elapsed_us;
                state.events.push(truncated);
            }
        }
    });
}

fn micros(duration: std::time::Duration) -> u64 {
    duration.as_micros().min(u64::MAX as u128) as u64
}

/// Lifetime is outside the record/account locks. No diagnostic I/O while held.
pub(crate) struct Scope {
    previous: Option<State>,
}

impl Scope {
    pub(crate) fn for_send(
        record: &SessionRecord,
        args: &cli::SendArgs,
        text: Option<&str>,
    ) -> Self {
        let state = if codex_app_server::runtime_is_supported(record)
            && args.guards.require_idle
            && args.guards.require_empty_composer
            && crate::carries_literal_text(text)
            && args.keys == [SpecialKey::Enter]
        {
            std::env::var_os(CONTROL)
                .and_then(|path| open_private(Path::new(&path)))
                .map(|file| State {
                    file,
                    started: Instant::now(),
                    events: Vec::new(),
                    next_request: 0,
                })
        } else {
            None
        };
        let previous = ACTIVE.with(|active| active.replace(state));
        record_event(Stage::AdmissionStarted);
        Self { previous }
    }
}

fn open_private(path: &Path) -> Option<File> {
    // Reject relative paths, symlink ancestors, public/foreign parent dirs and
    // pre-existing targets. Never replace an operator's existing evidence.
    let parent = path.parent()?;
    if !path.is_absolute() || parent.canonicalize().ok()? != parent {
        return None;
    }
    let metadata = fs::symlink_metadata(parent).ok()?;
    if !metadata.is_dir()
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return None;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()
}

impl Drop for Scope {
    fn drop(&mut self) {
        let state = ACTIVE.with(|active| active.replace(self.previous.take()));
        if let Some(mut state) = state {
            // Typed bounded events serialize only after all send guards finish.
            let mut bytes = Vec::new();
            for event in state.events {
                if serde_json::to_writer(&mut bytes, &event).is_err() {
                    return;
                }
                bytes.push(b'\n');
            }
            let _ = state.file.write_all(&bytes);
        }
    }
}

fn record_event(stage: Stage) {
    record(Event::new(stage));
}

pub(crate) fn record_lock<T, E>(acquire: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    record_event(Stage::RecordLockWaiting);
    let result = acquire();
    record_event(if result.is_ok() {
        Stage::RecordLockAcquired
    } else {
        Stage::RecordLockFailed
    });
    result
}

pub(crate) fn budget_start() -> Instant {
    let started = Instant::now();
    record_event(Stage::BudgetStarted);
    started
}

pub(super) struct Request {
    number: Option<u32>,
    method: Method,
    include_turns: Option<bool>,
    finished: bool,
}
impl Request {
    pub(super) fn start(method: Method, deadline: Instant, include_turns: Option<bool>) -> Self {
        let number = ACTIVE.with(|active| {
            active.borrow_mut().as_mut().map(|state| {
                state.next_request += 1;
                state.next_request
            })
        });
        let mut event = Event::new(Stage::RpcRequest);
        event.request = number;
        event.method = Some(method);
        event.include_turns = include_turns;
        event.remaining_us = Some(micros(deadline.saturating_duration_since(Instant::now())));
        record(event);
        Self {
            number,
            method,
            include_turns,
            finished: false,
        }
    }
    pub(super) fn finish(mut self, outcome: Outcome) {
        self.finished = true;
        self.emit(outcome);
    }
    fn emit(&self, outcome: Outcome) {
        let mut event = Event::new(Stage::RpcResponse);
        event.request = self.number;
        event.method = Some(self.method);
        event.include_turns = self.include_turns;
        event.outcome = Some(outcome);
        record(event);
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        if !self.finished && self.number.is_some() {
            self.emit(Outcome::Cancelled);
        }
    }
}

pub(super) fn turn_ack(valid: bool) {
    let mut event = Event::new(Stage::TurnAcknowledged);
    event.outcome = Some(if valid { Outcome::Ok } else { Outcome::Invalid });
    record(event);
}

pub(super) fn rpc_error(error: &str) -> Outcome {
    if error == "Codex app-server request timed out" {
        Outcome::Timeout
    } else if error.starts_with("Codex app-server rejected request:") {
        Outcome::Rejected
    } else {
        Outcome::Unavailable
    }
}

pub(super) struct Observation {
    pub(super) result: Result<bool, ()>,
    pub(super) thread_matches: Option<bool>,
    pub(super) turn_matches: Option<bool>,
    pub(super) text_matches: Option<bool>,
}
impl Observation {
    pub(super) fn invalid() -> Self {
        Self {
            result: Err(()),
            thread_matches: None,
            turn_matches: None,
            text_matches: None,
        }
    }
}

pub(super) fn observation(observed: &Observation) {
    let mut event = Event::new(Stage::InputObservation);
    event.outcome = Some(match observed.result {
        Ok(true) => Outcome::Observed,
        Ok(false) => Outcome::NotObserved,
        Err(()) => Outcome::Invalid,
    });
    event.thread_matches = observed.thread_matches;
    event.turn_matches = observed.turn_matches;
    event.text_matches = observed.text_matches;
    record(event);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};

    fn scope(path: &Path) -> Scope {
        let state = State {
            file: open_private(path).unwrap(),
            started: Instant::now(),
            events: Vec::new(),
            next_request: 0,
        };
        Scope {
            previous: ACTIVE.with(|active| active.replace(Some(state))),
        }
    }

    #[test]
    fn bounded_trace_has_one_truncation_record_and_no_untyped_fields() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let path = dir.path().canonicalize().unwrap().join("trace");
        {
            let _scope = scope(&path);
            for _ in 0..(MAX_EVENTS * 2) {
                record_event(Stage::AdmissionStarted);
            }
        }
        let rows: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), MAX_EVENTS);
        assert_eq!(rows.last().unwrap()["stage"], "trace_truncated");
        assert!(rows.iter().all(|r| r.as_object().unwrap().len() == 3));
    }

    #[test]
    fn payload_and_error_markers_never_reach_trace_bytes() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let path = dir.path().canonicalize().unwrap().join("trace");
        let payload = json!({"thread":{"id":"private-thread", "turns":[{
            "id":"private-turn", "items":[{"type":"userMessage", "content":[
                {"type":"text", "text":"private-prompt", "token":"private-token"}]}]
        }]}, "secret":"private-value"});
        {
            let _scope = scope(&path);
            observation(&super::super::input_observed(
                &payload,
                "private-thread",
                "private-turn",
                "private-prompt",
            ));
            let request = Request::start(Method::TurnStart, Instant::now(), None);
            request.finish(rpc_error(
                "Codex app-server rejected request: private-error",
            ));
        }
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("private-"));
        assert!(!bytes.contains("token"));
        let row: Value = serde_json::from_str(bytes.lines().next().unwrap()).unwrap();
        assert_eq!(row["thread_matches"], true);
        assert_eq!(row["turn_matches"], true);
        assert_eq!(row["text_matches"], true);
    }
    #[test]
    fn unfinished_request_has_one_correlated_cancelled_response() {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(
            dir.path(),
            std::os::unix::fs::PermissionsExt::from_mode(0o700),
        )
        .unwrap();
        let path = dir.path().canonicalize().unwrap().join("trace");
        {
            let _scope = scope(&path);
            let _request = Request::start(Method::Initialize, Instant::now(), None);
        }
        let rows: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["stage"], "rpc_request");
        assert_eq!(rows[1]["stage"], "rpc_response");
        assert_eq!(rows[1]["request"], rows[0]["request"]);
        assert_eq!(rows[1]["method"], "initialize");
        assert_eq!(rows[1]["outcome"], "cancelled");
    }
}

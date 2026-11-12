use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::thread;
use std::time::{Duration, Instant};

use jiff::Timestamp;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;

use super::super::{SHADOW_FILE, ShadowDocument, annotate_for_view, read_current, session_dir};
use super::*;
use crate::{CliContext, activity};

const MIXED: &str = include_str!("../../../../tests/fixtures/claude-agents/mixed.json");
const WITH_STATE: &str = include_str!("../../../../tests/fixtures/claude-agents/with-state.json");
const MALFORMED: &str = include_str!("../../../../tests/fixtures/claude-agents/malformed.txt");
const IDLE_ID: &str = "00000000-0000-4000-8000-000000000001";
const BUSY_ID: &str = "00000000-0000-4000-8000-000000000002";
const WAITING_ID: &str = "00000000-0000-4000-8000-000000000003";
const WAITING_OTHER_ID: &str = "00000000-0000-4000-8000-000000000004";
const PAUSED_ID: &str = "00000000-0000-4000-8000-000000000005";
const STATE_WORKING_IDLE_PROCESS_ID: &str = "00000000-0000-4000-8000-000000000011";
const STATE_BLOCKED_PROMPT_ID: &str = "00000000-0000-4000-8000-000000000012";
const STATE_BLOCKED_NO_PROCESS_ID: &str = "00000000-0000-4000-8000-000000000013";
const STATE_DONE_BUSY_PROCESS_ID: &str = "00000000-0000-4000-8000-000000000014";
const STATE_FAILED_ID: &str = "00000000-0000-4000-8000-000000000015";
const STATE_STOPPED_ID: &str = "00000000-0000-4000-8000-000000000016";
const STATE_UNKNOWN_ID: &str = "00000000-0000-4000-8000-000000000017";

/// Writes an executable stub. A sibling test's fork can hold the write
/// descriptor until it execs, which makes the first exec fail with ETXTBSY and
/// the adapter report `Failed`. Running the stub once through the shared retry
/// (it exits at once when probed) outlasts every such fork, because a fork made
/// after this write no longer inherits the descriptor.
fn write_script(path: &Path, body: &str) {
    fs::write(
        path,
        format!("#!/bin/sh\n[ -n \"$STUB_PROBE\" ] && exit 0\n{body}\n"),
    )
    .unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    nils_test_support::cmd::retry_executable_file_busy(|| {
        ProcessCommand::new(path).env("STUB_PROBE", "1").status()
    })
    .unwrap();
}

/// A stub `claude` that counts calls and prints the mixed fixture.
fn counting_claude(dir: &Path) -> (PathBuf, PathBuf) {
    counting_claude_printing(dir, MIXED)
}

/// A stub `claude` that counts calls and prints the given listing.
fn counting_claude_printing(dir: &Path, listing: &str) -> (PathBuf, PathBuf) {
    let fixture = dir.join("agents.json");
    fs::write(&fixture, listing).unwrap();
    let calls = dir.join("calls");
    let bin = dir.join("claude");
    write_script(
        &bin,
        &format!(
            "printf '%s\\n' \"$*\" >> '{}'\ncat '{}'",
            calls.display(),
            fixture.display()
        ),
    );
    (bin, calls)
}

fn call_count(calls: &Path) -> usize {
    fs::read_to_string(calls).map_or(0, |calls| calls.lines().count())
}

fn claude_record(id: &str, provider_session_id: &str) -> SessionRecord {
    serde_json::from_value(json!({
        "schema_version": "agent-session.session.v1",
        "id": id,
        "agent": "claude",
        "mode": "interactive",
        "title": null,
        "cwd": "/tmp",
        "tmux_session": format!("hs-claude-{id}"),
        "prompt_file": null,
        "log_file": null,
        "created_at": "2026-07-10T00:00:00Z",
        "updated_at": "2026-07-10T00:00:00Z",
        "provider_resume": {
            "provider": "claude",
            "session_id": provider_session_id,
            "captured_at": "2026-07-10T00:00:00Z",
            "capture_method": "test",
            "resume_args": []
        },
        "runtime": {
            "kind": "tmux",
            "tmux_session": format!("hs-claude-{id}"),
            "generation": 1,
            "started_at": "2026-07-10T00:00:00Z",
            "launch_id": "launch-a"
        }
    }))
    .unwrap()
}

#[test]
fn snapshot_maps_idle_busy_and_waiting_by_provider_session_id() {
    let snapshot = parse(MIXED.as_bytes()).unwrap();
    assert_eq!(snapshot.state(IDLE_ID), Ok(AgentState::Idle));
    assert_eq!(snapshot.state(BUSY_ID), Ok(AgentState::Busy));
    assert_eq!(
        snapshot.state(WAITING_ID),
        Ok(AgentState::Waiting(WaitingFor::PermissionPrompt))
    );
}

#[test]
fn unrecognized_waiting_reason_is_not_copied_from_the_provider() {
    let snapshot = parse(MIXED.as_bytes()).unwrap();
    assert_eq!(
        snapshot.state(WAITING_OTHER_ID),
        Ok(AgentState::Waiting(WaitingFor::Other))
    );
}

#[test]
fn unlisted_session_is_unmatched() {
    let snapshot = parse(MIXED.as_bytes()).unwrap();
    assert_eq!(
        snapshot.state("00000000-0000-4000-8000-0000000000ff"),
        Err(Unavailable::Unmatched)
    );
}

fn entry(session_id: &str, status: &str) -> serde_json::Value {
    json!({"sessionId": session_id, "status": status, "pid": 1})
}

#[test]
fn conflicting_duplicate_entries_are_unavailable_in_either_order() {
    for entries in [
        [entry(BUSY_ID, "busy"), entry(BUSY_ID, "idle")],
        [entry(BUSY_ID, "idle"), entry(BUSY_ID, "busy")],
    ] {
        let snapshot = parse(json!(entries).to_string().as_bytes()).unwrap();
        assert_eq!(
            snapshot.state(BUSY_ID),
            Err(Unavailable::ConflictingEntries),
            "{entries:?}"
        );
    }
}

#[test]
fn a_conflict_among_three_duplicate_entries_is_not_resolved_by_a_later_agreeing_entry() {
    for entries in [
        [
            entry(BUSY_ID, "busy"),
            entry(BUSY_ID, "idle"),
            entry(BUSY_ID, "busy"),
        ],
        [
            entry(BUSY_ID, "idle"),
            entry(BUSY_ID, "busy"),
            entry(BUSY_ID, "idle"),
        ],
    ] {
        let snapshot = parse(json!(entries).to_string().as_bytes()).unwrap();
        assert_eq!(
            snapshot.state(BUSY_ID),
            Err(Unavailable::ConflictingEntries),
            "{entries:?}"
        );
    }
}

#[test]
fn identical_duplicate_entries_still_resolve() {
    let snapshot = parse(
        json!([
            entry(BUSY_ID, "busy"),
            entry(BUSY_ID, "busy"),
            entry(IDLE_ID, "idle")
        ])
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(snapshot.state(BUSY_ID), Ok(AgentState::Busy));
    assert_eq!(snapshot.state(IDLE_ID), Ok(AgentState::Idle));
}

#[test]
fn unrecognized_status_is_unavailable_for_that_session_only() {
    let snapshot = parse(MIXED.as_bytes()).unwrap();
    assert_eq!(snapshot.state(PAUSED_ID), Err(Unavailable::UnknownState));
    assert_eq!(snapshot.state(BUSY_ID), Ok(AgentState::Busy));
}

#[test]
fn malformed_output_is_unavailable() {
    for output in [MALFORMED, "", "{\"sessionId\": \"x\"}", "[1, 2]"] {
        assert_eq!(
            parse(output.as_bytes()).err(),
            Some(Unavailable::Malformed),
            "{output:?}"
        );
    }
}

#[test]
fn unavailability_codes_are_stable() {
    let codes = [
        (Unavailable::BinaryMissing, "binary_missing"),
        (Unavailable::Timeout, "timeout"),
        (Unavailable::Failed, "failed"),
        (Unavailable::Malformed, "malformed"),
        (Unavailable::NoProviderSession, "no_provider_session"),
        (Unavailable::Unmatched, "unmatched"),
        (Unavailable::UnknownState, "unknown_state"),
        (Unavailable::ConflictingEntries, "conflicting_entries"),
    ];
    for (reason, code) in codes {
        assert_eq!(reason.code(), code);
    }
}

#[test]
fn observation_records_the_source_and_projects_each_state() {
    for (state, rule_id, projection, phase, disagrees) in [
        (
            AgentState::Busy,
            "claude_agents_busy",
            "working",
            TurnPhase::Working,
            false,
        ),
        (
            AgentState::Idle,
            "claude_agents_idle",
            "waiting",
            TurnPhase::Working,
            true,
        ),
        (
            AgentState::Waiting(WaitingFor::PermissionPrompt),
            "claude_agents_waiting_permission_prompt",
            "needs_input",
            TurnPhase::NeedsInput,
            false,
        ),
        (
            AgentState::Waiting(WaitingFor::Other),
            "claude_agents_waiting_other",
            "needs_input",
            TurnPhase::Working,
            true,
        ),
    ] {
        let observation = observation(state, &phase);
        assert_eq!(observation.observer_version, OBSERVER_VERSION);
        assert_eq!(observation.rule_id, rule_id);
        assert_eq!(observation.projection, projection);
        assert_eq!(observation.disagrees, disagrees);
    }
}

#[test]
fn fetch_builds_an_explicit_child_environment() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let dump = tmp.path().join("env");
    let bin = tmp.path().join("claude");
    write_script(
        &bin,
        &format!(
            "printf 'ARGS=%s\\n' \"$*\" > '{0}'\nenv >> '{0}'\nprintf '[]'",
            dump.display()
        ),
    );
    let config = tmp.path().join("config");
    let _guards = [
        EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "synthetic-key-canary"),
        EnvGuard::set(&lock, "ANTHROPIC_AUTH_TOKEN", "synthetic-token-canary"),
        EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", "https://example.invalid"),
        EnvGuard::set(&lock, "CLAUDE_CODE_OAUTH_TOKEN", "synthetic-oauth-canary"),
    ];

    fetch(&bin, Some(&config), Duration::from_secs(5)).unwrap();

    let dump = fs::read_to_string(&dump).unwrap();
    assert!(dump.starts_with("ARGS=agents --json\n"), "{dump}");
    assert!(
        dump.contains(&format!("CLAUDE_CONFIG_DIR={}", config.display())),
        "{dump}"
    );
    for forbidden in ["ANTHROPIC_", "canary", "CLAUDE_CODE_OAUTH_TOKEN"] {
        assert!(!dump.contains(forbidden), "{forbidden} leaked: {dump}");
    }
    assert!(dump.contains("PATH="), "{dump}");
}

#[test]
fn fetch_reports_missing_binary_timeout_and_failure() {
    let tmp = tempfile::TempDir::new().unwrap();
    assert_eq!(
        fetch(&tmp.path().join("absent"), None, Duration::from_secs(1)).err(),
        Some(Unavailable::BinaryMissing)
    );
    let slow = tmp.path().join("slow");
    write_script(&slow, "sleep 5");
    assert_eq!(
        fetch(&slow, None, Duration::from_millis(200)).err(),
        Some(Unavailable::Timeout)
    );
    let failing = tmp.path().join("failing");
    write_script(&failing, "exit 3");
    assert_eq!(
        fetch(&failing, None, Duration::from_secs(5)).err(),
        Some(Unavailable::Failed)
    );
}

#[test]
fn one_call_serves_every_session_sharing_a_config_dir() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, calls) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");

    assert_eq!(
        state_for(&claude_record("one", IDLE_ID)),
        Ok(AgentState::Idle)
    );
    assert_eq!(
        state_for(&claude_record("two", BUSY_ID)),
        Ok(AgentState::Busy)
    );
    assert_eq!(
        state_for(&claude_record("three", WAITING_ID)),
        Ok(AgentState::Waiting(WaitingFor::PermissionPrompt))
    );
    assert_eq!(call_count(&calls), 1, "one spawn per poll, not per session");

    // A session on another provider config dir lives in another registry.
    let mut other = claude_record("four", BUSY_ID);
    other.runtime.as_mut().unwrap().extra.insert(
        crate::AGENT_PROFILE_PROVIDER_CONFIG_DIR_RUNTIME_KEY.to_string(),
        json!(tmp.path().join("other-config")),
    );
    assert_eq!(state_for(&other), Ok(AgentState::Busy));
    assert_eq!(call_count(&calls), 2);
}

#[test]
fn session_without_a_provider_session_id_is_unavailable() {
    let mut record = claude_record("missing", IDLE_ID);
    record.provider_resume = None;
    assert_eq!(state_for(&record), Err(Unavailable::NoProviderSession));
}

struct Sampled {
    context: CliContext,
    record: SessionRecord,
    tmux: PathBuf,
    captures: PathBuf,
    _tmp: tempfile::TempDir,
}

/// A running Claude turn (open turn, no attention) observed through a fake tmux.
fn running_turn(provider_session_id: &str) -> Sampled {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = claude_record("sampled", provider_session_id);
    let dir = session_dir(&context, &record.id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&record).unwrap(),
    )
    .unwrap();
    activity::activate_runtime(&context, &record).unwrap();
    let event: activity::TurnEvent = serde_json::from_value(json!({
        "schema_version": "agent-session.turn-event.v1", "event_id": "start",
        "runtime_id": "launch-a", "provider": "claude", "provider_turn_id": "turn-1",
        "provider_session_id": provider_session_id,
        "kind": "progress", "confidence": "observed", "source_kind": "provider_hook"
    }))
    .unwrap();
    activity::ingest_event(&context, &record.id, event).unwrap();
    let captures = tmp.path().join("captures");
    let tmux = tmp.path().join("fake-tmux");
    write_script(
        &tmux,
        &format!(
            "case \"$1\" in\n display-message) printf 'Claude\\n' ;;\n capture-pane) printf 'capture\\n' >> '{}'; printf '❯ \\n' ;;\nesac",
            captures.display()
        ),
    );
    Sampled {
        context,
        record,
        tmux,
        captures,
        _tmp: tmp,
    }
}

impl Sampled {
    fn collect(&self, schedule: bool) -> activity::TurnState {
        annotate_for_view(
            &self.context,
            &self.record,
            "running",
            &self.tmux,
            activity::state_for_view(&self.context, &self.record).unwrap(),
            schedule,
        )
    }

    fn path(&self) -> PathBuf {
        session_dir(&self.context, &self.record.id).join(SHADOW_FILE)
    }

    fn wait_for_document(&self, ready: impl Fn(&ShadowDocument) -> bool) -> ShadowDocument {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(document) =
                read_current(&self.path(), &self.record).filter(|document| ready(document))
            {
                return document;
            }
            assert!(Instant::now() < deadline, "shadow sample timed out");
            thread::sleep(Duration::from_millis(50));
            // The sampler has a process-wide permit cap shared with concurrent
            // tests; a request that found no permit is simply asked again.
            self.collect(true);
        }
    }
}

#[test]
fn sampler_prefers_the_structured_state_over_the_screen() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(BUSY_ID);

    sampled.collect(true);
    let document = sampled.wait_for_document(|_| true);

    assert_eq!(document.observation.observer_version, OBSERVER_VERSION);
    assert_eq!(document.observation.rule_id, "claude_agents_busy");
    assert_eq!(document.observation.projection, "working");
    assert!(!document.observation.disagrees);
    assert!(document.observation.extra.get(UNAVAILABLE_KEY).is_none());
    assert!(!sampled.captures.exists(), "the screen was sampled anyway");
    let state = sampled.collect(false);
    assert_eq!(
        state.shadow_observation.map(|view| view.rule_id),
        Some("claude_agents_busy".to_string())
    );
}

#[test]
fn unavailable_structured_state_falls_through_to_the_screen_and_says_so() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude(tmp.path());
    for (pinned, session, reason) in [
        (tmp.path().join("absent"), BUSY_ID, "binary_missing"),
        (bin, "00000000-0000-4000-8000-0000000000ff", "unmatched"),
    ] {
        let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", pinned.to_str().unwrap());
        let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
        let sampled = running_turn(session);

        sampled.collect(true);
        let document = sampled.wait_for_document(|_| true);

        assert_eq!(document.observation.rule_id, "claude_prompt_visible");
        assert_eq!(
            document.observation.extra.get(UNAVAILABLE_KEY),
            Some(&json!(reason))
        );
        assert!(sampled.captures.exists());
    }
}

#[test]
fn idle_provider_state_over_an_open_turn_is_sustained_uncertainty() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(IDLE_ID);

    sampled.collect(true);
    let mut first = sampled.wait_for_document(|_| true);
    assert_eq!(first.observation.rule_id, "claude_agents_idle");
    assert!(first.observation.disagrees);
    assert_eq!(sampled.collect(false).phase, activity::TurnPhase::Working);

    // The same idle reading twenty seconds later, on the same revision.
    let earlier = Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(20))
        .unwrap()
        .to_string();
    first.observation.observed_at = earlier.clone();
    first.interrupt_since = Some(earlier.clone());
    fs::write(sampled.path(), serde_json::to_vec(&first).unwrap()).unwrap();
    sampled.collect(true);
    let second = sampled.wait_for_document(|document| document.observation.observed_at != earlier);
    assert_eq!(second.interrupt_since.as_deref(), Some(earlier.as_str()));

    let state = sampled.collect(false);
    assert_eq!(state.phase, activity::TurnPhase::Unknown);
    assert_eq!(
        state.diagnostic.map(|diagnostic| diagnostic.reason),
        Some("interrupted_suspected".to_string())
    );
}

#[test]
fn state_field_decides_and_status_only_says_whether_the_process_is_busy() {
    let snapshot = parse(WITH_STATE.as_bytes()).unwrap();
    // `state: working` outranks `status: idle`.
    assert_eq!(
        snapshot.state(STATE_WORKING_IDLE_PROCESS_ID),
        Ok(AgentState::Busy)
    );
    assert_eq!(
        snapshot.state(STATE_BLOCKED_PROMPT_ID),
        Ok(AgentState::Waiting(WaitingFor::PermissionPrompt))
    );
    // Blocked without a live process names no prompt.
    assert_eq!(
        snapshot.state(STATE_BLOCKED_NO_PROCESS_ID),
        Ok(AgentState::Waiting(WaitingFor::Other))
    );
    // `state: done` outranks `status: busy`; failed and stopped have no status.
    for id in [
        STATE_DONE_BUSY_PROCESS_ID,
        STATE_FAILED_ID,
        STATE_STOPPED_ID,
    ] {
        assert_eq!(snapshot.state(id), Ok(AgentState::Idle), "{id}");
    }
}

#[test]
fn unknown_state_is_unavailable_and_does_not_fall_back_to_status() {
    let snapshot = parse(WITH_STATE.as_bytes()).unwrap();
    assert_eq!(
        snapshot.state(STATE_UNKNOWN_ID),
        Err(Unavailable::UnknownState)
    );
    assert_eq!(
        snapshot.state(STATE_FAILED_ID),
        Ok(AgentState::Idle),
        "one unknown entry must not affect another"
    );
}

#[test]
fn a_state_that_is_not_a_string_is_unknown() {
    let snapshot = parse(
        json!([{"sessionId": BUSY_ID, "status": "busy", "state": null}])
            .to_string()
            .as_bytes(),
    )
    .unwrap();
    assert_eq!(snapshot.state(BUSY_ID), Err(Unavailable::UnknownState));
}

#[test]
fn sampler_reads_state_over_status_end_to_end() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude_printing(tmp.path(), WITH_STATE);
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(STATE_WORKING_IDLE_PROCESS_ID);

    sampled.collect(true);
    let document = sampled.wait_for_document(|_| true);

    assert_eq!(document.observation.observer_version, OBSERVER_VERSION);
    assert_eq!(document.observation.rule_id, "claude_agents_busy");
    assert_eq!(document.observation.projection, "working");
    assert!(!sampled.captures.exists(), "the screen was sampled anyway");
}

#[test]
fn unknown_state_falls_through_to_the_screen_and_says_so() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, _) = counting_claude_printing(tmp.path(), WITH_STATE);
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(STATE_UNKNOWN_ID);

    sampled.collect(true);
    let document = sampled.wait_for_document(|_| true);

    assert_eq!(document.observation.rule_id, "claude_prompt_visible");
    assert_eq!(
        document.observation.extra.get(UNAVAILABLE_KEY),
        Some(&json!("unknown_state"))
    );
}

#[test]
fn the_cache_serves_a_snapshot_only_until_it_expires() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, calls) = counting_claude(tmp.path());

    snapshot(bin.clone(), None).unwrap();
    snapshot(bin.clone(), None).unwrap();
    assert_eq!(call_count(&calls), 1, "a fresh snapshot is reused");

    thread::sleep(Duration::from_millis(2300));
    snapshot(bin, None).unwrap();
    assert_eq!(
        call_count(&calls),
        2,
        "an expired snapshot is fetched again"
    );
}

#[test]
fn the_cache_is_keyed_by_binary_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let first = tmp.path().join("first");
    let second = tmp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    let (first_bin, first_calls) = counting_claude(&first);
    let (second_bin, second_calls) = counting_claude(&second);

    snapshot(first_bin, None).unwrap();
    snapshot(second_bin, None).unwrap();

    assert_eq!(call_count(&first_calls), 1);
    assert_eq!(
        call_count(&second_calls),
        1,
        "another binary must not be served from the first one's snapshot"
    );
}

#[test]
fn only_a_claude_provider_session_id_is_looked_up() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, calls) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let mut record = claude_record("other-provider", BUSY_ID);
    record.provider_resume.as_mut().unwrap().provider = "codex".to_string();

    assert_eq!(state_for(&record), Err(Unavailable::NoProviderSession));
    assert_eq!(call_count(&calls), 0, "no call for a non-Claude id");
}

#[test]
fn a_codex_session_never_consults_the_claude_listing() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let (bin, calls) = counting_claude(tmp.path());
    let _bin = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_BIN", bin.to_str().unwrap());
    let _config = EnvGuard::remove(&lock, "CLAUDE_CONFIG_DIR");
    let sampled = running_turn(BUSY_ID);
    // A Claude-shaped resume id that is listed as busy must still not be used.
    let mut record = sampled.record.clone();
    record.agent = "codex".to_string();

    let observation = super::super::sample(&record, &sampled.tmux, &TurnPhase::Working);

    assert_eq!(call_count(&calls), 0);
    assert_eq!(observation.observer_version, super::super::SHADOW_VERSION);
    assert!(observation.extra.get(UNAVAILABLE_KEY).is_none());
}

/// A stub `claude` printing a valid empty listing padded to `total` bytes.
fn padded_listing_claude(dir: &Path, total: usize) -> PathBuf {
    let bin = dir.join("padded");
    write_script(
        &bin,
        &format!(
            "printf '['\nhead -c {} /dev/zero | tr '\\0' ' '\nprintf ']'",
            total - 2
        ),
    );
    bin
}

#[test]
fn output_is_capped_at_one_mebibyte() {
    let tmp = tempfile::TempDir::new().unwrap();
    let limit = 1024 * 1024;
    assert!(
        snapshot(padded_listing_claude(tmp.path(), limit), None).is_ok(),
        "exactly the cap is accepted"
    );
    let over = tmp.path().join("over");
    fs::create_dir_all(&over).unwrap();
    assert_eq!(
        snapshot(padded_listing_claude(&over, limit + 1), None).err(),
        Some(Unavailable::Failed),
        "one byte over the cap is refused"
    );
}

#[test]
fn the_call_is_bounded_to_one_and_a_half_seconds() {
    let tmp = tempfile::TempDir::new().unwrap();
    let within = tmp.path().join("within");
    write_script(&within, "sleep 0.5\nprintf '[]'");
    assert!(
        snapshot(within, None).is_ok(),
        "a call within the bound completes"
    );
    let beyond = tmp.path().join("beyond");
    write_script(&beyond, "sleep 3\nprintf '[]'");
    assert_eq!(
        snapshot(beyond, None).err(),
        Some(Unavailable::Timeout),
        "a call beyond the bound is cut off"
    );
}

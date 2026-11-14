//! Exact-owner recovery uses disposable state and the test process as the runtime root.
#![cfg(target_os = "linux")]

use std::{fs, os::unix::fs::MetadataExt, path::Path};

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use super::coordination::{
    capability, digest, init_checkout, load_coordination_registry, rewrite_registry,
    seed_activity_state, seed_brokers_at, write_private_json,
};

const TOKEN: &str = "synthetic-owner-capability-material-0001";

struct Fixture {
    root: tempfile::TempDir,
    state: std::path::PathBuf,
    checkout: std::path::PathBuf,
    identity: Value,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        fs::create_dir(&state).unwrap();
        let checkout = root.path().join("checkout");
        init_checkout(&checkout, "https://example.invalid/example/repository.git");
        fs::write(checkout.join("user-edit.txt"), "unfinished user edit\n").unwrap();
        seed_brokers_at(
            &state,
            &[("owner", "original", TOKEN, &checkout, Some("enforce"))],
        );
        let pid = std::process::id();
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields: Vec<_> = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        let namespace = fs::metadata("/proc/self/ns/pid").unwrap();
        let identity = json!({
            "launch_id": "original", "session_id": "$1", "pane_id": "%1",
            "pane_pid": pid, "pane_start_time": fields[19].parse::<u64>().unwrap(),
            "process_group_id": fields[2].parse::<i32>().unwrap(),
            "process_session_id": fields[3].parse::<i32>().unwrap(),
            "pid_namespace": {
                "device": namespace.dev(), "inode": namespace.ino(),
                "boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()
            }
        });
        let fixture = Self {
            root,
            state,
            checkout,
            identity,
        };
        fixture.publish_runtime("original", 1);
        let record: agent_session::SessionRecord = serde_json::from_slice(
            &fs::read(fixture.state.join("sessions/owner/session.json")).unwrap(),
        )
        .unwrap();
        let runtime = agent_session::coordination_runtime_evidence(
            &agent_session::CliContext {
                state_dir: fixture.state.clone(),
                host: None,
            },
            &record,
        )
        .unwrap();
        rewrite_registry(&fixture.state, |registry| {
            registry["brokers"]["owner"]["runtime_identity"] = fixture.identity.clone();
            registry["brokers"]["owner"]["runtime_identity_digest"] =
                json!(runtime.identity_digest);
        });
        let claimed = fixture.set("original");
        assert_eq!(claimed.code, 0, "{}", claimed.stdout_text());
        fixture
    }

    fn publish_runtime(&self, incarnation: &str, generation: u64) {
        let path = self.state.join("sessions/owner/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["runtime"]["launch_id"] = json!(incarnation);
        record["runtime"]["generation"] = json!(generation);
        let mut identity = self.identity.clone();
        identity["launch_id"] = json!(incarnation);
        record["delete_tmux_identity"] = identity;
        write_private_json(&path, &record);
        seed_activity_state(
            &self.state,
            "owner",
            incarnation,
            "working",
            json!({"provider_turn_id":"fixture-turn", "started_at":"2030-01-01T00:00:01Z"}),
            Value::Null,
        );
        let activity = self.state.join("sessions/owner/activity.json");
        let mut document: Value = serde_json::from_slice(&fs::read(&activity).unwrap()).unwrap();
        document["runtime_generation"] = json!(generation);
        write_private_json(&activity, &document);
    }

    fn run(&self, incarnation: &str, arguments: &[&str]) -> CmdOutput {
        let selected = self
            .state
            .join("sessions/owner/coordination")
            .join(format!("capability-{}", digest(incarnation)));
        self.run_with_selector(incarnation, &selected, arguments)
    }

    fn run_with_selector(
        &self,
        incarnation: &str,
        selected: &Path,
        arguments: &[&str],
    ) -> CmdOutput {
        let mut args = vec!["--state-dir", self.state.to_str().unwrap()];
        args.extend_from_slice(arguments);
        run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .without_ambient_managed_session_env()
                .with_cwd(self.root.path())
                .with_env("AGENT_SESSION_ID", "owner")
                .with_env("AGENT_SESSION_RUNTIME_ID", incarnation)
                .with_env("AGENT_SESSION_CAPABILITY_FILE", selected.to_str().unwrap()),
        )
    }

    fn set(&self, incarnation: &str) -> CmdOutput {
        let selected = self
            .state
            .join("sessions/owner/coordination")
            .join(format!("capability-{}", digest(incarnation)));
        self.set_with_selector(incarnation, &selected)
    }

    fn set_with_selector(&self, incarnation: &str, selected: &Path) -> CmdOutput {
        self.run_with_selector(
            incarnation,
            selected,
            &[
                "work-context",
                "set",
                "--if-absent",
                "--tier",
                "issue",
                "--repository",
                "example/repository",
                "--issue",
                "1",
                "--path",
                "user-edit.txt",
                "--format",
                "json",
            ],
        )
    }

    fn match_broker_to_record(&self) {
        let document: Value = serde_json::from_slice(
            &fs::read(self.state.join("sessions/owner/session.json")).unwrap(),
        )
        .unwrap();
        let record: agent_session::SessionRecord = serde_json::from_slice(
            &fs::read(self.state.join("sessions/owner/session.json")).unwrap(),
        )
        .unwrap();
        let runtime = agent_session::coordination_runtime_evidence(
            &agent_session::CliContext {
                state_dir: self.state.clone(),
                host: None,
            },
            &record,
        )
        .unwrap();
        // Keep every later binding fence valid so only the caller proof rejects.
        assert_eq!(
            runtime.status,
            agent_session::CoordinationRuntimeStatus::Running
        );
        rewrite_registry(&self.state, |registry| {
            registry["brokers"]["owner"]["runtime_identity"] =
                document["delete_tmux_identity"].clone();
            registry["brokers"]["owner"]["runtime_identity_digest"] =
                json!(runtime.identity_digest);
        });
    }

    fn assert_refused(&self, output: &CmdOutput, reason: &str, before: &Value) {
        assert_eq!(output.code, 65, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["details"]["reason_code"],
            reason
        );
        assert_eq!(load_coordination_registry(&self.state), *before);
        assert!(!Path::new(&capability(&self.state, "owner")).exists());
        assert_eq!(
            fs::read_to_string(self.checkout.join("user-edit.txt")).unwrap(),
            "unfinished user edit\n"
        );
    }

    fn assert_owner_works(&self, incarnation: &str, claim_id: &Value) {
        let status = self.run(incarnation, &["work-context", "status", "--format", "json"]);
        assert_eq!(status.code, 0, "{}", status.stdout_text());
        let shown = self.run(
            incarnation,
            &[
                "work-context",
                "show",
                "--session",
                "owner",
                "--format",
                "json",
            ],
        );
        assert_eq!(shown.code, 0, "{}", shown.stdout_text());
        assert_eq!(&shown.stdout_json()["data"]["claim_id"], claim_id);
        let lease = self.admit(incarnation, claim_id);
        self.complete(incarnation, &lease);
        assert_eq!(
            fs::read_to_string(self.checkout.join("user-edit.txt")).unwrap(),
            "unfinished user edit\n"
        );
        let journal = self.run(
            incarnation,
            &["logs", "owner", "--lifecycle", "--format", "json"],
        );
        assert_eq!(journal.code, 0, "{}", journal.stdout_text());
        let data = journal.stdout_json()["data"].clone();
        let row = data["records"].as_array().unwrap().last().unwrap();
        assert_eq!(
            row["operation"],
            if incarnation == "original" {
                "owner-capability-reissue"
            } else {
                "owner-capability-rebind"
            }
        );
        assert_eq!(row["result"]["ok"], true);
        assert_eq!(
            row["session_generation"],
            if incarnation == "original" { 1 } else { 2 }
        );
        assert!(journal.stdout_text().len() < 4096);
        assert!(!journal.stdout_text().contains(TOKEN));
    }

    fn admit(&self, incarnation: &str, claim_id: &Value) -> Value {
        let registry = load_coordination_registry(&self.state);
        let revision = registry["claims"]
            .as_array()
            .unwrap()
            .iter()
            .find(|claim| &claim["claim_id"] == claim_id)
            .unwrap()["revision"]
            .as_u64()
            .unwrap()
            .to_string();
        let targets = self.root.path().join("targets.json");
        write_private_json(
            &targets,
            &json!({
                "schema_version": "agent-session.operation-targets.v1",
                "targets": [{"kind":"path-exact", "repository":"example/repository", "value":"user-edit.txt"}]
            }),
        );
        let execution = self.root.path().join("execution-token");
        nils_common::fs::write_atomic(
            &execution,
            b"synthetic-execution-token-material-0001",
            0o600,
        )
        .unwrap();
        let admitted = self.run(
            incarnation,
            &[
                "work-context",
                "admit",
                "--session",
                "owner",
                "--claim",
                claim_id.as_str().unwrap(),
                "--if-revision",
                &revision,
                "--targets-file",
                targets.to_str().unwrap(),
                "--operation",
                "fixture",
                "--execution-token-file",
                execution.to_str().unwrap(),
                "--idempotency-key",
                "owner-recovery-admit-0001",
                "--format",
                "json",
            ],
        );
        assert_eq!(admitted.code, 0, "{}", admitted.stdout_text());
        admitted.stdout_json()["data"].clone()
    }

    fn complete(&self, incarnation: &str, lease: &Value) {
        let execution = self.root.path().join("execution-token");
        let completed = self.run(
            incarnation,
            &[
                "work-context",
                "complete",
                "--session",
                "owner",
                "--lease",
                lease["lease_id"].as_str().unwrap(),
                "--if-revision",
                &lease["revision"].as_u64().unwrap().to_string(),
                "--execution-token-file",
                execution.to_str().unwrap(),
                "--outcome",
                "pass",
                "--idempotency-key",
                "owner-recovery-complete-0001",
                "--format",
                "json",
            ],
        );
        assert_eq!(completed.code, 0, "{}", completed.stdout_text());
    }

    fn stop_predecessor(&self) {
        let mut exited = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let absent_pid = exited.id();
        exited.wait().unwrap();
        rewrite_registry(&self.state, |registry| {
            registry["brokers"]["owner"]["runtime_identity"] = json!({
                "session_id": "$2", "pane_id": "%2", "pane_pid": absent_pid,
                "process_group_id": absent_pid, "process_session_id": absent_pid,
                "pid_namespace": self.identity["pid_namespace"]
            });
            registry["brokers"]["owner"]["heartbeat_epoch"] = json!(0);
        });
        fs::remove_file(self.state.join("sessions/owner/coordination/heartbeat")).unwrap();
    }
}

#[test]
fn owner_capability_recovery_after_file_loss_preserves_current_claim() {
    let fixture = Fixture::new();
    let before = load_coordination_registry(&fixture.state);
    let claim_id = before["claims"][0]["claim_id"].clone();
    fs::remove_file(capability(&fixture.state, "owner")).unwrap();
    let recovered = fixture.set("original");
    assert_eq!(recovered.code, 0, "{}", recovered.stdout_text());
    fixture.assert_owner_works("original", &claim_id);
}

#[test]
fn owner_capability_recovery_rebinds_split_replacement_and_stopped_claim() {
    let fixture = Fixture::new();
    let before = load_coordination_registry(&fixture.state);
    let claim_id = before["claims"][0]["claim_id"].clone();
    fixture.publish_runtime("replacement", 2);
    // The predecessor has exited and its heartbeat is stale. Its claim is still active.
    fixture.stop_predecessor();
    let recovered = fixture.set("replacement");
    assert_eq!(recovered.code, 0, "{}", recovered.stdout_text());
    fixture.assert_owner_works("replacement", &claim_id);
    let after = load_coordination_registry(&fixture.state);
    assert_eq!(after["claims"][0]["session_incarnation"], "replacement");
    assert_eq!(
        after["claims"][0]["revision"],
        before["claims"][0]["revision"].as_u64().unwrap() + 1
    );
    for field in [
        "claim_id",
        "scopes",
        "repositories",
        "expires_at",
        "expires_at_epoch",
    ] {
        assert_eq!(
            after["claims"][0][field], before["claims"][0][field],
            "{field}"
        );
    }
}

#[test]
fn owner_capability_recovery_refuses_stale_peer_and_unproven_runtime_without_state_change() {
    for mismatch in ["stale", "peer", "birth", "namespace"] {
        let fixture = Fixture::new();
        fs::remove_file(capability(&fixture.state, "owner")).unwrap();
        let path = fixture.state.join("sessions/owner/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let mut peer = None;
        match mismatch {
            "peer" => {
                let child = super::cli::spawn_scoped_test_process_group().unwrap();
                let stat = fs::read_to_string(format!("/proc/{}/stat", child.pid())).unwrap();
                let fields: Vec<_> = stat
                    .rsplit_once(") ")
                    .unwrap()
                    .1
                    .split_whitespace()
                    .collect();
                record["delete_tmux_identity"]["pane_pid"] = json!(child.pid());
                record["delete_tmux_identity"]["pane_start_time"] =
                    json!(fields[19].parse::<u64>().unwrap());
                record["delete_tmux_identity"]["process_group_id"] =
                    json!(fields[2].parse::<i32>().unwrap());
                record["delete_tmux_identity"]["process_session_id"] =
                    json!(fields[3].parse::<i32>().unwrap());
                peer = Some(child);
            }
            "birth" => record["delete_tmux_identity"]["pane_start_time"] = json!(1),
            "namespace" => record["delete_tmux_identity"]["pid_namespace"]["inode"] = json!(1),
            _ => {}
        }
        write_private_json(&path, &record);
        fixture.match_broker_to_record();
        let before = load_coordination_registry(&fixture.state);
        let refused = fixture.set_with_selector(
            if mismatch == "stale" {
                "stale"
            } else {
                "original"
            },
            Path::new(&capability(&fixture.state, "owner")),
        );
        fixture.assert_refused(
            &refused,
            if mismatch == "stale" {
                "owner-recovery-stale-runtime"
            } else {
                "owner-recovery-caller-unverified"
            },
            &before,
        );
        drop(peer);
    }
}

#[test]
fn owner_capability_recovery_requires_the_canonical_selector() {
    let fixture = Fixture::new();
    fs::remove_file(capability(&fixture.state, "owner")).unwrap();
    let before = load_coordination_registry(&fixture.state);
    let selected = fixture.root.path().join("wrong-capability");
    let refused = fixture.set_with_selector("original", &selected);
    fixture.assert_refused(&refused, "owner-recovery-selector-mismatch", &before);
    assert!(!selected.exists());
}

#[test]
fn owner_capability_recovery_waits_for_the_session_transition_lock() {
    let fixture = Fixture::new();
    fs::remove_file(capability(&fixture.state, "owner")).unwrap();
    let before = load_coordination_registry(&fixture.state);
    let context = agent_session::CliContext {
        state_dir: fixture.state.clone(),
        host: None,
    };
    let held = agent_session::acquire_session_record_lock(&context, "owner").unwrap();
    let started = std::time::Instant::now();
    let refused = std::thread::scope(|scope| {
        let (send, receive) = std::sync::mpsc::channel();
        let fixture = &fixture;
        scope.spawn(move || send.send(fixture.set("original")).unwrap());
        let result = receive.recv_timeout(std::time::Duration::from_secs(5));
        // Even a missing recovery fence must not hang the fixture at the later
        // claim fence. Release it before inspecting the CLI's final response.
        drop(held);
        result.unwrap_or_else(|_| {
            receive
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap()
        })
    });
    fixture.assert_refused(&refused, "owner-recovery-session-busy", &before);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(fixture.set("original").code, 0);
}

#[test]
fn owner_capability_recovery_requires_a_post_restoration_heartbeat_and_allows_retry() {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let fixture = Fixture::new();
    let before = load_coordination_registry(&fixture.state);
    let claim = before["claims"][0]["claim_id"].clone();
    let lock_path = fixture
        .state
        .join("sessions/owner/coordination")
        .join(format!("broker-{}.lock", digest("original")));
    let held = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    fs::remove_file(capability(&fixture.state, "owner")).unwrap();
    let first = fixture.set("original");
    let retry = fixture.set("original");
    for result in [&first, &retry] {
        assert_ne!(result.code, 0, "{}", result.stdout_text());
        assert_eq!(
            result.stdout_json()["error"]["details"]["reason_code"],
            "owner-recovery-heartbeat-unavailable"
        );
        assert_eq!(result.stdout_json()["error"]["details"]["retryable"], true);
    }
    assert_eq!(
        load_coordination_registry(&fixture.state)["claims"],
        before["claims"]
    );
    // The restored token remains complete, but capability-authenticated reads stay fenced.
    assert!(Path::new(&capability(&fixture.state, "owner")).is_file());
    let status = fixture.run("original", &["work-context", "status", "--format", "json"]);
    assert_eq!(status.code, 65, "{}", status.stdout_text());
    assert_eq!(
        status.stdout_json()["error"]["details"]["reason_code"],
        "broker-not-ready"
    );
    drop(held);
    let recovered = fixture.set("original");
    assert_eq!(recovered.code, 0, "{}", recovered.stdout_text());
    fixture.assert_owner_works("original", &claim);
}

#[test]
fn owner_capability_recovery_does_not_replace_wrong_or_unsafe_capability() {
    for unsafe_path in [false, true] {
        let fixture = Fixture::new();
        let selected = capability(&fixture.state, "owner");
        if unsafe_path {
            fs::remove_file(&selected).unwrap();
            std::os::unix::fs::symlink(fixture.checkout.join("user-edit.txt"), &selected).unwrap();
        } else {
            fs::write(&selected, "synthetic-wrong-capability-material-0001").unwrap();
        }
        let before = load_coordination_registry(&fixture.state);
        let refused = fixture.set("original");
        assert_eq!(refused.code, 65, "{}", refused.stdout_text());
        assert_eq!(load_coordination_registry(&fixture.state), before);
        assert_eq!(
            fs::read_to_string(fixture.checkout.join("user-edit.txt")).unwrap(),
            "unfinished user edit\n"
        );
    }
}

#[test]
fn owner_capability_recovery_refuses_a_live_predecessor_without_state_change() {
    let fixture = Fixture::new();
    fixture.publish_runtime("replacement", 2);
    let before = load_coordination_registry(&fixture.state);
    let refused = fixture.set("replacement");
    assert_eq!(refused.code, 65, "{}", refused.stdout_text());
    assert_eq!(load_coordination_registry(&fixture.state), before);
}

#[test]
fn owner_capability_recovery_preserves_same_runtime_operation_for_authenticated_completion() {
    let fixture = Fixture::new();
    let claim = load_coordination_registry(&fixture.state)["claims"][0]["claim_id"].clone();
    let lease = fixture.admit("original", &claim);
    let before = load_coordination_registry(&fixture.state)["operations"].clone();
    fs::remove_file(capability(&fixture.state, "owner")).unwrap();
    let recovered = fixture.set("original");
    assert_eq!(recovered.code, 0, "{}", recovered.stdout_text());
    assert_eq!(
        load_coordination_registry(&fixture.state)["operations"],
        before
    );
    fixture.complete("original", &lease);
}

#[test]
fn owner_capability_recovery_refuses_split_with_unresolved_operation_or_unknown_predecessor() {
    for uncertainty in ["operation", "namespace"] {
        let fixture = Fixture::new();
        if uncertainty == "operation" {
            let claim = load_coordination_registry(&fixture.state)["claims"][0]["claim_id"].clone();
            fixture.admit("original", &claim);
        }
        fixture.publish_runtime("replacement", 2);
        fixture.stop_predecessor();
        if uncertainty == "namespace" {
            rewrite_registry(&fixture.state, |registry| {
                registry["brokers"]["owner"]["runtime_identity"]["pid_namespace"]["inode"] =
                    json!(1);
            });
        }
        let before = load_coordination_registry(&fixture.state);
        let refused = fixture.set("replacement");
        assert_eq!(refused.code, 65, "{}", refused.stdout_text());
        assert_eq!(
            refused.stdout_json()["error"]["details"]["reason_code"],
            if uncertainty == "operation" {
                "owner-recovery-operation-pending"
            } else {
                "owner-recovery-predecessor-unverified"
            }
        );
        assert_eq!(load_coordination_registry(&fixture.state), before);
    }
}

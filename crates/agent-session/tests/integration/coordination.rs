use std::fs::{self, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nils_test_support::bin;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::json;
use sha2::{Digest, Sha256};

#[cfg(target_os = "linux")]
use super::cli::{TestProcessGroup, fake_agent, fake_tmux, spawn_scoped_test_process_group};

#[test]
fn worktree_lifecycle_state_root_override_cannot_split_removal_fence() {
    use nils_common::worktree_lifecycle::Guard;
    for verb in ["start", "run"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let checkout = tmp.path().join("checkout");
        init_checkout(&checkout, "https://example.invalid/example/repository.git");
        let removal_state = tmp.path().join("removal-state");
        let session_state = tmp.path().join("session-state");
        let _guard = Guard::acquire(&removal_state, &checkout).unwrap();
        let tmux = tmp.path().join("tmux");
        let marker = tmp.path().join("launch-attempted");
        fs::write(&tmux, "#!/bin/sh\nfor arg in \"$@\"; do\n if [ \"$arg\" = new-session ]; then touch \"$LIFECYCLE_LAUNCH_MARKER\"; exit 1; fi\ndone\nexit 0\n").unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = fake_agent(tmp.path(), "codex");
        let mut args = vec![
            "--state-dir",
            session_state.to_str().unwrap(),
            verb,
            "--agent",
            "codex",
            "--id",
            "state-root-override",
            "--cwd",
            checkout.to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--coordination-mode",
            "advisory",
            "--format",
            "json",
        ];
        if verb == "run" {
            args.extend(["--prompt", "fixture prompt"]);
        }
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .without_ambient_managed_session_env()
                .with_cwd(tmp.path())
                .with_env(
                    "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                    removal_state.to_str().unwrap(),
                )
                .with_env("LIFECYCLE_LAUNCH_MARKER", marker.to_str().unwrap()),
        );
        assert_ne!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "worktree-lifecycle-busy",
            "an explicit session-state override must not split checkout exclusion"
        );
        assert!(
            !marker.exists(),
            "startup bypassed removal's checkout fence"
        );
        assert!(!session_state.join("sessions/state-root-override").exists());
    }
}

#[test]
fn worktree_lifecycle_removal_barrier_prevents_start_and_run_publication() {
    use std::os::unix::fs::OpenOptionsExt;

    for (verb, marker_removed) in [
        ("start", false),
        ("run", false),
        ("start", true),
        ("run", true),
    ] {
        let tmp = tempfile::TempDir::new().unwrap();
        let checkout = tmp.path().join("checkout");
        init_checkout(&checkout, "https://example.invalid/example/repository.git");
        let nested = checkout.join("nested");
        fs::create_dir(&nested).unwrap();
        let state = tmp.path().join("state");
        let directory = state.join("coordination/worktree-lifecycle");
        fs::create_dir_all(&directory).unwrap();
        fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(
            state.join("coordination"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
        let root = fs::canonicalize(&checkout).unwrap();
        let key = Sha256::digest(root.as_os_str().as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join(format!("{key}.lock")))
            .unwrap();
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        if marker_removed {
            // Model deletion's .git-before-root window, with the same
            // persistent lifecycle key still held by the remover.
            fs::remove_dir_all(checkout.join(".git")).unwrap();
        }

        // This fake records attempted launch and exits, so even a failing red
        // regression leaves no provider or pane process behind.
        let tmux = tmp.path().join("tmux");
        let marker = tmp.path().join("launch-attempted");
        fs::write(&tmux, "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ \"$arg\" = new-session ]; then touch \"$LIFECYCLE_LAUNCH_MARKER\"; exit 1; fi\ndone\nexit 0\n").unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = fake_agent(tmp.path(), "codex");
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            verb,
            "--agent",
            "codex",
            "--id",
            "lifecycle-test",
            "--cwd",
            nested.to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--coordination-mode",
            "advisory",
            "--format",
            "json",
        ];
        if verb == "run" {
            args.extend(["--prompt", "fixture prompt"]);
        }
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new()
                .without_ambient_managed_session_env()
                .with_cwd(tmp.path())
                .with_env(
                    "AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME",
                    state.to_str().unwrap(),
                )
                .with_env("LIFECYCLE_LAUNCH_MARKER", marker.to_str().unwrap()),
        );
        assert_ne!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "worktree-lifecycle-busy",
            "startup must observe removal's barrier before publishing: {}",
            output.stdout_text()
        );
        assert!(
            !marker.exists(),
            "startup entered the checkout while removal owned its fence"
        );
        assert!(
            !state.join("sessions/lifecycle-test").exists(),
            "a refused startup must not publish a session"
        );
    }
}

#[test]
fn worktree_lifecycle_guard_survives_launch_and_failure_rollback() {
    use nils_common::worktree_lifecycle::{Error, Guard};
    for verb in ["start", "run"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let checkout = tmp.path().join("checkout");
        init_checkout(&checkout, "https://example.invalid/example/repository.git");
        let state = tmp.path().join("state");
        let marker = tmp.path().join("launch-entered");
        let release = tmp.path().join("launch-release");
        let tmux = tmp.path().join("tmux");
        fs::write(&tmux, "#!/bin/sh\nfor arg in \"$@\"; do\n if [ \"$arg\" = new-session ]; then\n  touch \"$LIFECYCLE_MARKER\"\n  count=0\n  while [ ! -f \"$LIFECYCLE_RELEASE\" ] && [ \"$count\" -lt 500 ]; do sleep 0.01; count=$((count+1)); done\n  exit 1\n fi\ndone\nexit 0\n").unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let agent = fake_agent(tmp.path(), "codex");
        let mut command = Command::new(bin::resolve("agent-session"));
        nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
        command
            .current_dir(tmp.path())
            .args([
                "--state-dir",
                state.to_str().unwrap(),
                verb,
                "--agent",
                "codex",
                "--id",
                "lifecycle-lifetime",
                "--cwd",
                checkout.to_str().unwrap(),
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--agent-bin",
                agent.to_str().unwrap(),
                "--coordination-mode",
                "advisory",
                "--format",
                "json",
            ])
            .env("AGENT_RUNTIME_CHECKOUT_LEASE_STATE_HOME", &state)
            .env("LIFECYCLE_MARKER", &marker)
            .env("LIFECYCLE_RELEASE", &release)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if verb == "run" {
            command.args(["--prompt", "fixture prompt"]);
        }
        let child = command.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let held = Guard::acquire(&state, &checkout);
        // Release before asserting, so even a regression reaps the fake launch.
        fs::write(&release, "release").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            marker.exists(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        assert!(
            matches!(held, Err(Error::Busy)),
            "guard dropped before launch finished"
        );
        assert!(!output.status.success());
        assert!(
            Guard::acquire(&state, &checkout).is_ok(),
            "rollback leaked its guard"
        );
    }
}

fn run(dir: &Path, args: &[&str]) -> CmdOutput {
    run_resolved("agent-session", args, &CmdOptions::new().with_cwd(dir))
}

pub(super) fn run_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    run_resolved(
        "agent-session",
        args,
        &CmdOptions::new().with_cwd(dir).with_envs(envs),
    )
}

fn seed_session(state_dir: &Path, id: &str, incarnation: &str) {
    seed_session_at(
        state_dir,
        id,
        incarnation,
        Path::new("/fixture/repository"),
        None,
    );
}

fn seed_session_at(
    state_dir: &Path,
    id: &str,
    incarnation: &str,
    cwd: &Path,
    coordination_mode: Option<&str>,
) {
    let session_dir = state_dir.join("sessions").join(id);
    fs::create_dir_all(&session_dir).expect("session directory");
    fs::set_permissions(state_dir, fs::Permissions::from_mode(0o700)).expect("state mode");
    fs::set_permissions(
        state_dir.join("sessions"),
        fs::Permissions::from_mode(0o700),
    )
    .expect("sessions mode");
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700)).expect("session mode");
    let mut record = json!({
        "schema_version": "agent-session.session.v1",
        "id": id,
        "agent": "codex",
        "mode": "interactive",
        "title": "coordination fixture",
        "title_revision": 0,
        "cwd": cwd,
        "tmux_session": format!("hs-codex-{id}"),
        "prompt_file": null,
        "log_file": null,
        "created_at": "2030-01-01T00:00:00Z",
        "updated_at": "2030-01-01T00:00:00Z",
        "runtime": {
            "kind": "tmux",
            "tmux_session": format!("hs-codex-{id}"),
            "generation": 1,
            "started_at": "2030-01-01T00:00:00Z",
            "launch_id": incarnation
        }
    });
    if let Some(mode) = coordination_mode {
        record["coordination_mode"] = json!(mode);
    }
    let path = session_dir.join("session.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&record).expect("session json"),
    )
    .expect("write session");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("record mode");
}

fn seed_activity_state(
    state_dir: &Path,
    id: &str,
    incarnation: &str,
    phase: &str,
    current_turn: serde_json::Value,
    last_turn: serde_json::Value,
) {
    seed_activity_state_with_source(
        state_dir,
        id,
        incarnation,
        phase,
        "runtime",
        current_turn,
        last_turn,
    );
}

fn seed_activity_state_with_source(
    state_dir: &Path,
    id: &str,
    incarnation: &str,
    phase: &str,
    source_kind: &str,
    current_turn: serde_json::Value,
    last_turn: serde_json::Value,
) {
    let path = state_dir.join("sessions").join(id).join("activity.json");
    write_private_json(
        &path,
        &json!({
            "schema_version": "agent-session.activity.v1",
            "runtime_id": incarnation,
            "runtime_generation": 1,
            "state": {
                "schema_version": "agent-session.turn-state.v1",
                "phase": phase,
                "phase_changed_at": "2030-01-01T00:00:00Z",
                "revision": 1,
                "source": {
                    "kind": source_kind,
                    "provider": null,
                    "confidence": "authoritative"
                },
                "current_turn": current_turn,
                "last_turn": last_turn
            },
            "pending_attention": [],
            "seen_event_count": 0
        }),
    );
}

fn seed_live_runtime_identity(
    state_dir: &Path,
    id: &str,
    incarnation: &str,
    tmux_slot: u32,
) -> TestProcessGroup {
    let runtime = spawn_scoped_test_process_group().expect("live runtime identity");
    let runtime_pid = runtime.pid() as libc::pid_t;
    let runtime_identity = json!({
        "launch_id": incarnation,
        "session_id": format!("${tmux_slot}"),
        "pane_id": format!("%{tmux_slot}"),
        "pane_pid": runtime_pid,
        "process_group_id": runtime_pid,
        "process_session_id": runtime_pid,
        "pid_namespace": current_pid_namespace_identity()
    });
    let session_path = state_dir.join("sessions").join(id).join("session.json");
    let mut session: serde_json::Value =
        serde_json::from_slice(&fs::read(&session_path).expect("session record"))
            .expect("session json");
    session["delete_tmux_identity"] = runtime_identity;
    write_private_json(&session_path, &session);
    runtime
}

fn current_pid_namespace_identity() -> serde_json::Value {
    #[cfg(target_os = "linux")]
    {
        let namespace = fs::metadata("/proc/self/ns/pid").expect("current PID namespace metadata");
        json!({
            "device": namespace.dev(),
            "inode": namespace.ino(),
            "boot_id": fs::read_to_string("/proc/sys/kernel/random/boot_id")
                .expect("boot id")
                .trim()
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        serde_json::Value::Null
    }
}

fn init_checkout(path: &Path, remote: &str) {
    fs::create_dir_all(path).expect("checkout directory");
    let init = Command::new("git")
        .current_dir(path)
        .args(["init", "--quiet", "--initial-branch", "main"])
        .status()
        .expect("git init");
    assert!(init.success());
    let remote_add = Command::new("git")
        .current_dir(path)
        .args(["remote", "add", "origin", remote])
        .status()
        .expect("git remote add");
    assert!(remote_add.success());
}

pub(super) fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn seed_brokers(state_dir: &Path, sessions: &[(&str, &str, &str)]) {
    let sessions = sessions
        .iter()
        .map(|(id, incarnation, capability)| {
            (
                *id,
                *incarnation,
                *capability,
                Path::new("/fixture/repository"),
                None,
            )
        })
        .collect::<Vec<_>>();
    seed_brokers_at(state_dir, &sessions);
}

fn seed_brokers_at(state_dir: &Path, sessions: &[(&str, &str, &str, &Path, Option<&str>)]) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let mut brokers = serde_json::Map::new();
    for (id, incarnation, capability, cwd, coordination_mode) in sessions {
        seed_session_at(state_dir, id, incarnation, cwd, *coordination_mode);
        seed_activity_state(
            state_dir,
            id,
            incarnation,
            "starting",
            serde_json::Value::Null,
            serde_json::Value::Null,
        );
        let capability_dir = state_dir.join("sessions").join(id).join("coordination");
        fs::create_dir(&capability_dir).expect("capability directory");
        fs::set_permissions(&capability_dir, fs::Permissions::from_mode(0o700))
            .expect("capability dir mode");
        let capability_path = capability_dir.join(format!("capability-{}", digest(incarnation)));
        fs::write(&capability_path, capability).expect("capability");
        fs::set_permissions(&capability_path, fs::Permissions::from_mode(0o600))
            .expect("capability mode");
        let checkpoint_path = capability_dir.join(format!(
            "main-agent-checkpoint-{}.json",
            digest(incarnation)
        ));
        fs::write(&checkpoint_path, []).expect("checkpoint");
        fs::set_permissions(&checkpoint_path, fs::Permissions::from_mode(0o600))
            .expect("checkpoint mode");
        let heartbeat_path = capability_dir.join("heartbeat");
        fs::write(&heartbeat_path, format!("{incarnation}:{now}\n")).expect("heartbeat");
        fs::set_permissions(&heartbeat_path, fs::Permissions::from_mode(0o600))
            .expect("heartbeat mode");
        brokers.insert(
            (*id).to_string(),
            json!({
                "session_id": id,
                "incarnation": incarnation,
                "coordination_mode": coordination_mode.unwrap_or("advisory"),
                "capability_digest": digest(capability),
                "generation": 1,
                "state": "ready",
                "heartbeat_at": "2030-01-01T00:00:00Z",
                "heartbeat_epoch": now
            }),
        );
    }
    let coordination = state_dir.join("coordination");
    fs::create_dir(&coordination).expect("coordination root");
    fs::set_permissions(&coordination, fs::Permissions::from_mode(0o700))
        .expect("coordination mode");
    let registry = coordination.join("registry.json");
    fs::write(
        &registry,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.coordination-registry.v1",
            "fingerprint_epoch": 1,
            "fingerprint_key": "fixture-private-fingerprint-key-material-0000000001",
            "brokers": brokers,
            "claims": [],
            "operations": [],
            "messages": [],
            "receipts": {},
            "notifications": {}
        }))
        .expect("registry json"),
    )
    .expect("registry");
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).expect("registry mode");
}

fn set_context_for_recorded_cwd(root: &Path, recorded_cwd: &Path, command_cwd: &Path) -> CmdOutput {
    let state_dir = root.join("state");
    fs::create_dir_all(&state_dir).expect("state");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            recorded_cwd,
            Some("advisory"),
        )],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    run_with_env(
        command_cwd,
        &[
            "work-context",
            "set",
            "--summary",
            "boundary classification",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
            ("AGENT_SESSION_STATE_DIR", state.as_ref()),
        ],
    )
}

fn grant_checkout_shell(state_dir: &Path, session_ids: &[&str]) {
    rewrite_registry(state_dir, |registry| {
        for claim in registry["claims"].as_array_mut().expect("claims") {
            if session_ids
                .iter()
                .any(|session_id| claim["session_id"] == *session_id)
                && claim["state"] == "active"
            {
                claim["checkout_shell_grant"] = json!(true);
            }
        }
    });
}

pub(super) fn capability(state_dir: &Path, id: &str) -> String {
    let record: serde_json::Value = serde_json::from_slice(
        &fs::read(state_dir.join("sessions").join(id).join("session.json"))
            .expect("session record"),
    )
    .expect("session json");
    let incarnation = record["runtime"]["launch_id"]
        .as_str()
        .expect("session incarnation");
    state_dir
        .join("sessions")
        .join(id)
        .join(format!("coordination/capability-{}", digest(incarnation)))
        .to_string_lossy()
        .to_string()
}

fn candidate(path: &Path, prefix: &str, summary: &str) {
    fs::write(
        path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.work-context-input.v1",
            "intent": "implementation",
            "tier": "program",
            "repositories": ["example/repository"],
            "worktrees": [],
            "provider_refs": [],
            "plan_refs": [],
            "scopes": [{
                "kind": "path-prefix",
                "repository": "example/repository",
                "value": prefix.trim_end_matches('/')
            }],
            "summary": summary
        }))
        .expect("candidate json"),
    )
    .expect("candidate");
}

pub(super) fn data(output: &CmdOutput) -> serde_json::Value {
    output.stdout_json()["data"].clone()
}

pub(super) fn write_private_json(path: &Path, value: &serde_json::Value) {
    let bytes = serde_json::to_vec_pretty(value).expect("private json");
    nils_common::fs::write_atomic(path, &bytes, 0o600).expect("write private json atomically");
}

fn rewrite_registry(state_dir: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let path = state_dir.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("registry")).expect("registry json");
    mutate(&mut registry);
    fs::write(
        &path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write registry");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("registry mode");
}

fn coordination_registry(state_dir: &Path) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(state_dir.join("coordination/registry.json")).expect("coordination registry"),
    )
    .expect("coordination registry json")
}

fn load_coordination_registry(state_dir: &Path) -> serde_json::Value {
    serde_json::from_slice(
        &fs::read(state_dir.join("coordination/registry.json")).expect("coordination registry"),
    )
    .expect("coordination registry json")
}

#[test]
fn coordination_help_exposes_closed_work_context_and_mailbox_command_families() {
    let tmp = tempfile::TempDir::new().expect("tempdir");

    let work_context = run(tmp.path(), &["work-context", "--help"]);
    assert_eq!(
        work_context.code,
        0,
        "stderr={}",
        work_context.stderr_text()
    );
    let work_context_help = work_context.stdout_text();
    for command in [
        "status",
        "set",
        "clear",
        "advise",
        "acknowledge",
        "claim",
        "show",
        "check",
        "renew",
        "release",
        "admit",
        "complete",
        "reconcile",
    ] {
        assert!(
            work_context_help.contains(command),
            "missing work-context command {command}: {work_context_help}"
        );
    }
    let set_help = run(tmp.path(), &["work-context", "set", "--help"]);
    assert_eq!(set_help.code, 0, "stderr={}", set_help.stderr_text());
    assert!(!set_help.stdout_text().contains("--plan-ref"));

    let start = run(tmp.path(), &["start", "--help"]);
    assert_eq!(start.code, 0, "stderr={}", start.stderr_text());
    assert!(start.stdout_text().contains("--coordination-mode"));
    assert!(start.stdout_text().contains("advisory"));
    assert!(start.stdout_text().contains("enforce"));
    assert!(start.stdout_text().contains("off"));

    let broker = run(tmp.path(), &["broker", "--help"]);
    assert_eq!(broker.code, 0, "stderr={}", broker.stderr_text());
    let broker_help = broker.stdout_text();
    for command in ["status", "adopt", "reconcile"] {
        assert!(
            broker_help.contains(command),
            "missing broker command {command}: {broker_help}"
        );
    }

    let message = run(tmp.path(), &["message", "--help"]);
    assert_eq!(message.code, 0, "stderr={}", message.stderr_text());
    let message_help = message.stdout_text();
    for command in ["send", "inbox", "show", "ack", "reply", "wait"] {
        assert!(
            message_help.contains(command),
            "missing message command {command}: {message_help}"
        );
    }

    let send = run(tmp.path(), &["message", "send", "--help"]);
    assert_eq!(send.code, 0, "stderr={}", send.stderr_text());
    assert!(send.stdout_text().contains("eventual fixed notification"));
    assert!(
        send.stdout_text()
            .contains("defaults to AGENT_SESSION_CAPABILITY_FILE")
    );

    let inbox = run(tmp.path(), &["message", "inbox", "--help"]);
    assert_eq!(inbox.code, 0, "stderr={}", inbox.stderr_text());
    assert!(
        inbox
            .stdout_text()
            .contains("defaults to AGENT_SESSION_CAPABILITY_FILE")
    );

    let reply = run(tmp.path(), &["message", "reply", "--help"]);
    assert_eq!(reply.code, 0, "stderr={}", reply.stderr_text());
    assert!(reply.stdout_text().contains("eventual fixed notification"));
}

#[test]
fn advisory_presence_defaults_for_unclaimed_sessions_and_classifies_overlap() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                None,
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let managed_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];

    let status = run_with_env(
        tmp.path(),
        &["work-context", "status", "--format", "json"],
        &managed_env,
    );
    assert_eq!(status.code, 0, "stderr={}", status.stderr_text());
    assert_eq!(data(&status)["managed"], true);
    assert_eq!(data(&status)["mode"], "advisory");
    assert_eq!(data(&status)["presence"]["state"], "active");
    assert!(data(&status)["context"].is_null());

    let advised = run_with_env(
        tmp.path(),
        &["work-context", "advise", "--format", "json"],
        &managed_env,
    );
    assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
    assert_eq!(data(&advised)["mode"], "advisory");
    assert_eq!(data(&advised)["severity"], "warning");
    assert_eq!(data(&advised)["suppressed"], false);
    assert_eq!(data(&advised)["reasons"][0]["code"], "same-worktree");
    assert_eq!(data(&advised)["peers"][0]["session_id"], "beta");
    assert!(
        !advised
            .stdout_text()
            .contains(checkout.to_string_lossy().as_ref())
    );
    assert!(
        !advised
            .stdout_text()
            .contains("beta-private-capability-material")
    );
}

#[test]
fn advisory_presence_distinguishes_same_repository_from_same_worktree() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let alpha_checkout = tmp.path().join("alpha-checkout");
    let beta_checkout = tmp.path().join("beta-checkout");
    init_checkout(&alpha_checkout, "git@github.com:example/repository.git");
    init_checkout(&beta_checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                alpha_checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                beta_checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let managed_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let advised = run_with_env(
        tmp.path(),
        &["work-context", "advise", "--format", "json"],
        &managed_env,
    );
    assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
    assert_eq!(data(&advised)["severity"], "info");
    assert_eq!(data(&advised)["reasons"][0]["code"], "same-repository");
}

#[test]
fn unmanaged_and_off_sessions_are_explicit_nonparticipants() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let unmanaged = run_with_env(
        tmp.path(),
        &["work-context", "advise", "--format", "json"],
        &[
            ("AGENT_SESSION_ID", ""),
            ("AGENT_SESSION_CAPABILITY_FILE", ""),
            ("AGENT_SESSION_STATE_DIR", ""),
        ],
    );
    assert_eq!(unmanaged.code, 0, "stderr={}", unmanaged.stderr_text());
    assert_eq!(data(&unmanaged)["managed"], false);
    assert_eq!(data(&unmanaged)["mode"], "off");
    assert_eq!(data(&unmanaged)["severity"], "none");

    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            checkout.as_path(),
            Some("off"),
        )],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let off = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
            ("AGENT_SESSION_STATE_DIR", state.as_ref()),
        ],
    );
    assert_eq!(off.code, 0, "stderr={}", off.stderr_text());
    assert_eq!(data(&off)["managed"], true);
    assert_eq!(data(&off)["mode"], "off");
    assert_eq!(data(&off)["severity"], "none");
}

#[test]
fn advisory_targets_require_the_exact_v1_schema() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            checkout.as_path(),
            Some("advisory"),
        )],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let cases = [
        (
            "valid",
            json!({
                "schema_version": "agent-session.operation-targets.v1",
                "targets": [],
                "provider_refs": [],
                "checkouts": [],
                "descendant": null
            }),
            true,
        ),
        (
            "missing",
            json!({ "targets": [], "provider_refs": [] }),
            false,
        ),
        (
            "future",
            json!({
                "schema_version": "agent-session.operation-targets.v2",
                "targets": [],
                "provider_refs": []
            }),
            false,
        ),
        (
            "misspelled",
            json!({
                "schema_version": "agent-session.operation-targets.v1",
                "tragets": [],
                "provider_refs": []
            }),
            false,
        ),
    ];
    for (name, body, succeeds) in cases {
        let path = tmp.path().join(format!("{name}.json"));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&body).expect("targets json"),
        )
        .expect("write targets");
        let output = run_with_env(
            &checkout,
            &[
                "work-context",
                "advise",
                "--targets-file",
                path.to_str().expect("target path"),
                "--format",
                "json",
            ],
            &env,
        );
        assert_eq!(
            output.code == 0,
            succeeds,
            "case={name} stdout={} stderr={}",
            output.stdout_text(),
            output.stderr_text()
        );
        if !succeeds {
            assert_eq!(
                output.stdout_json()["schema_version"],
                "cli.agent-session.work-context-advise.v1"
            );
            assert_eq!(
                output.stdout_json()["error"]["code"],
                "invalid-operation-targets"
            );
        }
    }
}

#[test]
fn clear_advisories_do_not_rewrite_the_registry_for_target_churn() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            checkout.as_path(),
            Some("advisory"),
        )],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let registry = state_dir.join("coordination/registry.json");
    let before = fs::read(&registry).expect("registry before");
    for target in ["src/one.rs", "src/two.rs"] {
        let path = tmp.path().join(target.replace('/', "-"));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.operation-targets.v1",
                "targets": [{
                    "kind": "path-exact",
                    "repository": "example/repository",
                    "value": target
                }],
                "provider_refs": [],
                "checkouts": []
            }))
            .expect("targets json"),
        )
        .expect("write targets");
        let advised = run_with_env(
            &checkout,
            &[
                "work-context",
                "advise",
                "--targets-file",
                path.to_str().expect("target path"),
                "--format",
                "json",
            ],
            &env,
        );
        assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
        assert_eq!(data(&advised)["severity"], "none");
    }
    assert_eq!(fs::read(&registry).expect("registry after"), before);
}

#[test]
fn self_targeting_context_set_clear_and_acknowledge_hide_mechanical_inputs() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let alpha_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let beta_env = [
        ("AGENT_SESSION_ID", "beta"),
        ("AGENT_SESSION_CAPABILITY_FILE", beta_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];

    for (envs, summary) in [(&alpha_env, "alpha task"), (&beta_env, "beta task")] {
        let set = run_with_env(
            &checkout,
            &[
                "work-context",
                "set",
                "--tier",
                "program",
                "--summary",
                summary,
                "--issue",
                "1318",
                "--pr",
                "42",
                "--path",
                "src/",
                "--format",
                "json",
            ],
            envs,
        );
        assert_eq!(
            set.code,
            0,
            "stdout={} stderr={}",
            set.stdout_text(),
            set.stderr_text()
        );
        assert_eq!(data(&set)["mode"], "advisory");
        assert_eq!(
            data(&set)["context"]["repositories"][0],
            "example/repository"
        );
        assert_eq!(data(&set)["context"]["provider_refs"][0]["kind"], "issue");
        assert_eq!(data(&set)["context"]["provider_refs"][1]["kind"], "pr");
        assert_eq!(data(&set)["context"]["plan_refs"], json!([]));
        assert_eq!(data(&set)["context"]["scopes"][0]["kind"], "path-prefix");
    }

    let acknowledged = run_with_env(
        &checkout,
        &[
            "work-context",
            "acknowledge",
            "--for",
            "30m",
            "--format",
            "json",
        ],
        &alpha_env,
    );
    assert_eq!(
        acknowledged.code,
        0,
        "stderr={}",
        acknowledged.stderr_text()
    );
    let advised = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
    assert_eq!(data(&advised)["severity"], "warning");
    assert_eq!(data(&advised)["suppressed"], true);

    let cleared = run_with_env(
        &checkout,
        &["work-context", "clear", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(cleared.code, 0, "stderr={}", cleared.stderr_text());
    assert_eq!(data(&cleared)["released"], true);
    let status = run_with_env(
        &checkout,
        &["work-context", "status", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(status.code, 0, "stderr={}", status.stderr_text());
    assert!(data(&status)["context"].is_null());
}

#[test]
fn self_targeting_context_set_distinguishes_proven_non_repository_cwd() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let non_repository = tmp.path().join("non-repository");
    fs::create_dir(&non_repository).expect("non-repository directory");
    let result = set_context_for_recorded_cwd(
        &tmp.path().join("non-repository-case"),
        &non_repository,
        &non_repository,
    );

    assert_ne!(result.code, 0);
    assert_eq!(result.stdout_json()["error"]["code"], "not-in-repository");
}

#[test]
fn self_targeting_context_set_keeps_unprovable_cwd_fail_closed() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    let checkout_alias = tmp.path().join("checkout-alias");
    std::os::unix::fs::symlink(&checkout, &checkout_alias).expect("checkout symlink");
    let symlinked =
        set_context_for_recorded_cwd(&tmp.path().join("symlink-case"), &checkout_alias, &checkout);
    assert_ne!(symlinked.code, 0);
    assert_eq!(
        symlinked.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );

    let missing = tmp.path().join("missing");
    let missing_result =
        set_context_for_recorded_cwd(&tmp.path().join("missing-case"), &missing, tmp.path());
    assert_ne!(missing_result.code, 0);
    assert_eq!(
        missing_result.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );
}

#[test]
fn self_targeting_context_set_keeps_checkout_without_origin_distinct() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    let removed = Command::new("git")
        .current_dir(&checkout)
        .args(["remote", "remove", "origin"])
        .status()
        .expect("remove origin");
    assert!(removed.success());
    let result =
        set_context_for_recorded_cwd(&tmp.path().join("no-origin-case"), &checkout, &checkout);

    assert_ne!(result.code, 0);
    assert_eq!(
        result.stdout_json()["error"]["code"],
        "repository-unavailable"
    );
}

#[test]
fn self_targeting_context_set_if_absent_preserves_an_existing_declaration() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    for retired in ["L0", "L1", "L2", "L3", "program/plan"] {
        let rejected = run_with_env(
            &checkout,
            &[
                "work-context",
                "set",
                "--tier",
                retired,
                "--summary",
                "retired mode",
                "--format",
                "json",
            ],
            &env,
        );
        assert_eq!(
            rejected.code,
            65,
            "tier={retired}: {}",
            rejected.stdout_text()
        );
        assert_eq!(
            rejected.stdout_json()["error"]["code"],
            "invalid-work-context"
        );
    }
    let existing = run_with_env(
        &checkout,
        &[
            "work-context",
            "set",
            "--tier",
            "program/dispatch",
            "--summary",
            "tracked delivery",
            "--issue",
            "213",
            "--path",
            "src/",
            "--format",
            "json",
        ],
        &env,
    );
    assert_eq!(existing.code, 0, "stderr={}", existing.stderr_text());
    // A pre-retirement claim can still be read. Do not reclassify its owner
    // while an unrelated named-mode set-if-absent request checks the registry.
    rewrite_registry(&state_dir, |registry| {
        registry["claims"][0]["tier"] = json!("L2");
        registry["claims"][0]["plan_refs"] = json!(["historical/plan.md"]);
    });

    let ensured = run_with_env(
        &checkout,
        &[
            "work-context",
            "set",
            "--if-absent",
            "--tier",
            "program",
            "--summary",
            "generic DSH context",
            "--format",
            "json",
        ],
        &env,
    );

    assert_eq!(ensured.code, 0, "stderr={}", ensured.stderr_text());
    assert_eq!(data(&ensured)["changed"], false);
    assert_eq!(data(&ensured)["mode"], "advisory");
    assert_eq!(data(&ensured)["context"]["tier"], "L2");
    assert_eq!(
        data(&ensured)["context"]["plan_refs"],
        json!(["historical/plan.md"])
    );
    assert_eq!(coordination_registry(&state_dir)["claims"][0]["tier"], "L2");

    let beta_cap = capability(&state_dir, "beta");
    let beta_env = [
        ("AGENT_SESSION_ID", "beta"),
        ("AGENT_SESSION_CAPABILITY_FILE", beta_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let created = run_with_env(
        &checkout,
        &[
            "work-context",
            "set",
            "--if-absent",
            "--tier",
            "program",
            "--summary",
            "fresh DSH context",
            "--format",
            "json",
        ],
        &beta_env,
    );
    assert_eq!(created.code, 0, "stderr={}", created.stderr_text());
    assert_eq!(data(&created)["changed"], true);
    assert_eq!(data(&created)["context"]["tier"], "program");
    assert_eq!(data(&created)["context"]["summary"], "fresh DSH context");

    let cleared = run_with_env(
        &checkout,
        &["work-context", "clear", "--format", "json"],
        &env,
    );
    assert_eq!(cleared.code, 0, "stderr={}", cleared.stderr_text());
    assert_eq!(data(&cleared)["released"], true);
    assert!(
        coordination_registry(&state_dir)["claims"]
            .as_array()
            .expect("claims")
            .iter()
            .all(|claim| claim["session_id"] != "alpha" || claim["state"] != "active")
    );
}

#[test]
fn concurrent_context_set_if_absent_has_one_winner_without_overwrite() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            checkout.as_path(),
            Some("advisory"),
        )],
    );
    let state = state_dir.to_string_lossy().into_owned();
    let alpha_cap = capability(&state_dir, "alpha");
    let env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_str()),
    ];

    let (first, second) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            run_with_env(
                &checkout,
                &[
                    "work-context",
                    "set",
                    "--if-absent",
                    "--tier",
                    "program",
                    "--summary",
                    "first contender",
                    "--format",
                    "json",
                ],
                &env,
            )
        });
        let second = scope.spawn(|| {
            run_with_env(
                &checkout,
                &[
                    "work-context",
                    "set",
                    "--if-absent",
                    "--tier",
                    "program/dispatch",
                    "--summary",
                    "second contender",
                    "--format",
                    "json",
                ],
                &env,
            )
        });
        (
            first.join().expect("first contender"),
            second.join().expect("second contender"),
        )
    });

    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    let results = [data(&first), data(&second)];
    assert_eq!(
        results
            .iter()
            .filter(|result| result["changed"] == true)
            .count(),
        1
    );
    let winner = results
        .iter()
        .find(|result| result["changed"] == true)
        .expect("one winner");
    let preserved = results
        .iter()
        .find(|result| result["changed"] == false)
        .expect("one preserved result");
    assert_eq!(preserved["context"], winner["context"]);
    assert!(matches!(
        winner["context"]["summary"].as_str(),
        Some("first contender" | "second contender")
    ));

    let registry = load_coordination_registry(&state_dir);
    let claims = registry["claims"].as_array().expect("claims");
    let active = claims
        .iter()
        .filter(|claim| {
            claim["session_id"] == "alpha"
                && claim["session_incarnation"] == "incarnation-alpha"
                && claim["state"] == "active"
        })
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0]["claim_id"], winner["context"]["claim_id"]);
    assert_eq!(active[0]["revision"], winner["context"]["revision"]);
    assert_eq!(active[0]["summary"], winner["context"]["summary"]);
}

#[test]
fn advisory_lifecycle_skips_stopped_and_off_peers_but_preserves_known_overlap() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "gamma",
                "incarnation-gamma",
                "gamma-private-capability-material",
                checkout.as_path(),
                Some("off"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let alpha_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];

    rewrite_registry(&state_dir, |registry| {
        registry["brokers"]["gamma"]["state"] = json!("starting");
    });
    let initial = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(initial.code, 0, "stderr={}", initial.stderr_text());
    assert_eq!(data(&initial)["available"], true);
    assert_eq!(data(&initial)["severity"], "warning");
    assert_eq!(data(&initial)["peers"].as_array().expect("peers").len(), 1);

    let gamma_record = state_dir.join("sessions/gamma/session.json");
    let mut gamma: serde_json::Value =
        serde_json::from_slice(&fs::read(&gamma_record).expect("gamma record"))
            .expect("gamma json");
    gamma["coordination_mode"] = json!("advisory");
    fs::write(
        &gamma_record,
        serde_json::to_vec_pretty(&gamma).expect("gamma json"),
    )
    .expect("write gamma");
    let mixed = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(mixed.code, 0, "stderr={}", mixed.stderr_text());
    assert_eq!(data(&mixed)["available"], false);
    assert_eq!(data(&mixed)["severity"], "warning");
    assert_eq!(data(&mixed)["reasons"][0]["peer_session_id"], "beta");

    rewrite_registry(&state_dir, |registry| {
        registry["brokers"]["beta"]["state"] = json!("stopped");
        registry["brokers"]["gamma"]["state"] = json!("stopped");
    });
    let stopped = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(stopped.code, 0, "stderr={}", stopped.stderr_text());
    assert_eq!(data(&stopped)["available"], true);
    assert_eq!(data(&stopped)["severity"], "none");
    assert!(
        data(&stopped)["reasons"]
            .as_array()
            .expect("reasons")
            .is_empty()
    );
}

#[test]
fn advisory_commit_preserves_a_replacement_incarnation_observation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let fake_bin = tmp.path().join("fake-bin");
    fs::create_dir(&fake_bin).expect("fake bin");
    let started = tmp.path().join("git-started");
    let release = tmp.path().join("git-release");
    let git = fake_bin.join("git");
    fs::write(
        &git,
        "#!/usr/bin/env bash\nset -euo pipefail\n: >\"$GIT_PROBE_STARTED\"\nwhile [ ! -e \"$GIT_PROBE_RELEASE\" ]; do sleep 0.01; done\nprintf '%s\\n' https://github.com/example/repository.git\n",
    )
    .expect("fake git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).expect("git mode");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("PATH")
    );
    let capability = capability(&state_dir, "alpha");
    let child = Command::new(bin::resolve("agent-session"))
        .current_dir(&checkout)
        .args(["work-context", "advise", "--format", "json"])
        .env("AGENT_SESSION_ID", "alpha")
        .env("AGENT_SESSION_CAPABILITY_FILE", &capability)
        .env("AGENT_SESSION_STATE_DIR", &state_dir)
        .env("GIT_PROBE_STARTED", &started)
        .env("GIT_PROBE_RELEASE", &release)
        .env("PATH", path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn advise");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !started.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        started.exists(),
        "advisory evaluation did not reach git probe"
    );
    let observed_at_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("current time")
        .as_secs();
    rewrite_registry(&state_dir, |registry| {
        registry["brokers"]["alpha"]["incarnation"] = json!("incarnation-replacement");
        registry["advisory_observations"]["alpha"] = json!({
            "session_incarnation": "incarnation-replacement",
            "advisory_digest": "replacement-observation-digest",
            "observed_at_epoch": observed_at_epoch
        });
    });
    fs::write(&release, b"release\n").expect("release git probe");
    let output = child.wait_with_output().expect("wait advise");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let registry: serde_json::Value = serde_json::from_slice(
        &fs::read(state_dir.join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    assert_eq!(
        registry["advisory_observations"]["alpha"]["session_incarnation"],
        "incarnation-replacement"
    );
    assert_eq!(
        registry["advisory_observations"]["alpha"]["advisory_digest"],
        "replacement-observation-digest"
    );
}

#[test]
fn advisory_reuses_checkout_resolution_across_many_peers() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    let owned = (0..12)
        .map(|index| {
            let id = if index == 0 {
                "alpha".to_string()
            } else {
                format!("peer-{index:02}")
            };
            (
                id.clone(),
                format!("incarnation-{id}"),
                format!("{id}-private-capability-material-0123456789"),
            )
        })
        .collect::<Vec<_>>();
    let sessions = owned
        .iter()
        .map(|(id, incarnation, capability)| {
            (
                id.as_str(),
                incarnation.as_str(),
                capability.as_str(),
                checkout.as_path(),
                Some("advisory"),
            )
        })
        .collect::<Vec<_>>();
    seed_brokers_at(&state_dir, &sessions);
    let fake_bin = tmp.path().join("fake-bin");
    fs::create_dir(&fake_bin).expect("fake bin");
    let counter = tmp.path().join("git-probe-count");
    let git = fake_bin.join("git");
    fs::write(
        &git,
        "#!/usr/bin/env bash\nset -euo pipefail\ncount=0\nif [ -f \"$GIT_PROBE_COUNT\" ]; then IFS= read -r count <\"$GIT_PROBE_COUNT\"; fi\nprintf '%s\\n' \"$((count + 1))\" >\"$GIT_PROBE_COUNT\"\nsleep 0.15\nprintf '%s\\n' https://github.com/example/repository.git\n",
    )
    .expect("fake git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).expect("git mode");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("PATH")
    );
    let capability = capability(&state_dir, "alpha");
    let state = state_dir.to_string_lossy();
    let started = std::time::Instant::now();
    let advised = run_with_env(
        &checkout,
        &["work-context", "advise", "--format", "json"],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", capability.as_str()),
            ("AGENT_SESSION_STATE_DIR", state.as_ref()),
            ("GIT_PROBE_COUNT", counter.to_str().expect("counter path")),
            ("PATH", path.as_str()),
        ],
    );
    assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "advisory evaluation took {:?}",
        started.elapsed()
    );
    assert_eq!(
        fs::read_to_string(counter).expect("probe count").trim(),
        "1"
    );
}

#[test]
fn advisory_budget_exhaustion_marks_later_repository_resolution_incomplete() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let alpha_checkout = tmp.path().join("alpha-checkout");
    let slow_checkout = tmp.path().join("peer-a-slow-checkout");
    let overlap_checkout = tmp.path().join("peer-z-overlap-checkout");
    init_checkout(&alpha_checkout, "https://github.com/example/shared.git");
    init_checkout(&slow_checkout, "https://github.com/example/unrelated.git");
    init_checkout(&overlap_checkout, "https://github.com/example/shared.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                alpha_checkout.as_path(),
                Some("advisory"),
            ),
            (
                "peer-a-slow",
                "incarnation-peer-a-slow",
                "peer-a-slow-private-capability-material",
                slow_checkout.as_path(),
                Some("advisory"),
            ),
            (
                "peer-z-overlap",
                "incarnation-peer-z-overlap",
                "peer-z-overlap-private-capability-material",
                overlap_checkout.as_path(),
                Some("advisory"),
            ),
        ],
    );
    let fake_bin = tmp.path().join("fake-bin");
    fs::create_dir(&fake_bin).expect("fake bin");
    let git = fake_bin.join("git");
    fs::write(
        &git,
        "#!/usr/bin/env bash\nset -euo pipefail\ncase \"$2\" in\n  *peer-a-slow-checkout) sleep 1; printf '%s\\n' https://github.com/example/unrelated.git ;;\n  *) printf '%s\\n' https://github.com/example/shared.git ;;\nesac\n",
    )
    .expect("fake git");
    fs::set_permissions(&git, fs::Permissions::from_mode(0o755)).expect("git mode");
    let path = format!(
        "{}:{}",
        fake_bin.display(),
        std::env::var("PATH").expect("PATH")
    );
    let capability = capability(&state_dir, "alpha");
    let state = state_dir.to_string_lossy();
    let advised = run_with_env(
        &alpha_checkout,
        &["work-context", "advise", "--format", "json"],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", capability.as_str()),
            ("AGENT_SESSION_STATE_DIR", state.as_ref()),
            ("PATH", path.as_str()),
        ],
    );
    assert_eq!(advised.code, 0, "stderr={}", advised.stderr_text());
    let body = data(&advised);
    assert_eq!(body["available"], false);
    assert_eq!(body["severity"], "degraded");
}

#[test]
fn acknowledgement_is_bound_to_the_observed_overlap_and_expiry() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                checkout.as_path(),
                Some("advisory"),
            ),
            (
                "gamma",
                "incarnation-gamma",
                "gamma-private-capability-material",
                checkout.as_path(),
                Some("off"),
            ),
        ],
    );
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let alpha_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let advise = || {
        run_with_env(
            &checkout,
            &["work-context", "advise", "--format", "json"],
            &alpha_env,
        )
    };

    let first = advise();
    assert_eq!(data(&first)["suppressed"], false);
    let acknowledged = run_with_env(
        &checkout,
        &[
            "work-context",
            "acknowledge",
            "--for",
            "30m",
            "--format",
            "json",
        ],
        &alpha_env,
    );
    assert_eq!(
        acknowledged.code,
        0,
        "stderr={}",
        acknowledged.stderr_text()
    );
    assert_eq!(data(&advise())["suppressed"], true);

    for target in ["src/one.rs", "src/two.rs"] {
        let path = tmp.path().join(target.replace('/', "-"));
        fs::write(
            &path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.operation-targets.v1",
                "targets": [{
                    "kind": "path-exact",
                    "repository": "example/repository",
                    "value": target
                }],
                "provider_refs": [],
                "checkouts": []
            }))
            .expect("targets json"),
        )
        .expect("write targets");
        let targeted = run_with_env(
            &checkout,
            &[
                "work-context",
                "advise",
                "--targets-file",
                path.to_str().expect("target path"),
                "--format",
                "json",
            ],
            &alpha_env,
        );
        assert_eq!(targeted.code, 0, "stderr={}", targeted.stderr_text());
        assert_eq!(data(&targeted)["suppressed"], true);
    }

    let gamma_record = state_dir.join("sessions/gamma/session.json");
    let mut gamma: serde_json::Value =
        serde_json::from_slice(&fs::read(&gamma_record).expect("gamma record"))
            .expect("gamma json");
    gamma["coordination_mode"] = json!("advisory");
    fs::write(
        &gamma_record,
        serde_json::to_vec_pretty(&gamma).expect("gamma json"),
    )
    .expect("write gamma");
    let changed = advise();
    assert_eq!(data(&changed)["severity"], "warning");
    assert_eq!(data(&changed)["suppressed"], false);
    assert_eq!(data(&changed)["peers"].as_array().expect("peers").len(), 2);

    let acknowledged_again = run_with_env(
        &checkout,
        &["work-context", "acknowledge", "--format", "json"],
        &alpha_env,
    );
    assert_eq!(
        acknowledged_again.code,
        0,
        "stderr={}",
        acknowledged_again.stderr_text()
    );
    assert_eq!(data(&advise())["suppressed"], true);
    rewrite_registry(&state_dir, |registry| {
        registry["advisory_acknowledgements"]["alpha"]["expires_at_epoch"] = json!(0);
    });
    assert_eq!(data(&advise())["suppressed"], false);
}

#[test]
fn raw_claim_and_high_level_set_share_the_checkout_root_fingerprint() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    let checkout = tmp.path().join("checkout");
    let nested = checkout.join("nested");
    init_checkout(&checkout, "https://github.com/example/repository.git");
    fs::create_dir(&nested).expect("nested checkout directory");
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            nested.as_path(),
            Some("enforce"),
        )],
    );
    let context_file = tmp.path().join("context.json");
    candidate(&context_file, "src/", "raw claim");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let raw = run(
        &nested,
        &[
            "--state-dir",
            &state,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            context_file.to_str().expect("context path"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "fingerprint-parity-raw",
            "--format",
            "json",
        ],
    );
    assert_eq!(raw.code, 0, "stderr={}", raw.stderr_text());
    let raw_fingerprint = data(&raw)["context"]["worktrees"][0].clone();
    let alpha_env = [
        ("AGENT_SESSION_ID", "alpha"),
        ("AGENT_SESSION_CAPABILITY_FILE", alpha_cap.as_str()),
        ("AGENT_SESSION_STATE_DIR", state.as_ref()),
    ];
    let declared = run_with_env(
        &nested,
        &[
            "work-context",
            "set",
            "--summary",
            "high-level declaration",
            "--format",
            "json",
        ],
        &alpha_env,
    );
    assert_eq!(declared.code, 0, "stderr={}", declared.stderr_text());
    assert_eq!(data(&declared)["context"]["worktrees"][0], raw_fingerprint);
}

#[test]
fn coordination_public_identifiers_do_not_authorize_a_claim_or_echo_peer_data() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state directory");
    seed_session(&state_dir, "alpha", "incarnation-alpha");
    let candidate = tmp.path().join("candidate.json");
    let private_canary = "PRIVATE-COORDINATION-SUMMARY-CANARY";
    fs::write(
        &candidate,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.work-context-input.v1",
            "intent": "implementation",
            "tier": "program",
            "repositories": ["example/repository"],
            "worktrees": [],
            "provider_refs": [],
            "plan_refs": [],
            "scopes": [{
                "kind": "path-prefix",
                "repository": "example/repository",
                "value": "src/"
            }],
            "summary": private_canary
        }))
        .expect("candidate json"),
    )
    .expect("write candidate");

    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state path"),
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            candidate.to_str().expect("candidate path"),
            "--idempotency-key",
            "claim-without-capability",
            "--format",
            "json",
        ],
    );

    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    assert!(
        output.stdout_json()["error"]["hint"]
            .as_str()
            .is_some_and(|hint| hint.contains("agent-session broker identity")),
        "coordination-unauthorized must carry a remedy hint"
    );
    let combined = format!("{}{}", output.stdout_text(), output.stderr_text());
    assert!(!combined.contains(private_canary), "{combined}");
    assert!(!combined.contains("incarnation-alpha"), "{combined}");
}

#[test]
fn atomic_claim_conflict_idempotency_and_uncovered_mutation_are_fenced() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let alpha_candidate = tmp.path().join("alpha.json");
    let beta_candidate = tmp.path().join("beta.json");
    candidate(&alpha_candidate, "src/", "alpha context");
    candidate(&beta_candidate, "src/", "beta context");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");

    let alpha = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            alpha_candidate.to_str().expect("candidate"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "claim-alpha-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        alpha.code,
        0,
        "stdout={} stderr={}",
        alpha.stdout_text(),
        alpha.stderr_text()
    );
    assert_eq!(
        data(&alpha)["evaluation"]["classification"],
        "unknown",
        "the unclaimed live beta peer prevents clear"
    );
    let alpha_claim = data(&alpha)["context"].clone();

    let retry = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            alpha_candidate.to_str().expect("candidate"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "claim-alpha-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(retry.code, 0, "stderr={}", retry.stderr_text());
    assert_eq!(data(&retry)["context"]["claim_id"], alpha_claim["claim_id"]);

    let beta = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "work-context",
            "claim",
            "--session",
            "beta",
            "--file",
            beta_candidate.to_str().expect("candidate"),
            "--capability-file",
            &beta_cap,
            "--idempotency-key",
            "claim-beta-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(beta.code, 0);
    assert_eq!(beta.stdout_json()["error"]["code"], "claim-conflict");
    assert_eq!(
        beta.stdout_json()["error"]["details"]["evaluation"]["reasons"][0]["code"],
        "overlapping-scope"
    );

    let targets = tmp.path().join("targets.json");
    fs::write(
        &targets,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "path-exact",
                "repository": "example/repository",
                "value": "tests/outside.rs"
            }]
        }))
        .expect("targets"),
    )
    .expect("targets file");
    let execution_token = tmp.path().join("execution-token");
    fs::write(&execution_token, "execution-token-alpha").expect("execution token");
    fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600))
        .expect("execution token mode");
    let uncovered = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "work-context",
            "admit",
            "--session",
            "alpha",
            "--claim",
            alpha_claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            "1",
            "--targets-file",
            targets.to_str().expect("targets"),
            "--operation",
            "edit",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "admit-alpha-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(uncovered.code, 0);
    assert_eq!(
        uncovered.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );
}

#[test]
fn legacy_checkout_shell_grant_cannot_widen_explicit_claim_scopes() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let checkout = tmp.path().join("checkout");
    let other_checkout = tmp.path().join("other-checkout");
    fs::create_dir(&state_dir).expect("state");
    init_checkout(&checkout, "https://example.invalid/example/repository.git");
    init_checkout(
        &other_checkout,
        "https://example.invalid/example/repository.git",
    );
    seed_brokers_at(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
            checkout.as_path(),
            Some("enforce"),
        )],
    );
    seed_activity_state(
        &state_dir,
        "alpha",
        "incarnation-alpha",
        "working",
        json!({
            "provider_turn_id": "turn-checkout-shell",
            "started_at": "2030-01-01T00:00:01Z"
        }),
        serde_json::Value::Null,
    );
    let _runtime = seed_live_runtime_identity(&state_dir, "alpha", "incarnation-alpha", 91);
    let candidate_file = tmp.path().join("candidate.json");
    candidate(&candidate_file, "src/owned/", "checkout shell context");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let claimed = run(
        &checkout,
        &[
            "--state-dir",
            &state,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            candidate_file.to_str().expect("candidate"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "claim-checkout-shell-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        claimed.code,
        0,
        "stdout={} stderr={}",
        claimed.stdout_text(),
        claimed.stderr_text()
    );
    let claim = data(&claimed)["context"].clone();
    assert_eq!(claim["scopes"][0]["kind"], "path-prefix");

    let targets = tmp.path().join("checkout-shell-targets.json");
    fs::write(
        &targets,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "repository",
                "repository": "example/repository",
                "value": "."
            }],
            "provider_refs": [],
            "checkouts": [{
                "repository": "example/repository",
                "path": checkout
            }]
        }))
        .expect("targets"),
    )
    .expect("targets file");
    let execution_token = tmp.path().join("checkout-shell-token");
    fs::write(&execution_token, "execution-token-checkout-shell").expect("execution token");
    fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600))
        .expect("execution token mode");

    let other_targets = tmp.path().join("other-checkout-shell-targets.json");
    fs::write(
        &other_targets,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "repository",
                "repository": "example/repository",
                "value": "."
            }],
            "provider_refs": [],
            "checkouts": [{
                "repository": "example/repository",
                "path": other_checkout
            }]
        }))
        .expect("targets"),
    )
    .expect("targets file");
    let wrong_checkout = run(
        &checkout,
        &[
            "--state-dir",
            &state,
            "work-context",
            "admit",
            "--session",
            "alpha",
            "--claim",
            claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            "1",
            "--targets-file",
            other_targets.to_str().expect("targets"),
            "--operation",
            "shell",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "admit-other-checkout-shell-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(wrong_checkout.code, 0);
    assert_eq!(
        wrong_checkout.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );

    let outside_edit_targets = tmp.path().join("outside-edit-targets.json");
    fs::write(
        &outside_edit_targets,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.operation-targets.v1",
            "targets": [{
                "kind": "path-exact",
                "repository": "example/repository",
                "value": "tests/outside.rs"
            }]
        }))
        .expect("targets"),
    )
    .expect("targets file");
    let outside_edit = run(
        &checkout,
        &[
            "--state-dir",
            &state,
            "work-context",
            "admit",
            "--session",
            "alpha",
            "--claim",
            claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            "1",
            "--targets-file",
            outside_edit_targets.to_str().expect("targets"),
            "--operation",
            "edit",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "admit-outside-edit-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(outside_edit.code, 0);
    assert_eq!(
        outside_edit.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );

    let unassigned_shell = run(
        &checkout,
        &[
            "--state-dir",
            &state,
            "work-context",
            "admit",
            "--session",
            "alpha",
            "--claim",
            claim["claim_id"].as_str().expect("claim id"),
            "--if-revision",
            "1",
            "--targets-file",
            targets.to_str().expect("targets"),
            "--operation",
            "shell",
            "--execution-token-file",
            execution_token.to_str().expect("execution token"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "admit-unassigned-checkout-shell-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(unassigned_shell.code, 0);
    assert_eq!(
        unassigned_shell.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );
    grant_checkout_shell(&state_dir, &["alpha"]);
    let legacy_grant = run(
        &checkout,
        &[
            "--state-dir",
            &state,
            "work-context",
            "admit",
            "--session",
            "alpha",
            "--claim",
            claim["claim_id"].as_str().unwrap(),
            "--if-revision",
            "1",
            "--targets-file",
            targets.to_str().unwrap(),
            "--operation",
            "shell",
            "--execution-token-file",
            execution_token.to_str().unwrap(),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "retired-grant-rejected",
            "--format",
            "json",
        ],
    );
    assert_eq!(legacy_grant.code, 65);
    assert_eq!(
        legacy_grant.stdout_json()["error"]["code"],
        "uncovered-mutation-scope"
    );
    let registry = coordination_registry(&state_dir);
    assert_eq!(
        registry["claims"][0]["checkout_shell_grant"], true,
        "retired metadata survives ordinary writes without admission authority"
    );
}

#[test]
fn explicitly_scoped_operations_in_distinct_worktrees_can_run_concurrently() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let alpha_checkout = tmp.path().join("alpha-checkout");
    let beta_checkout = tmp.path().join("beta-checkout");
    fs::create_dir(&state_dir).expect("state");
    init_checkout(
        &alpha_checkout,
        "https://example.invalid/example/repository.git",
    );
    init_checkout(
        &beta_checkout,
        "https://example.invalid/example/repository.git",
    );
    seed_brokers_at(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
                alpha_checkout.as_path(),
                Some("enforce"),
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
                beta_checkout.as_path(),
                Some("enforce"),
            ),
        ],
    );
    for (id, incarnation, turn) in [
        ("alpha", "incarnation-alpha", "turn-alpha-shell"),
        ("beta", "incarnation-beta", "turn-beta-shell"),
    ] {
        seed_activity_state(
            &state_dir,
            id,
            incarnation,
            "working",
            json!({
                "provider_turn_id": turn,
                "started_at": "2030-01-01T00:00:01Z"
            }),
            serde_json::Value::Null,
        );
    }
    let _alpha_runtime = seed_live_runtime_identity(&state_dir, "alpha", "incarnation-alpha", 92);
    let _beta_runtime = seed_live_runtime_identity(&state_dir, "beta", "incarnation-beta", 93);
    let state = state_dir.to_string_lossy();

    let mut claims = serde_json::Map::new();
    for (id, checkout, prefix) in [
        ("alpha", alpha_checkout.as_path(), "src/alpha/"),
        ("beta", beta_checkout.as_path(), "src/beta/"),
    ] {
        let candidate_file = tmp.path().join(format!("{id}-candidate.json"));
        candidate(&candidate_file, prefix, &format!("{id} checkout shell"));
        let capability_file = capability(&state_dir, id);
        let claimed = run(
            checkout,
            &[
                "--state-dir",
                &state,
                "work-context",
                "claim",
                "--session",
                id,
                "--file",
                candidate_file.to_str().expect("candidate"),
                "--capability-file",
                &capability_file,
                "--idempotency-key",
                &format!("claim-{id}-checkout-shell-0001"),
                "--format",
                "json",
            ],
        );
        assert_eq!(
            claimed.code,
            0,
            "id={id} stdout={} stderr={}",
            claimed.stdout_text(),
            claimed.stderr_text()
        );
        claims.insert(id.to_string(), data(&claimed)["context"].clone());
    }

    let mut admitted_leases = Vec::new();
    for (id, checkout) in [
        ("alpha", alpha_checkout.as_path()),
        ("beta", beta_checkout.as_path()),
    ] {
        let targets_file = tmp.path().join(format!("{id}-shell-targets.json"));
        fs::write(
            &targets_file,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.operation-targets.v1",
                "targets": [{
                    "kind": "path-prefix",
                    "repository": "example/repository",
                    "value": format!("src/{id}")
                }],
                "provider_refs": [],
                "checkouts": [{
                    "repository": "example/repository",
                    "path": checkout
                }]
            }))
            .expect("targets"),
        )
        .expect("targets file");
        let execution_token = tmp.path().join(format!("{id}-shell-token"));
        fs::write(&execution_token, format!("execution-token-{id}-shell"))
            .expect("execution token");
        fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600))
            .expect("execution token mode");
        let capability_file = capability(&state_dir, id);
        let claim = &claims[id];
        let admitted = run(
            checkout,
            &[
                "--state-dir",
                &state,
                "work-context",
                "admit",
                "--session",
                id,
                "--claim",
                claim["claim_id"].as_str().expect("claim id"),
                "--if-revision",
                "1",
                "--targets-file",
                targets_file.to_str().expect("targets"),
                "--operation",
                "shell",
                "--execution-token-file",
                execution_token.to_str().expect("execution token"),
                "--capability-file",
                &capability_file,
                "--idempotency-key",
                &format!("admit-{id}-checkout-shell-0001"),
                "--format",
                "json",
            ],
        );
        assert_eq!(
            admitted.code,
            0,
            "id={id} stdout={} stderr={}",
            admitted.stdout_text(),
            admitted.stderr_text()
        );
        admitted_leases.push(data(&admitted)["lease_id"].clone());
    }
    assert_ne!(admitted_leases[0], admitted_leases[1]);
}

#[test]
fn concurrent_definite_contenders_admit_exactly_one_claim() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let alpha_candidate = tmp.path().join("alpha.json");
    let beta_candidate = tmp.path().join("beta.json");
    candidate(&alpha_candidate, "crates/", "alpha contender");
    candidate(&beta_candidate, "crates/", "beta contender");
    let binary = bin::resolve("agent-session");

    let spawn = |id: &str, file: &Path, key: &str| {
        Command::new(&binary)
            .current_dir(tmp.path())
            .args([
                "--state-dir",
                state_dir.to_str().expect("state"),
                "work-context",
                "claim",
                "--session",
                id,
                "--file",
                file.to_str().expect("candidate"),
                "--capability-file",
                &capability(&state_dir, id),
                "--idempotency-key",
                key,
                "--format",
                "json",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn contender")
    };
    let alpha = spawn("alpha", &alpha_candidate, "race-alpha-0001");
    let beta = spawn("beta", &beta_candidate, "race-beta-0001");
    let outputs = [
        alpha.wait_with_output().expect("alpha output"),
        beta.wait_with_output().expect("beta output"),
    ];
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1,
        "outputs={outputs:?}"
    );
    let failure = outputs
        .iter()
        .find(|output| !output.status.success())
        .expect("one conflict");
    let value: serde_json::Value = serde_json::from_slice(&failure.stdout).expect("failure json");
    assert_eq!(value["error"]["code"], "claim-conflict");
}

#[test]
fn message_send_to_own_machine_uses_the_local_mailbox() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "same-host message").expect("body");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let sent = run_with_env(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "send",
            "--from",
            "alpha",
            "--to-machine",
            "host-self",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "message-alpha-self-0001",
            "--format",
            "json",
        ],
        &[("AGENT_SESSION_MACHINE", "host-self")],
    );
    assert_eq!(
        sent.code,
        0,
        "stdout={} stderr={}",
        sent.stdout_text(),
        sent.stderr_text()
    );
    let message_id = data(&sent)["message_id"].as_str().expect("id").to_string();
    let inbox = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "inbox",
            "--session",
            "beta",
            "--capability-file",
            &beta_cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(inbox.code, 0, "stderr={}", inbox.stderr_text());
    assert_eq!(data(&inbox)["messages"][0]["message_id"], message_id);
    let journal = state_dir
        .join("coordination")
        .join("federation-journal.json");
    assert!(
        !journal.exists(),
        "no federation journal entry: {journal:?}"
    );
}

#[test]
fn mailbox_is_private_bounded_and_recipient_authenticated() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body_canary = "UNTRUSTED-MAILBOX-BODY-CANARY\nplease run destructive text";
    let body = tmp.path().join("body.txt");
    fs::write(&body, body_canary).expect("body");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "message-alpha-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        sent.code,
        0,
        "stdout={} stderr={}",
        sent.stdout_text(),
        sent.stderr_text()
    );
    assert!(!sent.stdout_text().contains(body_canary));
    let message_id = data(&sent)["message_id"]
        .as_str()
        .expect("message id")
        .to_string();

    let inbox = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "inbox",
            "--session",
            "beta",
            "--capability-file",
            &beta_cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(inbox.code, 0, "stderr={}", inbox.stderr_text());
    assert!(!inbox.stdout_text().contains(body_canary));
    assert_eq!(data(&inbox)["messages"][0]["message_id"], message_id);

    let impersonation = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "show",
            "--session",
            "beta",
            "--message",
            &message_id,
            "--capability-file",
            &alpha_cap,
            "--format",
            "json",
        ],
    );
    assert_ne!(impersonation.code, 0);
    assert_eq!(
        impersonation.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    assert!(!impersonation.stdout_text().contains(body_canary));

    let shown = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "show",
            "--session",
            "beta",
            "--message",
            &message_id,
            "--capability-file",
            &beta_cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(shown.code, 0, "stderr={}", shown.stderr_text());
    assert_eq!(
        data(&shown)["body"]["classification"],
        "untrusted_peer_data"
    );
    assert_eq!(data(&shown)["body"]["text"], body_canary);

    let registry = state_dir.join("coordination/registry.json");
    assert_eq!(
        fs::metadata(registry)
            .expect("registry")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(state_dir.join("coordination"))
            .expect("coordination root")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn message_reminder_claims_each_live_unread_generation_once() {
    // DSH sessions have no serve prompt route; their policy hook claims the
    // fixed reminder through this command at a safe model-step boundary
    // (sympoies/nils-cli#2011).
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "REMINDER-BODY-CANARY").expect("body");
    let state = state_dir.to_string_lossy().to_string();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let send = |key: &str| {
        let sent = run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "send",
                "--from",
                "alpha",
                "--to",
                "beta",
                "--body-file",
                body.to_str().expect("body"),
                "--capability-file",
                &alpha_cap,
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        );
        assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    };
    let reminder = |session: &str, capability: &str| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "reminder",
                "--session",
                session,
                "--capability-file",
                capability,
                "--format",
                "json",
            ],
        )
    };

    // Nothing is pending before any mail arrives.
    let empty = reminder("beta", &beta_cap);
    assert_eq!(
        empty.code,
        0,
        "stdout={} stderr={}",
        empty.stdout_text(),
        empty.stderr_text()
    );
    assert_eq!(
        empty.stdout_json()["schema_version"],
        "cli.agent-session.message-reminder.v1"
    );
    assert_eq!(data(&empty)["reminder"], serde_json::Value::Null);

    send("reminder-send-0001");
    let first = reminder("beta", &beta_cap);
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    let text = data(&first)["reminder"]
        .as_str()
        .expect("reminder text")
        .to_string();
    assert!(
        text.starts_with("Coordination mailbox has unread messages (newest queued "),
        "{text}"
    );
    assert!(text.contains("agent-session message inbox --session beta --state unread"));
    assert!(
        text.contains("If you already read the inbox after that time, nothing new is waiting.")
    );
    assert!(!first.stdout_text().contains("REMINDER-BODY-CANARY"));
    assert_eq!(data(&first)["generation"], 1);
    let receipt = coordination_registry(&state_dir)["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .find(|receipt| receipt["target_session_id"] == "beta")
        .expect("beta receipt")
        .clone();
    assert_eq!(receipt["state"], "prompt_submitted");
    assert_eq!(receipt["notified_generation"], 1);

    // One generation is delivered once.
    let repeat = reminder("beta", &beta_cap);
    assert_eq!(repeat.code, 0, "stderr={}", repeat.stderr_text());
    assert_eq!(data(&repeat)["reminder"], serde_json::Value::Null);

    // A later send advances the generation, including after serve recorded
    // that the hook owns delivery.
    rewrite_registry(&state_dir, |registry| {
        for receipt in registry["notifications"]
            .as_object_mut()
            .expect("notifications")
            .values_mut()
        {
            receipt["state"] = json!("undeliverable");
            receipt["last_reason"] = json!("hook-delivered");
        }
    });
    send("reminder-send-0002");
    rewrite_registry(&state_dir, |registry| {
        for receipt in registry["notifications"]
            .as_object_mut()
            .expect("notifications")
            .values_mut()
        {
            receipt["state"] = json!("undeliverable");
            receipt["last_reason"] = json!("hook-delivered");
        }
    });
    let second = reminder("beta", &beta_cap);
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    assert_eq!(data(&second)["generation"], 2);
    assert!(data(&second)["reminder"].is_string());

    // A drained inbox has nothing to announce, even for a new generation.
    send("reminder-send-0003");
    rewrite_registry(&state_dir, |registry| {
        for message in registry["messages"].as_array_mut().expect("messages") {
            message["state"] = json!("acknowledged");
        }
    });
    let drained = reminder("beta", &beta_cap);
    assert_eq!(drained.code, 0, "stderr={}", drained.stderr_text());
    assert_eq!(data(&drained)["reminder"], serde_json::Value::Null);

    // Only the authenticated recipient may claim its reminder.
    send("reminder-send-0004");
    let impersonation = reminder("beta", &alpha_cap);
    assert_ne!(impersonation.code, 0);
    assert_eq!(
        impersonation.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    let claimed = reminder("beta", &beta_cap);
    assert!(data(&claimed)["reminder"].is_string());
}

#[test]
fn message_reminder_gives_up_under_lock_contention_without_claiming() {
    // The hook that runs `message reminder` has a 5 s child deadline. The
    // claim must finish well inside it or give up unclaimed, so a reminder is
    // never persisted as delivered by a child the hook already abandoned.
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "contended reminder body").expect("body");
    let state = state_dir.to_string_lossy().to_string();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &alpha_cap,
            "--idempotency-key",
            "contended-reminder-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    let reminder = || {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "reminder",
                "--session",
                "beta",
                "--capability-file",
                &beta_cap,
                "--format",
                "json",
            ],
        )
    };

    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(state_dir.join("coordination/registry.lock"))
        .expect("registry lock");
    lock.lock().expect("hold the registry lock");
    let started = Instant::now();
    let contended = reminder();
    let elapsed = started.elapsed();
    lock.unlock().expect("release the registry lock");
    assert_ne!(contended.code, 0, "stdout={}", contended.stdout_text());
    assert_eq!(
        contended.stdout_json()["error"]["code"],
        "coordination-lock-timeout"
    );
    assert!(
        elapsed < Duration::from_millis(1_800),
        "the reminder claim must give up well inside the hook deadline: {elapsed:?}"
    );
    let receipt = coordination_registry(&state_dir)["notifications"]
        .as_object()
        .expect("notifications")
        .values()
        .find(|receipt| receipt["target_session_id"] == "beta")
        .expect("beta receipt")
        .clone();
    assert_eq!(receipt["notified_generation"], 0);

    // The generation stays claimable once the lock is free.
    let claimed = reminder();
    assert_eq!(claimed.code, 0, "stderr={}", claimed.stderr_text());
    assert_eq!(data(&claimed)["generation"], 1);
    assert!(data(&claimed)["reminder"].is_string());
}

#[test]
fn cli_send_and_reply_share_recipient_generation_scheduling() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let first_body = tmp.path().join("first.txt");
    let second_body = tmp.path().join("second.txt");
    let reply_body = tmp.path().join("reply.txt");
    fs::write(&first_body, "first private body").expect("first body");
    fs::write(&second_body, "second private body").expect("second body");
    fs::write(&reply_body, "private reply").expect("reply body");
    let state = state_dir.to_string_lossy();
    let alpha_cap = capability(&state_dir, "alpha");
    let beta_cap = capability(&state_dir, "beta");

    let send = |body: &Path, key: &str| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                &state,
                "message",
                "send",
                "--from",
                "alpha",
                "--to",
                "beta",
                "--body-file",
                body.to_str().expect("body"),
                "--capability-file",
                &alpha_cap,
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        )
    };
    let first = send(&first_body, "generation-send-0001");
    assert_eq!(first.code, 0, "stderr={}", first.stderr_text());
    assert_eq!(data(&first)["notification"]["state"], "queued");
    assert_eq!(data(&first)["notification"]["generation"], 1);
    assert_eq!(data(&first)["notification"]["notified_generation"], 0);
    assert_eq!(
        data(&first)["notification"]["last_reason"],
        "notification-pending"
    );
    assert_eq!(data(&first)["notification"]["controller_available"], false);
    assert!(!first.stdout_text().contains("first private body"));
    let message_id = data(&first)["message_id"]
        .as_str()
        .expect("message id")
        .to_string();
    let replay = send(&first_body, "generation-send-0001");
    assert_eq!(replay.code, 0, "stderr={}", replay.stderr_text());
    assert_eq!(data(&replay)["message_id"], message_id);
    assert_eq!(data(&replay)["notification"]["generation"], 1);
    let second = send(&second_body, "generation-send-0002");
    assert_eq!(second.code, 0, "stderr={}", second.stderr_text());
    assert_eq!(data(&second)["notification"]["generation"], 2);

    let reply = run(
        tmp.path(),
        &[
            "--state-dir",
            &state,
            "message",
            "reply",
            "--session",
            "beta",
            "--message",
            &message_id,
            "--if-revision",
            "1",
            "--body-file",
            reply_body.to_str().expect("reply body"),
            "--capability-file",
            &beta_cap,
            "--idempotency-key",
            "generation-reply-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(reply.code, 0, "stderr={}", reply.stderr_text());
    assert_eq!(data(&reply)["notification"]["state"], "queued");
    assert_eq!(data(&reply)["notification"]["generation"], 1);
    assert_eq!(data(&reply)["notification"]["controller_available"], false);
    assert!(!reply.stdout_text().contains("private reply"));

    let registry: serde_json::Value = serde_json::from_slice(
        &fs::read(state_dir.join("coordination/registry.json")).expect("registry"),
    )
    .expect("registry json");
    let notifications = registry["notifications"]
        .as_object()
        .expect("notifications");
    assert_eq!(notifications.len(), 2);
    let beta = notifications
        .values()
        .find(|receipt| receipt["target_session_id"] == "beta")
        .expect("beta generation");
    assert_eq!(
        beta["schema_version"],
        "agent-session.notification-generation.v1"
    );
    assert_eq!(beta["target_incarnation"], "incarnation-beta");
    assert_eq!(beta["generation"], 2);
    assert_eq!(beta["state"], "queued");
    let alpha = notifications
        .values()
        .find(|receipt| receipt["target_session_id"] == "alpha")
        .expect("alpha generation");
    assert_eq!(alpha["target_incarnation"], "incarnation-alpha");
    assert_eq!(alpha["generation"], 1);
    assert_eq!(alpha["state"], "queued");
}

#[test]
fn coordination_review_envelopes_identify_the_exact_operation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "inbox",
            "--session",
            "alpha",
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert_eq!(
        output.stdout_json()["schema_version"],
        "cli.agent-session.message-inbox.v1"
    );
}

#[test]
fn coordination_review_wait_processes_message_expiry() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "expires shortly").expect("body");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--expires-in",
            "1s",
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "message-expiry-review-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    let message_id = data(&sent)["message_id"]
        .as_str()
        .expect("message id")
        .to_string();
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let waited = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "wait",
            "--session",
            "beta",
            "--message",
            &message_id,
            "--if-revision",
            "1",
            "--timeout",
            "1s",
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--format",
            "json",
        ],
    );
    assert_ne!(waited.code, 0);
    assert_eq!(waited.stdout_json()["error"]["code"], "message-expired");
}

#[test]
fn coordination_review_registry_lock_rejects_symlinks_without_chmod() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let lock_path = state_dir.join("coordination/registry.lock");
    let sentinel = tmp.path().join("sentinel");
    fs::write(&sentinel, "do not touch").expect("sentinel");
    fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o644)).expect("sentinel mode");
    std::os::unix::fs::symlink(&sentinel, &lock_path).expect("lock symlink");

    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "inbox",
            "--session",
            "alpha",
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--format",
            "json",
        ],
    );
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "coordination-store-untrusted"
    );
    assert_eq!(
        fs::metadata(&sentinel)
            .expect("sentinel metadata")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn coordination_review_bound_operation_blocks_claim_release_and_replacement() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let context_file = tmp.path().join("context.json");
    candidate(&context_file, "src/", "claim with active operation");
    let common = state_dir.to_string_lossy();
    let cap = capability(&state_dir, "alpha");
    let claimed = run(
        tmp.path(),
        &[
            "--state-dir",
            &common,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            context_file.to_str().expect("context"),
            "--capability-file",
            &cap,
            "--idempotency-key",
            "review-bound-claim-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(claimed.code, 0, "stderr={}", claimed.stderr_text());
    let claim_id = data(&claimed)["context"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_string();
    let registry_path = state_dir.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).expect("registry")).expect("json");
    registry["operations"]
        .as_array_mut()
        .expect("operations")
        .push(json!({
            "schema_version": "agent-session.operation-lease.v1",
            "lease_id": "review-lease",
            "session_id": "alpha",
            "session_incarnation": "incarnation-alpha",
            "claim_id": claim_id,
            "claim_revision": 1,
            "operation": "edit",
            "targets": [{"kind": "path-exact", "repository": "example/repository", "value": "src/lib.rs"}],
            "state": "active",
            "revision": 1,
            "started_at": "2030-01-01T00:00:00Z",
            "expires_at": "2030-01-08T00:00:00Z",
            "expires_at_epoch": i64::MAX,
            "execution_token_digest": "digest",
            "activity_revision": 1,
            "runtime_identity_digest": "runtime"
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write registry");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");

    let released = run(
        tmp.path(),
        &[
            "--state-dir",
            &common,
            "work-context",
            "release",
            "--session",
            "alpha",
            "--claim",
            &claim_id,
            "--if-revision",
            "1",
            "--capability-file",
            &cap,
            "--idempotency-key",
            "review-bound-release-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(released.code, 0);
    assert_eq!(
        released.stdout_json()["error"]["code"],
        "operation-in-progress"
    );

    let replaced = run(
        tmp.path(),
        &[
            "--state-dir",
            &common,
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            context_file.to_str().expect("context"),
            "--if-revision",
            "1",
            "--capability-file",
            &cap,
            "--idempotency-key",
            "review-bound-replace-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(replaced.code, 0);
    assert_eq!(
        replaced.stdout_json()["error"]["code"],
        "operation-in-progress"
    );
}

#[test]
fn coordination_review_recovery_rejects_a_healthy_exact_broker() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let proof = tmp.path().join("proof.json");
    fs::write(
        &proof,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.coordination-recovery-proof.v1",
            "session_incarnation": "incarnation-alpha",
            "generation": 1
        }))
        .expect("proof json"),
    )
    .expect("proof");
    let alpha_capability = capability(&state_dir, "alpha");
    let recovered = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "broker",
            "adopt",
            "--session",
            "alpha",
            "--capability-file",
            &alpha_capability,
            "--proof-file",
            proof.to_str().expect("proof"),
            "--idempotency-key",
            "review-healthy-recovery-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(recovered.code, 0);
    assert_eq!(
        recovered.stdout_json()["error"]["code"],
        "coordination-broker-not-lost"
    );
}

#[test]
fn broker_recovery_rejects_cross_session_capabilities_without_state_change() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let proof = tmp.path().join("proof.json");
    fs::write(
        &proof,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.coordination-recovery-proof.v1",
            "session_incarnation": "incarnation-alpha",
            "generation": 1
        }))
        .expect("proof json"),
    )
    .expect("proof");
    let before = fs::read(state_dir.join("coordination/registry.json"))
        .expect("coordination registry before unauthorized recovery");
    let beta_capability = capability(&state_dir, "beta");

    for (subcommand, extra) in [
        ("adopt", Vec::<&str>::new()),
        (
            "reconcile",
            vec![
                "--operation",
                "lease-alpha",
                "--if-revision",
                "1",
                "--attest-inactive",
            ],
        ),
    ] {
        let mut args = vec![
            "--state-dir",
            state_dir.to_str().expect("state"),
            "broker",
            subcommand,
            "--session",
            "alpha",
            "--capability-file",
            &beta_capability,
            "--proof-file",
            proof.to_str().expect("proof"),
            "--idempotency-key",
            if subcommand == "adopt" {
                "cross-session-adopt-0001"
            } else {
                "cross-session-reconcile-0001"
            },
        ];
        args.extend(extra);
        args.extend(["--format", "json"]);
        let recovered = run(tmp.path(), &args);
        assert_ne!(recovered.code, 0);
        assert_eq!(
            recovered.stdout_json()["error"]["code"],
            "coordination-unauthorized",
            "cross-session {subcommand} must fail at capability authentication"
        );
        assert_eq!(
            fs::read(state_dir.join("coordination/registry.json"))
                .expect("coordination registry after unauthorized recovery"),
            before,
            "unauthorized {subcommand} must not change broker or operation state"
        );
    }
}

#[test]
fn broker_recovery_rejects_copied_capability_from_replaced_same_id_incarnation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-old",
            "alpha-old-private-capability-material",
        )],
    );
    let copied_old_capability = tmp.path().join("copied-old-capability");
    fs::copy(capability(&state_dir, "alpha"), &copied_old_capability)
        .expect("copy prior incarnation capability");
    fs::set_permissions(&copied_old_capability, fs::Permissions::from_mode(0o600))
        .expect("copied capability mode");
    fs::remove_dir_all(state_dir.join("sessions/alpha"))
        .expect("remove prior incarnation session state");
    fs::remove_dir_all(state_dir.join("coordination"))
        .expect("remove prior incarnation coordination registry");

    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-new",
            "alpha-new-private-capability-material",
        )],
    );
    let proof = tmp.path().join("proof.json");
    fs::write(
        &proof,
        serde_json::to_vec_pretty(&json!({
            "schema_version": "agent-session.coordination-recovery-proof.v1",
            "session_incarnation": "incarnation-new",
            "generation": 1
        }))
        .expect("proof json"),
    )
    .expect("proof");
    let before = fs::read(state_dir.join("coordination/registry.json"))
        .expect("coordination registry before replaced-incarnation recovery");
    let recovered = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "broker",
            "adopt",
            "--session",
            "alpha",
            "--capability-file",
            copied_old_capability.to_str().expect("copied capability"),
            "--proof-file",
            proof.to_str().expect("proof"),
            "--idempotency-key",
            "replaced-incarnation-adopt-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(recovered.code, 0);
    assert_eq!(
        recovered.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    assert_eq!(
        fs::read(state_dir.join("coordination/registry.json"))
            .expect("coordination registry after replaced-incarnation recovery"),
        before,
        "a copied prior-incarnation capability must not mutate recovery state"
    );
}

#[test]
fn coordination_review_target_exit_revokes_copied_capability_without_hiding_public_status() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let live_capability = capability(&state_dir, "alpha");
    let copied_capability = tmp.path().join("copied-capability");
    fs::copy(&live_capability, &copied_capability).expect("copy capability");
    fs::set_permissions(&copied_capability, fs::Permissions::from_mode(0o600))
        .expect("copied capability mode");
    fs::remove_file(&live_capability).expect("simulate target exit revocation");

    let status = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "broker",
            "status",
            "--session",
            "alpha",
            "--capability-file",
            copied_capability.to_str().expect("copied capability"),
            "--format",
            "json",
        ],
    );
    assert_eq!(status.code, 0, "stderr={}", status.stderr_text());
    assert_eq!(data(&status)["capability_available"], false);
}

#[test]
fn authenticated_broker_status_binds_the_exact_session_incarnation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let alpha_capability = capability(&state_dir, "alpha");
    let beta_capability = capability(&state_dir, "beta");

    let status = |capability: &str| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "broker",
                "status",
                "--session",
                "alpha",
                "--capability-file",
                capability,
                "--authenticated",
                "--format",
                "json",
            ],
        )
    };
    let valid = status(&alpha_capability);
    assert_eq!(valid.code, 0, "stderr={}", valid.stderr_text());
    assert_eq!(data(&valid)["session_id"], "alpha");

    let cross_session = status(&beta_capability);
    assert_ne!(cross_session.code, 0);
    assert_eq!(
        cross_session.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
}

#[test]
fn authenticated_forge_identity_projection_is_read_only_and_refuses_cross_session_or_stale_binding()
{
    let tmp = tempfile::TempDir::new().unwrap();
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).unwrap();
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let path = state_dir.join("sessions/alpha/session.json");
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["role"] = json!("reviewer");
    record["lineage"] = json!({
        "schema_version":"agent-session.session-lineage.v1", "parent":null,
        "root":{"machine":"launch-source", "session_id":"alpha", "session_created_at":record["created_at"]},
        "depth":0,"starter":{"kind":"operator", "via":"cli"},
        "forge_context":{"initiator":"operator", "role":"reviewer"}
    });
    write_private_json(&path, &record);
    let registry_path = state_dir.join("coordination/registry.json");
    let registry_before = fs::read(&registry_path).unwrap();
    let record_before = fs::read(&path).unwrap();
    let project = |cap: &str, runtime: &str| {
        run_with_env(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().unwrap(),
                "broker",
                "identity",
                "--session",
                "alpha",
                "--capability-file",
                cap,
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_ID", "alpha"),
                ("AGENT_SESSION_RUNTIME_ID", runtime),
            ],
        )
    };
    let alpha_cap = capability(&state_dir, "alpha");
    let valid = project(&alpha_cap, "incarnation-alpha");
    assert_eq!(valid.code, 0, "{}", valid.stdout_text());
    assert_eq!(data(&valid)["initiator"], "operator");
    assert_eq!(data(&valid)["role"], "reviewer");
    assert_eq!(data(&valid)["session_incarnation"], "incarnation-alpha");
    assert_eq!(fs::read(&registry_path).unwrap(), registry_before);
    assert_eq!(fs::read(&path).unwrap(), record_before);
    let (tmux, tmux_log) = fake_tmux(tmp.path());
    let agent = fake_agent(tmp.path(), "codex");
    let child = run_with_env(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--id",
            "bound-child",
            "--cwd",
            tmp.path().to_str().unwrap(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            agent.to_str().unwrap(),
            "--paste-delay-ms",
            "0",
            "--role",
            "tester",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_ID", "alpha"),
            ("AGENT_SESSION_RUNTIME_ID", "incarnation-alpha"),
            ("AGENT_SESSION_CAPABILITY_FILE", &alpha_cap),
            ("AGENT_SESSION_MACHINE", "launch-source"),
            ("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap()),
        ],
    );
    assert_eq!(child.code, 0, "{}", child.stdout_text());
    let child_record: serde_json::Value = serde_json::from_slice(
        &fs::read(state_dir.join("sessions/bound-child/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        child_record["lineage"]["forge_context"]["initiator"],
        "operator"
    );
    assert_eq!(child_record["lineage"]["forge_context"]["role"], "tester");
    assert_eq!(child_record["lineage"]["parent"]["session_id"], "alpha");
    assert_eq!(child_record["role"], "tester");
    let crossed = project(&capability(&state_dir, "beta"), "incarnation-alpha");
    assert_eq!(
        crossed.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    let stale = project(&alpha_cap, "old-incarnation");
    assert_eq!(
        stale.stdout_json()["error"]["code"],
        "identity_session_binding_mismatch"
    );
    record["lineage"]["forge_context"]["role"] = json!("tester");
    write_private_json(&path, &record);
    assert_eq!(
        project(&alpha_cap, "incarnation-alpha").stdout_json()["error"]["code"],
        "identity_session_binding_invalid"
    );
    record["lineage"]
        .as_object_mut()
        .unwrap()
        .remove("forge_context");
    write_private_json(&path, &record);
    assert_eq!(
        project(&alpha_cap, "incarnation-alpha").stdout_json()["error"]["code"],
        "identity_session_binding_missing"
    );
}

#[test]
fn coordination_review_round2_half_ttl_renew_does_not_self_conflict() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[(
            "alpha",
            "incarnation-alpha",
            "alpha-private-capability-material",
        )],
    );
    let candidate_file = tmp.path().join("candidate.json");
    candidate(&candidate_file, "src/", "renewable claim");
    let claimed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            candidate_file.to_str().expect("candidate"),
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round2-renew-claim-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(claimed.code, 0, "stderr={}", claimed.stderr_text());
    let claim_id = data(&claimed)["context"]["claim_id"]
        .as_str()
        .expect("claim id")
        .to_string();
    let near_expiry = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64
        + 10;
    rewrite_registry(&state_dir, |registry| {
        registry["claims"][0]["expires_at_epoch"] = json!(near_expiry);
    });

    let renewed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "renew",
            "--session",
            "alpha",
            "--claim",
            &claim_id,
            "--if-revision",
            "1",
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round2-renew-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        renewed.code,
        0,
        "stdout={} stderr={}",
        renewed.stdout_text(),
        renewed.stderr_text()
    );
    assert_eq!(data(&renewed)["revision"], 2);
}

#[test]
fn coordination_review_round2_send_rejects_recipient_after_capability_revocation() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    fs::remove_file(capability(&state_dir, "beta")).expect("revoke recipient capability");
    let body = tmp.path().join("body.txt");
    fs::write(&body, "must remain unsent").expect("body");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round2-revoked-recipient-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(sent.code, 0);
    assert_eq!(
        sent.stdout_json()["error"]["code"],
        "coordination-unavailable"
    );
}

#[test]
fn coordination_review_round2_unknown_fingerprint_epoch_is_not_a_definite_conflict() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let fingerprint = format!("hmac-sha256:999:{}", "a".repeat(64));
    let write_candidate = |path: &Path, summary: &str| {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.work-context-input.v1",
                "intent": "implementation",
                "tier": "program",
                "repositories": [],
                "worktrees": [fingerprint],
                "provider_refs": [],
                "plan_refs": [],
                "scopes": [],
                "summary": summary
            }))
            .expect("candidate"),
        )
        .expect("write candidate");
    };
    let alpha_file = tmp.path().join("alpha.json");
    let beta_file = tmp.path().join("beta.json");
    write_candidate(&alpha_file, "alpha unknown epoch");
    write_candidate(&beta_file, "beta unknown epoch");
    for (id, file, key) in [
        ("alpha", &alpha_file, "round2-epoch-alpha-0001"),
        ("beta", &beta_file, "round2-epoch-beta-0001"),
    ] {
        let claimed = run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "work-context",
                "claim",
                "--session",
                id,
                "--file",
                file.to_str().expect("candidate"),
                "--capability-file",
                &capability(&state_dir, id),
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        );
        assert_eq!(
            claimed.code,
            0,
            "id={id} stdout={} stderr={}",
            claimed.stdout_text(),
            claimed.stderr_text()
        );
        assert_ne!(data(&claimed)["evaluation"]["classification"], "conflict");
    }
}

#[test]
fn coordination_review_round2_cli_declares_reply_cas_and_file_backed_execution_tokens() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let reply = run(tmp.path(), &["message", "reply", "--help"]);
    assert_eq!(reply.code, 0);
    assert!(reply.stdout_text().contains("--if-revision"));

    for leaf in ["admit", "complete"] {
        let help = run(tmp.path(), &["work-context", leaf, "--help"]);
        assert_eq!(help.code, 0);
        assert!(help.stdout_text().contains("--execution-token-file"));
        assert!(!help.stdout_text().contains("--execution-token <"));
    }
}

#[test]
fn coordination_review_round2_parse_errors_keep_exact_leaf_envelope_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let output = run(
        tmp.path(),
        &[
            "message",
            "inbox",
            "--format",
            "json",
            "--unknown-review-flag",
        ],
    );
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["schema_version"],
        "cli.agent-session.message-inbox.v1"
    );
}

#[test]
fn federation_parse_errors_keep_exact_leaf_envelope_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    for leaf in ["peers", "delivery"] {
        let output = run(tmp.path(), &["message", leaf, "--format", "json"]);
        assert_eq!(output.code, 64);
        assert_eq!(
            output.stdout_json()["schema_version"],
            format!("cli.agent-session.message-{leaf}.v1")
        );
    }
}

#[test]
fn coordination_review_round2_reply_revalidates_parent_revision_in_final_transaction() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "original").expect("body");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round2-reply-parent-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    let message_id = data(&sent)["message_id"]
        .as_str()
        .expect("message id")
        .to_string();
    let reply_body = tmp.path().join("reply.txt");
    fs::write(&reply_body, "reply").expect("reply");
    let replied = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "reply",
            "--session",
            "beta",
            "--message",
            &message_id,
            "--if-revision",
            "2",
            "--body-file",
            reply_body.to_str().expect("reply"),
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--idempotency-key",
            "round2-reply-cas-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(replied.code, 0);
    assert_eq!(
        replied.stdout_json()["error"]["code"],
        "message-revision-conflict"
    );
}

#[test]
fn coordination_review_round3_frozen_v1_scope_grammar_and_limits_are_exact() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            ("alpha", "inc-alpha", "alpha-private-capability-material"),
            ("beta", "inc-beta", "beta-private-capability-material"),
            ("gamma", "inc-gamma", "gamma-private-capability-material"),
        ],
    );
    let write_context = |path: &Path, repositories: Vec<String>, scopes: serde_json::Value| {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.work-context-input.v1",
                "intent": "implementation",
                "tier": "program",
                "repositories": repositories,
                "worktrees": [],
                "provider_refs": [],
                "plan_refs": [],
                "scopes": scopes,
                "summary": "round three frozen contract"
            }))
            .expect("context json"),
        )
        .expect("write context");
    };
    let capability_scope = tmp.path().join("capability.json");
    write_context(
        &capability_scope,
        vec!["example/repository".to_string()],
        json!([{"kind":"capability","repository":"example/repository","value":"deploy"}]),
    );
    let too_many = tmp.path().join("too-many.json");
    write_context(
        &too_many,
        (0..9)
            .map(|index| format!("example/repository-{index}"))
            .collect(),
        json!([]),
    );
    let glob = tmp.path().join("glob.json");
    write_context(
        &glob,
        vec!["example/repository".to_string()],
        json!([{"kind":"path-exact","repository":"example/repository","value":"src/*.rs"}]),
    );
    let attempt = |session: &str, file: &Path, key: &str| {
        run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "work-context",
                "claim",
                "--session",
                session,
                "--file",
                file.to_str().expect("context"),
                "--capability-file",
                &capability(&state_dir, session),
                "--idempotency-key",
                key,
                "--format",
                "json",
            ],
        )
    };
    let results = [
        attempt("alpha", &capability_scope, "round3-scope-capability-0001"),
        attempt("beta", &too_many, "round3-scope-limits-0001"),
        attempt("gamma", &glob, "round3-scope-glob-0001"),
    ];
    assert!(results.iter().all(|output| output.code != 0));
    assert_eq!(
        results[0].stdout_json()["error"]["code"],
        "invalid-work-context"
    );
    assert_eq!(
        results[1].stdout_json()["error"]["code"],
        "invalid-work-context"
    );
    assert_eq!(results[2].stdout_json()["error"]["code"], "invalid-scope");
}

#[test]
fn coordination_review_round3_public_check_selectors_do_not_suppress_candidates() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[("alpha", "inc-alpha", "alpha-private-capability-material")],
    );
    let context_file = tmp.path().join("context.json");
    candidate(&context_file, "src", "public context");
    let claimed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "claim",
            "--session",
            "alpha",
            "--file",
            context_file.to_str().expect("context"),
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round3-public-claim-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(claimed.code, 0, "stderr={}", claimed.stderr_text());
    let shown = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "show",
            "--session",
            "alpha",
            "--format",
            "json",
        ],
    );
    let candidate_check = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "check",
            "--candidate",
            context_file.to_str().expect("context"),
            "--format",
            "json",
        ],
    );
    let selected_check = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "check",
            "--session",
            "alpha",
            "--format",
            "json",
        ],
    );
    assert_eq!(shown.code, 0, "stderr={}", shown.stderr_text());
    assert_eq!(
        candidate_check.code,
        0,
        "stderr={}",
        candidate_check.stderr_text()
    );
    assert_eq!(
        data(&candidate_check)["classification"],
        "conflict",
        "candidate must compare against every persisted record"
    );
    assert_eq!(
        selected_check.code,
        0,
        "stderr={}",
        selected_check.stderr_text()
    );
    assert_eq!(data(&selected_check)["classification"], "clear");
}

#[test]
fn coordination_review_round3_idempotency_keys_are_principal_scoped() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            ("alpha", "inc-alpha", "alpha-private-capability-material"),
            ("beta", "inc-beta", "beta-private-capability-material"),
        ],
    );
    rewrite_registry(&state_dir, |_| {});
    let beta_record_path = state_dir.join("sessions/beta/session.json");
    let mut beta_record: serde_json::Value =
        serde_json::from_slice(&fs::read(&beta_record_path).expect("beta record")).expect("json");
    beta_record["cwd"] = json!("/fixture/repository-beta");
    fs::write(
        &beta_record_path,
        serde_json::to_vec_pretty(&beta_record).expect("json"),
    )
    .expect("write beta");
    fs::set_permissions(&beta_record_path, fs::Permissions::from_mode(0o600)).expect("mode");
    let write_context = |path: &Path, repository: &str| {
        fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": "agent-session.work-context-input.v1",
                "intent": "implementation",
                "tier": "program",
                "repositories": [repository],
                "worktrees": [],
                "provider_refs": [],
                "plan_refs": [],
                "scopes": [{"kind":"path-prefix","repository":repository,"value":"src"}],
                "summary": repository
            }))
            .expect("context"),
        )
        .expect("write context");
    };
    let alpha_file = tmp.path().join("alpha.json");
    let beta_file = tmp.path().join("beta.json");
    write_context(&alpha_file, "example/alpha");
    write_context(&beta_file, "example/beta");
    for (session, file) in [("alpha", alpha_file), ("beta", beta_file)] {
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "work-context",
                "claim",
                "--session",
                session,
                "--file",
                file.to_str().expect("context"),
                "--capability-file",
                &capability(&state_dir, session),
                "--idempotency-key",
                "round3-shared-idempotency-key",
                "--format",
                "json",
            ],
        );
        assert_eq!(
            output.code,
            0,
            "session={session} stderr={}",
            output.stderr_text()
        );
    }
}

#[test]
fn coordination_review_round3_reply_binding_and_revision_are_in_the_receipt() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            ("alpha", "inc-alpha", "alpha-private-capability-material"),
            ("beta", "inc-beta", "beta-private-capability-material"),
            ("gamma", "inc-gamma", "gamma-private-capability-material"),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "body").expect("body");
    let sent = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round3-parent-send-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(sent.code, 0, "stderr={}", sent.stderr_text());
    let parent = data(&sent)["message_id"].as_str().expect("id").to_string();
    let forged = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "send",
            "--from",
            "beta",
            "--to",
            "gamma",
            "--reply-to",
            &parent,
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--idempotency-key",
            "round3-forged-reply-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(forged.code, 0);
    let first_reply = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "reply",
            "--session",
            "beta",
            "--message",
            &parent,
            "--if-revision",
            "1",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--idempotency-key",
            "round3-reply-cas-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(first_reply.code, 0, "stderr={}", first_reply.stderr_text());
    let changed_revision = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "reply",
            "--session",
            "beta",
            "--message",
            &parent,
            "--if-revision",
            "2",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--idempotency-key",
            "round3-reply-cas-0001",
            "--format",
            "json",
        ],
    );
    assert_ne!(changed_revision.code, 0);
    assert_eq!(
        changed_revision.stdout_json()["error"]["code"],
        "idempotency-key-reused"
    );
    rewrite_registry(&state_dir, |registry| {
        registry["messages"]
            .as_array_mut()
            .expect("messages")
            .retain(|message| message["message_id"] != parent);
    });
    let replayed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "reply",
            "--session",
            "beta",
            "--message",
            &parent,
            "--if-revision",
            "1",
            "--body-file",
            body.to_str().expect("body"),
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--idempotency-key",
            "round3-reply-cas-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(replayed.code, 0, "stderr={}", replayed.stderr_text());
    assert_eq!(
        data(&replayed)["message_id"],
        data(&first_reply)["message_id"]
    );
}

#[test]
fn coordination_review_round3_mailbox_burst_and_cursor_are_bounded() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[
            ("alpha", "inc-alpha", "alpha-private-capability-material"),
            ("beta", "inc-beta", "beta-private-capability-material"),
        ],
    );
    let body = tmp.path().join("body.txt");
    fs::write(&body, "body").expect("body");
    let mut sent_ids = Vec::new();
    let mut eleventh = None;
    for index in 0..11 {
        let key = format!("round3-burst-{index:04}");
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                state_dir.to_str().expect("state"),
                "message",
                "send",
                "--from",
                "alpha",
                "--to",
                "beta",
                "--body-file",
                body.to_str().expect("body"),
                "--capability-file",
                &capability(&state_dir, "alpha"),
                "--idempotency-key",
                &key,
                "--format",
                "json",
            ],
        );
        if index < 10 {
            assert_eq!(
                output.code,
                0,
                "index={index} stderr={}",
                output.stderr_text()
            );
            sent_ids.push(
                data(&output)["message_id"]
                    .as_str()
                    .expect("id")
                    .to_string(),
            );
        } else {
            eleventh = Some(output);
        }
    }
    let eleventh = eleventh.expect("eleventh");
    assert_ne!(eleventh.code, 0);
    assert_eq!(eleventh.stdout_json()["error"]["code"], "rate-limited");
    let inbox = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "message",
            "inbox",
            "--session",
            "beta",
            "--capability-file",
            &capability(&state_dir, "beta"),
            "--limit",
            "1",
            "--format",
            "json",
        ],
    );
    assert_eq!(inbox.code, 0, "stderr={}", inbox.stderr_text());
    let inbox_data = data(&inbox);
    let cursor = inbox_data["next_cursor"].as_str().expect("cursor");
    assert!(!sent_ids.iter().any(|message_id| message_id == cursor));
}

#[test]
fn coordination_review_round4_completion_can_close_an_uncertain_lease() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    fs::create_dir(&state_dir).expect("state");
    seed_brokers(
        &state_dir,
        &[("alpha", "inc-alpha", "alpha-private-capability-material")],
    );
    let execution_token = tmp.path().join("execution-token");
    fs::write(&execution_token, "round-four-execution-token").expect("token");
    fs::set_permissions(&execution_token, fs::Permissions::from_mode(0o600)).expect("token mode");
    let registry_path = state_dir.join("coordination/registry.json");
    let mut registry: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry_path).expect("registry")).expect("json");
    registry["operations"]
        .as_array_mut()
        .expect("operations")
        .push(json!({
            "schema_version": "agent-session.operation-lease.v1",
            "lease_id": "round-four-lease",
            "session_id": "alpha",
            "session_incarnation": "inc-alpha",
            "claim_id": "round-four-claim",
            "claim_revision": 1,
            "operation": "edit",
            "targets": [{"kind":"path-exact","repository":"example/repository","value":"src/lib.rs"}],
            "state": "completing",
            "revision": 2,
            "started_at": "2030-01-01T00:00:00Z",
            "expires_at": "2030-01-01T00:30:00Z",
            "expires_at_epoch": i64::MAX,
            "execution_token_digest": digest("round-four-execution-token"),
            "activity_revision": 1,
            "runtime_identity_digest": "runtime"
        }));
    fs::write(
        &registry_path,
        serde_json::to_vec_pretty(&registry).expect("registry json"),
    )
    .expect("write registry");
    fs::set_permissions(&registry_path, fs::Permissions::from_mode(0o600)).expect("registry mode");

    let completed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_dir.to_str().expect("state"),
            "work-context",
            "complete",
            "--session",
            "alpha",
            "--lease",
            "round-four-lease",
            "--if-revision",
            "2",
            "--execution-token-file",
            execution_token.to_str().expect("token"),
            "--outcome",
            "pass",
            "--capability-file",
            &capability(&state_dir, "alpha"),
            "--idempotency-key",
            "round4-complete-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(completed.code, 0, "stderr={}", completed.stderr_text());
    assert_eq!(data(&completed)["state"], "completed");
}

fn wait_for_descendant_pid(path: &Path, deadline: Instant) -> u32 {
    wait_for_descendant_pid_with(path, deadline, || {})
}

fn wait_for_descendant_pid_with(
    path: &Path,
    deadline: Instant,
    mut incomplete: impl FnMut(),
) -> u32 {
    loop {
        if let Ok(contents) = fs::read_to_string(path)
            && contents.ends_with('\n')
            && let Ok(pid) = contents.trim().parse::<u32>()
            && pid > 0
        {
            return pid;
        }
        assert!(
            Instant::now() < deadline,
            "the provider did not publish a complete descendant identity"
        );
        incomplete();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn descendant_pid_reader_waits_for_complete_publication_and_enforces_deadline() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let pid_file = tmp.path().join("descendant.pid");
    fs::write(&pid_file, "").unwrap();
    let (observed_tx, observed_rx) = std::sync::mpsc::channel();
    let (publish_tx, publish_rx) = std::sync::mpsc::channel();
    let reader_path = pid_file.clone();
    let reader = std::thread::spawn(move || {
        wait_for_descendant_pid_with(
            &reader_path,
            Instant::now() + Duration::from_secs(5),
            || {
                observed_tx.send(()).unwrap();
                publish_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            },
        )
    });
    // Pause publication after truncation and again after a partial numeric
    // write. The reader explicitly acknowledges each incomplete observation.
    observed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    fs::write(&pid_file, "12").unwrap();
    publish_tx.send(()).unwrap();
    observed_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    fs::write(&pid_file, "12345\n").unwrap();
    publish_tx.send(()).unwrap();
    assert_eq!(reader.join().unwrap(), 12345);
    for contents in ["", "12", "invalid\n", "0\n"] {
        fs::write(&pid_file, contents).unwrap();
        assert!(
            std::panic::catch_unwind(|| wait_for_descendant_pid(&pid_file, Instant::now()))
                .is_err()
        );
    }
}

#[test]
fn dsh_tmux_owned_entrypoints_refuse_before_provider_side_effects() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let checkout = tmp.path().join("checkout");
    fs::create_dir(&state_dir).expect("state");
    init_checkout(&checkout, "https://example.invalid/example/repository.git");
    let state_arg = state_dir.to_string_lossy().into_owned();
    let checkout_arg = checkout.to_string_lossy().into_owned();

    for (verb, extra, expected_code) in [
        ("start", Vec::<&str>::new(), "unsupported-start-agent"),
        (
            "run",
            vec!["--prompt", "do not launch"],
            "unsupported-run-agent",
        ),
    ] {
        let mut args = vec![
            "--state-dir",
            state_arg.as_str(),
            verb,
            "--agent",
            "dsh",
            "--cwd",
            checkout_arg.as_str(),
        ];
        args.extend(extra);
        args.extend(["--format", "json"]);
        let refused = run(&checkout, &args);
        assert_eq!(refused.code, 64, "outcome={}", refused.stdout_text());
        assert_eq!(refused.stdout_json()["error"]["code"], expected_code);
    }
    assert!(
        !state_dir.join("sessions").exists()
            || fs::read_dir(state_dir.join("sessions"))
                .expect("sessions dir")
                .next()
                .is_none(),
        "managed start/run refusals must not create session records"
    );

    let provider_marker = tmp.path().join("provider-setup-invoked");
    let fake_agent_hook = tmp.path().join("agent-hook");
    fs::write(
        &fake_agent_hook,
        format!(
            "#!/bin/sh\nprintf invoked > '{}'\nexit 1\n",
            provider_marker.display()
        ),
    )
    .expect("fake agent-hook");
    fs::set_permissions(&fake_agent_hook, fs::Permissions::from_mode(0o700))
        .expect("fake agent-hook mode");
    let fake_agent_hook_arg = fake_agent_hook.to_string_lossy().into_owned();
    let refused_setup = run_with_env(
        &checkout,
        &[
            "--state-dir",
            &state_arg,
            "activity",
            "setup",
            "--agent",
            "dsh",
            "--dry-run",
            "--format",
            "json",
        ],
        &[("AGENT_HOOK_BIN", fake_agent_hook_arg.as_str())],
    );
    assert_eq!(
        refused_setup.code,
        64,
        "outcome={}",
        refused_setup.stdout_text()
    );
    assert_eq!(
        refused_setup.stdout_json()["error"]["code"],
        "unsupported-activity-agent"
    );
    assert!(
        !provider_marker.exists(),
        "the unsupported DSH activity setup must refuse before invoking a provider binary"
    );
}

#[test]
fn message_categories_filter_and_forward_preserve_source_and_independent_ack() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("state");
    fs::create_dir(&root).expect("state");
    seed_brokers(
        &root,
        &[
            (
                "alpha",
                "incarnation-alpha",
                "alpha-private-capability-material",
            ),
            (
                "beta",
                "incarnation-beta",
                "beta-private-capability-material",
            ),
            (
                "gamma",
                "incarnation-gamma",
                "gamma-private-capability-material",
            ),
        ],
    );
    let state = root.to_string_lossy();
    let body = tmp.path().join("body.txt");
    fs::write(&body, "Category forwarding body canary").expect("body");
    let alpha = capability(&root, "alpha");
    let beta = capability(&root, "beta");
    let gamma = capability(&root, "gamma");
    let call = |args: &[&str]| {
        let mut full = vec!["--state-dir", state.as_ref(), "message"];
        full.extend_from_slice(args);
        full.extend(["--format", "json"]);
        run(tmp.path(), &full)
    };
    let send = |key: &str, category: Option<&str>| {
        let mut args = vec![
            "send",
            "--from",
            "alpha",
            "--to",
            "beta",
            "--capability-file",
            &alpha,
            "--body-file",
            body.to_str().unwrap(),
            "--idempotency-key",
            key,
        ];
        if let Some(category) = category {
            args.extend(["--category", category]);
        }
        call(&args)
    };
    let untagged = send("categories-untagged", None);
    assert_eq!(untagged.code, 0, "{}", untagged.stdout_text());
    assert_eq!(data(&untagged)["category"], "uncategorized");
    let first = send("categories-progress", Some("progress"));
    assert_eq!(first.code, 0, "{}", first.stdout_text());
    let handoff = send("categories-handoff", Some("handoff"));
    assert_eq!(handoff.code, 0, "{}", handoff.stdout_text());
    let id = data(&first)["message_id"].as_str().unwrap().to_string();
    let filtered = call(&[
        "inbox",
        "--session",
        "beta",
        "--capability-file",
        &beta,
        "--category",
        "progress",
        "--category",
        "handoff",
        "--limit",
        "1",
    ]);
    assert_eq!(filtered.code, 0, "{}", filtered.stdout_text());
    let page = data(&filtered);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert!(!filtered.stdout_text().contains("body canary"));
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = call(&[
        "inbox",
        "--session",
        "beta",
        "--capability-file",
        &beta,
        "--category",
        "handoff",
        "--category",
        "progress",
        "--cursor",
        cursor,
    ]);
    assert_eq!(next.code, 0, "{}", next.stdout_text());
    assert_eq!(data(&next)["messages"].as_array().unwrap().len(), 1);
    let wrong_cursor = call(&[
        "inbox",
        "--session",
        "beta",
        "--capability-file",
        &beta,
        "--category",
        "progress",
        "--cursor",
        cursor,
    ]);
    assert_eq!(
        wrong_cursor.stdout_json()["error"]["code"],
        "cursor-invalid"
    );
    let changed = send("categories-progress", Some("report"));
    assert_eq!(
        changed.stdout_json()["error"]["code"],
        "idempotency-key-reused"
    );
    let forward = |cap: &str, revision: &str, category: &str, key: &str| {
        call(&[
            "forward",
            "--session",
            "beta",
            "--message",
            &id,
            "--if-revision",
            revision,
            "--to",
            "gamma",
            "--capability-file",
            cap,
            "--category",
            category,
            "--idempotency-key",
            key,
        ])
    };
    let unauthorized = forward(&alpha, "1", "progress", "categories-unauthorized");
    assert_eq!(
        unauthorized.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
    let wrong_revision = forward(&beta, "2", "progress", "categories-stale");
    assert_eq!(
        wrong_revision.stdout_json()["error"]["code"],
        "message-revision-conflict"
    );
    let wrong_category = forward(&beta, "1", "report", "categories-wrong-category");
    assert_eq!(
        wrong_category.stdout_json()["error"]["code"],
        "message-category-conflict"
    );
    let forwarded = forward(&beta, "1", "progress", "categories-forward");
    assert_eq!(forwarded.code, 0, "{}", forwarded.stdout_text());
    let copy = data(&forwarded)["message_id"].as_str().unwrap().to_string();
    assert_ne!(copy, id);
    assert_eq!(data(&forwarded)["category"], "progress");
    assert_eq!(data(&forwarded)["sender"]["session_id"], "beta");
    assert_eq!(
        data(&forwarded)["forwarding"]["original_sender"]["session_id"],
        "alpha"
    );
    assert_eq!(data(&forwarded)["forwarding"]["original_message_id"], id);
    let original = call(&[
        "inbox",
        "--session",
        "beta",
        "--capability-file",
        &beta,
        "--category",
        "progress",
    ]);
    assert_eq!(data(&original)["messages"][0]["state"], "unread");
    assert_eq!(data(&original)["messages"][0]["revision"], 1);
    let shown = call(&[
        "show",
        "--session",
        "gamma",
        "--message",
        &copy,
        "--capability-file",
        &gamma,
    ]);
    assert_eq!(shown.code, 0, "{}", shown.stdout_text());
    assert_eq!(
        data(&shown)["body"]["text"],
        "Category forwarding body canary"
    );
    assert_eq!(
        data(&shown)["body"]["classification"],
        "untrusted_peer_data"
    );
    let looped = call(&[
        "forward",
        "--session",
        "gamma",
        "--message",
        &copy,
        "--if-revision",
        "2",
        "--to",
        "alpha",
        "--capability-file",
        &gamma,
        "--idempotency-key",
        "categories-loop",
    ]);
    assert_eq!(
        looped.stdout_json()["error"]["code"],
        "message-forward-loop"
    );
    let copy_ack = call(&[
        "ack",
        "--session",
        "gamma",
        "--message",
        &copy,
        "--if-revision",
        "2",
        "--capability-file",
        &gamma,
        "--idempotency-key",
        "categories-copy-ack",
    ]);
    assert_eq!(copy_ack.code, 0, "{}", copy_ack.stdout_text());
    let source_ack = call(&[
        "ack",
        "--session",
        "beta",
        "--message",
        &id,
        "--if-revision",
        "1",
        "--capability-file",
        &beta,
        "--idempotency-key",
        "categories-source-ack",
    ]);
    assert_eq!(source_ack.code, 0, "{}", source_ack.stdout_text());
    let replay = forward(&beta, "1", "progress", "categories-forward");
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    assert_eq!(data(&replay)["message_id"], copy);
}

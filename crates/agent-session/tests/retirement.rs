use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::fs;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

fn private_json(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn seed(
    dir: &Path,
    pid: i32,
    group: i32,
    age: i64,
) -> (agent_session::CliContext, PathBuf, PathBuf) {
    let context = agent_session::CliContext {
        state_dir: dir.to_path_buf(),
        host: None,
    };
    let mut identity = json!({"launch_id":"prior-runtime", "session_id":"$7", "pane_id":"%7", "pane_pid":pid, "process_group_id":group});
    #[cfg(target_os = "linux")]
    {
        let ns = fs::metadata("/proc/self/ns/pid").unwrap();
        identity["pid_namespace"] = json!({"device":ns.dev(),"inode":ns.ino(),"boot_id":fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()});
    }
    #[cfg(target_os = "macos")]
    {
        let boot = std::process::Command::new("/usr/sbin/sysctl")
            .args(["-n", "kern.bootsessionuuid"])
            .output()
            .unwrap();
        assert!(boot.status.success());
        identity["macos_boot_id"] = json!(
            String::from_utf8(boot.stdout)
                .unwrap()
                .trim()
                .to_ascii_lowercase()
        );
    }
    let record = json!({"schema_version":"agent-session.session.v1", "id":"recoverable", "agent":"claude", "mode":"interactive", "title":null, "cwd":dir, "tmux_session":"hs-claude-recoverable", "prompt_file":null,"log_file":null,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z", "runtime":{"kind":"tmux","tmux_session":"hs-claude-recoverable","generation":1,"started_at":"2026-01-01T00:00:00Z","launch_id":"prior-runtime"},"provider_resume":{"provider":"claude","session_id":"conversation-id","captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":["--resume","conversation-id"]},"delete_tmux_identity":identity});
    private_json(&dir.join("sessions/recoverable/session.json"), &record);
    let session = agent_session::internal::load_session_record(&context, "recoverable").unwrap();
    let evidence = agent_session::coordination_runtime_evidence(&context, &session).unwrap();
    let epoch = jiff::Timestamp::now().as_second() - age;
    private_json(
        &dir.join("coordination/registry.json"),
        &json!({"schema_version":"agent-session.coordination-registry.v1","brokers":{"recoverable":{"session_id":"recoverable","incarnation":"prior-runtime","generation":1,"coordination_mode":"advisory","capability_digest":agent_session::coordination::digest_bytes(b"fixture-capability"),"state":"ready","heartbeat_at":"2026-01-01T00:00:00Z","heartbeat_epoch":epoch,"runtime_identity":identity,"runtime_identity_digest":evidence.identity_digest}}}),
    );
    let heartbeat = nils_common::coordination_projection::heartbeat_path(dir, "recoverable");
    fs::create_dir_all(heartbeat.parent().unwrap()).unwrap();
    fs::write(&heartbeat, format!("prior-runtime:{epoch}\n")).unwrap();
    fs::set_permissions(&heartbeat, fs::Permissions::from_mode(0o600)).unwrap();
    let capability =
        agent_session::coordination::capability_path(&context, "recoverable", "prior-runtime");
    fs::create_dir_all(capability.parent().unwrap()).unwrap();
    fs::write(&capability, "fixture-capability").unwrap();
    fs::set_permissions(&capability, fs::Permissions::from_mode(0o600)).unwrap();
    let tmux = dir.join("tmux-absent");
    fs::write(
        &tmux,
        "#!/bin/sh\nprintf 'no server running on fixture socket\\n' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    (context, tmux, capability)
}

fn retire(context: &agent_session::CliContext, tmux: &Path, extra: &[&str]) -> CmdOutput {
    let mut args = vec![
        "--state-dir",
        context.state_dir.to_str().unwrap(),
        "--host",
        "test-host",
        "broker",
        "retire-stopped",
        "--session",
        "recoverable",
        "--incarnation",
        "prior-runtime",
        "--generation",
        "1",
        "--idempotency-key",
        "retirement-test-key",
        "--tmux-bin",
        tmux.to_str().unwrap(),
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    run_resolved("agent-session", &args, &CmdOptions::new())
}

#[test]
fn retirement_preview_is_read_only_and_apply_replays_one_atomic_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let registry_path = dir.path().join("coordination/registry.json");
    let mut initial: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    initial["claims"] = json!([{"schema_version":"agent-session.work-context.v1","session_id":"recoverable","session_incarnation":"prior-runtime","claim_id":"retained-claim","revision":1,"state":"active","intent":"project-dev","tier":"direct","repositories":[],"worktrees":[],"provider_refs":[],"plan_refs":[],"scopes":[],"summary":"fixture","updated_at":"2026-01-01T00:00:00Z","expires_at":"2026-01-01T00:00:00Z","expires_at_epoch":1}]);
    private_json(&registry_path, &initial);
    let session = fs::read(dir.path().join("sessions/recoverable/session.json")).unwrap();
    let registry = fs::read(dir.path().join("coordination/registry.json")).unwrap();
    let preview = retire(&context, &tmux, &[]);
    assert_eq!(preview.code, 0, "{}", preview.stderr_text());
    assert_eq!(preview.stdout_json()["data"]["eligible"], true);
    assert_eq!(preview.stdout_json()["data"]["stale_after_seconds"], 600);
    assert!(
        preview.stdout_json()["data"]["proofs"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["passed"] == true)
    );
    assert_eq!(
        fs::read(dir.path().join("coordination/registry.json")).unwrap(),
        registry
    );
    assert!(capability.exists());
    let applied = retire(&context, &tmux, &["--apply"]);
    assert_eq!(applied.code, 0, "{}", applied.stdout_text());
    assert!(!capability.exists());
    let replay = retire(&context, &tmux, &["--apply"]);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    assert_eq!(replay.stdout_json()["data"], applied.stdout_json()["data"]);
    assert_eq!(
        fs::read(dir.path().join("sessions/recoverable/session.json")).unwrap(),
        session
    );
    let state: Value =
        serde_json::from_slice(&fs::read(dir.path().join("coordination/registry.json")).unwrap())
            .unwrap();
    assert_eq!(state["brokers"]["recoverable"]["state"], "stopped");
    assert_eq!(state["brokers"]["recoverable"]["capability_digest"], "");
    assert_eq!(state["receipts"].as_object().unwrap().len(), 1);
    assert_eq!(state["claims"][0]["state"], "released");
    assert_eq!(state["claims"][0]["revision"], 2);
    let changed_request = retire(&context, &tmux, &["--apply", "--stale-after", "601"]);
    assert_eq!(
        changed_request.stdout_json()["error"]["code"],
        "idempotency-key-reused"
    );
}

#[test]
fn retirement_refuses_live_tmux_and_untrusted_or_future_heartbeat() {
    for heartbeat_body in [
        "prior-runtime:99999999999\n",
        "prior-runtime:0\n",
        "prior-runtime:-1\n",
        "malformed\n",
        "other-runtime:1\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
        let path = nils_common::coordination_projection::heartbeat_path(dir.path(), "recoverable");
        fs::write(path, heartbeat_body).unwrap();
        let output = retire(&context, &tmux, &["--apply"]);
        assert_eq!(
            output.stdout_json()["error"]["details"]["proof_step"],
            "heartbeat-stale"
        );
        assert!(capability.exists());
    }
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    fs::write(&tmux, "#!/bin/sh\nexit 0\n").unwrap();
    let output = retire(&context, &tmux, &["--apply"]);
    assert_eq!(
        output.stdout_json()["error"]["details"]["proof_step"],
        "tmux-target-absent"
    );
    assert!(capability.exists());
}

#[test]
fn retirement_refuses_fresh_live_reused_and_replaced_identities_with_proof_items() {
    for (pid, group, age, failed_step) in [
        (i32::MAX, i32::MAX, 0, "heartbeat-stale"),
        (
            std::process::id() as i32,
            i32::MAX,
            1200,
            "pane-process-absent",
        ),
        (
            i32::MAX,
            unsafe { libc::getpgrp() },
            1200,
            "process-group-absent",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (context, tmux, capability) = seed(dir.path(), pid, group, age);
        let preview = retire(&context, &tmux, &[]);
        assert_eq!(preview.code, 0, "{}", preview.stdout_text());
        let data = preview.stdout_json()["data"].clone();
        assert_eq!(data["eligible"], false);
        assert!(
            data["proofs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["step"] == failed_step && p["passed"] == false),
            "{data}"
        );
        let applied = retire(&context, &tmux, &["--apply"]);
        assert_ne!(applied.code, 0);
        assert_eq!(
            applied.stdout_json()["error"]["code"],
            "coordination-retirement-refused"
        );
        assert!(capability.exists());
    }
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let path = dir.path().join("sessions/recoverable/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["generation"] = json!(2);
    private_json(&path, &record);
    let applied = retire(&context, &tmux, &["--apply"]);
    assert_ne!(applied.code, 0);
    assert_eq!(
        applied.stdout_json()["error"]["code"],
        "session-incarnation-conflict"
    );
    assert!(capability.exists());
}

#[test]
fn retirement_rejects_unknown_boot_and_unresolved_expired_operations() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let path = dir.path().join("sessions/recoverable/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    #[cfg(target_os = "linux")]
    {
        record["delete_tmux_identity"]
            .as_object_mut()
            .unwrap()
            .remove("pid_namespace");
    }
    #[cfg(target_os = "macos")]
    {
        record["delete_tmux_identity"]
            .as_object_mut()
            .unwrap()
            .remove("macos_boot_id");
    }
    private_json(&path, &record);
    let preview = retire(&context, &tmux, &[]).stdout_json();
    assert_eq!(preview["data"]["eligible"], false);
    assert!(
        preview["data"]["proofs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["step"] == "same-boot" && p["passed"] == false)
    );
    assert!(capability.exists());

    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let registry_path = dir.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).unwrap()).unwrap();
    registry["operations"] = json!([{"schema_version":"agent-session.operation-lease.v1","lease_id":"retained-operation","session_id":"recoverable","session_incarnation":"prior-runtime","claim_id":"claim","claim_revision":1,"operation":"fixture","targets":[],"state":"reconcile_pending","revision":1,"started_at":"2026-01-01T00:00:00Z","expires_at":"2026-01-01T00:00:00Z","expires_at_epoch":1,"execution_token_digest":"fixture","outcome":null}]);
    private_json(&registry_path, &registry);
    let before = fs::read(&registry_path).unwrap();
    let applied = retire(&context, &tmux, &["--apply"]);
    assert_ne!(applied.code, 0);
    assert_eq!(
        applied.stdout_json()["error"]["details"]["proof_step"],
        "operations-quiescent"
    );
    assert_eq!(fs::read(registry_path).unwrap(), before);
    assert!(capability.exists());
}

#[test]
fn retirement_cleanup_failure_keeps_revocation_and_retries_from_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    fs::remove_file(&capability).unwrap();
    fs::create_dir(&capability).unwrap();
    let failed = retire(&context, &tmux, &["--apply"]);
    assert_ne!(failed.code, 0);
    assert_eq!(
        failed.stdout_json()["error"]["code"],
        "coordination-retirement-cleanup-pending"
    );
    let registry: Value =
        serde_json::from_slice(&fs::read(dir.path().join("coordination/registry.json")).unwrap())
            .unwrap();
    assert_eq!(registry["brokers"]["recoverable"]["capability_digest"], "");
    assert_eq!(registry["receipts"].as_object().unwrap().len(), 1);
    fs::remove_dir(&capability).unwrap();
    let replay = retire(&context, &tmux, &["--apply"]);
    assert_eq!(replay.code, 0, "{}", replay.stdout_text());
    assert_eq!(
        replay.stdout_json()["data"],
        failed.stdout_json()["error"]["details"]["receipt"]
    );
}

#[test]
fn retirement_uses_the_same_lifecycle_lock_as_a_replacement_resume() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let lock =
        agent_session::internal::acquire_session_record_lock(&context, "recoverable").unwrap();
    std::thread::scope(|scope| {
        let context = &context;
        let tmux = &tmux;
        let pending = scope.spawn(move || retire(context, tmux, &["--apply"]));
        // A replacement resume writes its new tuple while holding this lock.
        let mut record =
            agent_session::internal::load_session_record(context, "recoverable").unwrap();
        record.runtime.as_mut().unwrap().generation = 2;
        agent_session::internal::write_session_record(context, &record).unwrap();
        drop(lock);
        let output = pending.join().unwrap();
        assert_ne!(output.code, 0);
        assert!(matches!(
            output.stdout_json()["error"]["code"].as_str(),
            Some("session-incarnation-conflict" | "session-runtime-changed")
        ));
    });
    assert!(capability.exists());
}

#[test]
#[cfg(target_os = "linux")]
fn retirement_unblocks_provisioning_the_next_incarnation() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, _) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    let applied = retire(&context, &tmux, &["--apply"]);
    assert_eq!(applied.code, 0, "{}", applied.stdout_text());
    let mut record = agent_session::internal::load_session_record(&context, "recoverable").unwrap();
    record.runtime.as_mut().unwrap().launch_id = "next-runtime".into();
    record.runtime.as_mut().unwrap().generation = 2;
    // Provision runs the same prior-broker gate that resume uses. It must accept
    // the retired tuple without relaxing the ordinary stopped-proof policy.
    let capability = agent_session::coordination::provision(&context, &record).unwrap();
    assert!(capability.exists());
    assert_eq!(
        serde_json::to_value(&record).unwrap()["provider_resume"]["session_id"],
        "conversation-id"
    );
}

#[test]
fn retirement_refuses_a_live_managed_name_after_tmux_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (context, tmux, capability) = seed(dir.path(), i32::MAX, i32::MAX, 1200);
    fs::write(
        &tmux,
        "#!/bin/sh\ncase \"$*\" in *hs-claude-recoverable*) exit 0;; esac\nprintf 'no server running on fixture socket\\n' >&2\nexit 1\n",
    )
    .unwrap();
    let output = retire(&context, &tmux, &["--apply"]);
    assert_ne!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        output.stdout_json()["error"]["details"]["proof_step"],
        "tmux-target-absent"
    );
    assert!(capability.exists());
}

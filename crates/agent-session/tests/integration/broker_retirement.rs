use std::fs;
use std::os::unix::fs::PermissionsExt;
#[cfg(target_os = "macos")]
use std::process::Command;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use crate::cli::{fake_agent, fake_tmux, run, write_resumable_session_record_with_agent_bin};

#[test]
fn operator_retirement_then_resume_preserves_the_provider_conversation() {
    let tmp = tempfile::tempdir().unwrap();
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let (tmux, tmux_log) = fake_tmux(tmp.path());
    let agent = fake_agent(tmp.path(), "claude");
    let session = write_resumable_session_record_with_agent_bin(
        &state_dir,
        "recoverable",
        "claude",
        "hs-claude-recoverable",
        &cwd,
        &["--resume", "resume-session-id"],
        Some(&agent),
    );
    let path = session.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["runtime"]["launch_id"] = json!("prior-runtime");
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
    let mut identity = json!({"launch_id":"prior-runtime","session_id":"$7","pane_id":"%7","pane_pid":i32::MAX,"process_group_id":i32::MAX});
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::MetadataExt;
        let ns = fs::metadata("/proc/self/ns/pid").unwrap();
        identity["pid_namespace"] = json!({"device":ns.dev(),"inode":ns.ino(),"boot_id":fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim()});
    }
    #[cfg(target_os = "macos")]
    {
        let boot = Command::new("/usr/sbin/sysctl")
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
    record["delete_tmux_identity"] = identity.clone();
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let context = agent_session::CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let persisted = agent_session::load_session_record(&context, "recoverable").unwrap();
    let evidence = agent_session::coordination_runtime_evidence(&context, &persisted).unwrap();
    let registry = state_dir.join("coordination/registry.json");
    fs::create_dir_all(registry.parent().unwrap()).unwrap();
    let epoch = jiff::Timestamp::now().as_second() - 1200;
    fs::write(&registry, serde_json::to_vec(&json!({"schema_version":"agent-session.coordination-registry.v1","brokers":{"recoverable":{"session_id":"recoverable","incarnation":"prior-runtime","generation":1,"state":"ready","capability_digest":"fixture","heartbeat_at":"2026-01-01T00:00:00Z","heartbeat_epoch":epoch,"runtime_identity":identity,"runtime_identity_digest":evidence.identity_digest}}})).unwrap()).unwrap();
    fs::set_permissions(&registry, fs::Permissions::from_mode(0o600)).unwrap();
    let state_arg = state_dir.to_str().unwrap();
    let tmux_arg = tmux.to_str().unwrap();
    let envs = [
        ("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap()),
        ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
    ];
    let retired = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "broker",
            "retire-stopped",
            "--session",
            "recoverable",
            "--incarnation",
            "prior-runtime",
            "--generation",
            "1",
            "--idempotency-key",
            "resume-retirement",
            "--tmux-bin",
            tmux_arg,
            "--apply",
            "--format",
            "json",
        ],
        &envs,
    );
    assert_eq!(retired.code, 0, "{}", retired.stdout_text());
    let resumed = run(
        tmp.path(),
        &[
            "--state-dir",
            state_arg,
            "resume",
            "recoverable",
            "--tmux-bin",
            tmux_arg,
            "--format",
            "json",
        ],
        &envs,
    );
    assert_eq!(resumed.code, 0, "{}", resumed.stdout_text());
    assert_eq!(resumed.stdout_json()["data"]["status"], "running");
    let after: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(after["runtime"]["generation"], 2);
    assert_eq!(
        after["provider_resume"]["session_id"],
        record["provider_resume"]["session_id"]
    );
}

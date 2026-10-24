#![cfg(target_os = "linux")]

use super::*;
use pretty_assertions::{assert_eq, assert_ne};

pub(super) fn run_generation_two_prior_boot_cli_resume_and_delete_ignore_reused_selectors() {
    let tmp_parent = tempfile::tempdir_in("/tmp").expect("short fixture parent");
    let parent_len = tmp_parent.path().as_os_str().len();
    assert!(
        parent_len < 169,
        "fixture parent must leave room for padding"
    );
    let long_tmpdir = tmp_parent.path().join("x".repeat(169 - parent_len));
    fs::create_dir(&long_tmpdir).unwrap();
    assert_eq!(long_tmpdir.as_os_str().len(), 170);
    let tmp = tempfile::TempDir::new_in(&long_tmpdir).unwrap();
    let state = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let (tmux, log) = fake_tmux(tmp.path());
    let agent = fake_agent(tmp.path(), "codex");
    write_executable(
        &agent,
        r#"#!/bin/sh
if [ "$1" = --version ]; then printf '%s\n' 'codex-cli 0.145.0'; exit 0; fi
if [ "$1" = app-server ] && [ "$2" = --help ]; then printf '%s\n' '--listen <URL> unix://'; exit 0; fi
exit 0
"#,
    );
    let runtime_root = tempfile::tempdir_in("/tmp").expect("short socket tempdir");
    fs::set_permissions(runtime_root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let runtime_agent_session = runtime_root.path().join("agent-session");
    fs::create_dir(&runtime_agent_session).unwrap();
    fs::set_permissions(&runtime_agent_session, fs::Permissions::from_mode(0o700)).unwrap();
    let old = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    assert_ne!(
        fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim(),
        old
    );
    for operation in ["resume", "delete"] {
        let id = format!("reboot-{operation}");
        let session = write_resumable_session_record_with_agent_bin(
            &state,
            &id,
            "codex",
            &format!("hs-codex-{id}"),
            &cwd,
            &[
                "resume",
                "resume-session-id",
                "--cd",
                cwd.to_str().unwrap(),
                "--no-alt-screen",
            ],
            Some(&agent),
        );
        let artifacts = attach_provider_runtime(
            tmp.path(),
            &state,
            &session,
            &id,
            "codex",
            &format!("hs-codex-{id}"),
        );
        for artifact in artifacts {
            fs::remove_file(artifact).unwrap();
        }
        fs::set_permissions(&runtime_agent_session, fs::Permissions::from_mode(0o700)).unwrap();
        let path = session.join("session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record
            .as_object_mut()
            .unwrap()
            .remove("tmux_runtime_never_launched");
        record["runtime"]["generation"] = json!(2);
        record["delete_tmux_identity"] = json!({
            "launch_id": record["runtime"]["launch_id"], "session_id": "$7", "pane_id": "%7",
            "pane_pid": 2000000000, "process_group_id": 2000000000,
            "pid_namespace": {"device": 1, "inode": 1, "boot_id": old}
        });
        fs::write(&path, serde_json::to_vec_pretty(&record).unwrap()).unwrap();
        fs::write(&log, b"").unwrap();
        let output = run(
            tmp.path(),
            &[
                "--state-dir",
                state.to_str().unwrap(),
                operation,
                &id,
                "--tmux-bin",
                tmux.to_str().unwrap(),
                "--format",
                "json",
            ],
            &[
                ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
                ("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1"),
                ("XDG_RUNTIME_DIR", runtime_root.path().to_str().unwrap()),
                ("AGENT_SESSION_CODEX_RUNTIME", "app-server"),
            ],
        );
        assert_eq!(
            output.code,
            0,
            "{operation}: {} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        let calls = tmux_calls(&log);
        assert!(!calls.iter().any(|call| {
            call.first().is_some_and(|arg| {
                matches!(arg.as_str(), "kill-session" | "if-shell" | "send-keys")
            })
        }));
        if operation == "delete" {
            assert!(!session.exists());
            assert!(
                calls.is_empty(),
                "prior-boot delete must not probe reused IDs: {calls:?}"
            );
        } else {
            let resumed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(resumed["runtime"]["generation"], 3);
            assert_eq!(resumed["runtime"]["kind"], "codex_app_server");
            assert_eq!(resumed["runtime"]["codex_app_server_protocol"], "v2");
            for key in [
                "codex_app_server_socket",
                "codex_app_server_proxy",
                "codex_app_server_thread_handoff",
                "codex_app_server_thread_attached",
            ] {
                assert!(Path::new(resumed["runtime"][key].as_str().unwrap()).is_absolute());
                assert_ne!(resumed["runtime"][key], record["runtime"][key]);
            }
            assert_ne!(
                resumed["runtime"]["launch_id"],
                record["runtime"]["launch_id"]
            );
            assert_eq!(resumed["provider_resume"], record["provider_resume"]);
        }
    }
}

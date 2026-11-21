#![cfg(target_os = "linux")]

use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn fixture_path(root: &Path) -> std::ffi::OsString {
    let bin = root.join("default-tmux-canary-bin");
    fs::create_dir_all(&bin).unwrap();
    let canary = bin.join("tmux");
    if !canary.exists() {
        let marker = root.join("default-tmux-invoked");
        write_executable(
            &canary,
            &format!(
                "#!/bin/sh\nprintf invoked > {}\nexit 125\n",
                shell_words::quote(&marker.to_string_lossy())
            ),
        );
    }
    std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    )))
    .unwrap()
}

fn assert_fixture_environment(child: &Child, root: &Path, tmux: &Path) {
    // The fixture-owned serve lock names the child only after exec and state
    // root selection. Do not inspect a pre-exec fork's ambient environment.
    let owner = root.join("state/serve.lock");
    let prefix = format!("pid {} on ", child.id());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fs::read_to_string(&owner).is_ok_and(|value| value.starts_with(&prefix)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture daemon did not acquire its private state root"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let expected = fs::canonicalize(nils_test_support::bin::resolve("agent-session")).unwrap();
    assert_eq!(
        fs::read_link(format!("/proc/{}/exe", child.id())).unwrap(),
        expected
    );
    let bytes = fs::read(format!("/proc/{}/environ", child.id())).unwrap();
    let entries: Vec<_> = bytes.split(|byte| *byte == 0).collect();
    let value = |key: &str| {
        let prefix = format!("{key}=");
        let bytes = entries
            .iter()
            .find_map(|entry| entry.strip_prefix(prefix.as_bytes()))
            .unwrap_or_else(|| panic!("missing fixture variable {key}"));
        std::str::from_utf8(bytes).unwrap().to_owned()
    };
    for key in [
        "HOME",
        "AGENT_HOME",
        "XDG_STATE_HOME",
        "XDG_CONFIG_HOME",
        "CODEX_HOME",
        "CLAUDE_CONFIG_DIR",
        "TMPDIR",
    ] {
        assert!(
            Path::new(&value(key)).starts_with(root),
            "fixture root escaped for {key}"
        );
    }
    assert_eq!(Path::new(&value("AGENT_SESSION_TMUX_BIN")), tmux);
}

// All subprocesses in this fixture must route even their internal tmp sweeps
// to the fixture's tmux executable, never to the ambient default server.
fn run(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> nils_test_support::cmd::CmdOutput {
    let private = dir.join("private-tmux");
    let tmux = if private.is_file() {
        private
    } else {
        dir.join("tmux")
    };
    let tmux = tmux.to_str().unwrap();
    let path = fixture_path(dir);
    let path = path.to_str().unwrap();
    let mut isolated = envs.to_vec();
    isolated.push(("PATH", path));
    if !isolated
        .iter()
        .any(|(key, _)| *key == "AGENT_SESSION_TMUX_BIN")
    {
        isolated.push(("AGENT_SESSION_TMUX_BIN", tmux));
    }
    super::run(dir, args, &isolated)
}

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
    assert!(!tmp.path().join("default-tmux-invoked").exists());
}

#[test]
fn serve_recovers_a_recorded_running_session_from_a_prior_boot() {
    let tmp = tempfile::tempdir_in("/tmp").unwrap();
    let state = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let (tmux, log) = fake_tmux(tmp.path());
    let agent = fake_agent(tmp.path(), "claude");
    write_executable(
        &agent,
        "#!/bin/sh\n[ \"$1\" = --version ] && { echo \"9.9.9 (Fixture)\"; exit 0; }\nsleep 60\n",
    );
    let session = write_resumable_session_record_with_agent_bin(
        &state,
        "boot-recovery",
        "claude",
        "hs-claude-boot-recovery",
        &cwd,
        &["--resume", "resume-session-id"],
        Some(&agent),
    );
    let path = session.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
    record["runtime"]["generation"] = json!(2);
    record["startup"]["state"] = json!("ready");
    record["startup"]
        .as_object_mut()
        .unwrap()
        .remove("failure_code");
    record["startup"].as_object_mut().unwrap().remove("message");
    record["startup"]
        .as_object_mut()
        .unwrap()
        .remove("occurred_at");
    record["startup"]
        .as_object_mut()
        .unwrap()
        .remove("retry_safe");
    record["delete_tmux_identity"] = json!({
        "launch_id": record["runtime"]["launch_id"], "session_id": "$7", "pane_id": "%7",
        "pane_pid": 2000000000, "process_group_id": 2000000000,
        "pid_namespace": {"device": 1, "inode": 1, "boot_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}
    });
    record["boot_recovery"] = json!({"shutdown_boot_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "marked_at_epoch": jiff::Timestamp::now().as_second() - 86_000});
    record["updated_at"] = json!(jiff::Timestamp::now().to_string());
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    for (id, intent) in [
        ("boot-stopped", Some("stopped")),
        ("boot-opted-out", Some("running")),
        ("boot-unknown", None),
        ("boot-failed", Some("running")),
        ("boot-no-metadata", Some("running")),
        ("boot-expired", Some("running")),
    ] {
        let mut other = record.clone();
        other["id"] = json!(id);
        other["tmux_session"] = json!(format!("hs-claude-{id}"));
        other["runtime"]["tmux_session"] = other["tmux_session"].clone();
        other["provider_resume"]["session_id"] = json!(format!("conversation-{id}"));
        other["provider_resume"]["resume_args"] = json!(["--resume", format!("conversation-{id}")]);
        if id == "boot-failed" {
            other["provider_resume"]["provider"] = json!("codex");
        }
        if id == "boot-expired" {
            other["boot_recovery"]["marked_at_epoch"] =
                json!(jiff::Timestamp::now().as_second() - 87_000);
        }
        if id == "boot-no-metadata" {
            other["provider_resume"] = Value::Null;
        }
        if let Some(intent) = intent {
            if intent == "stopped" {
                other["boot_recovery"]
                    .as_object_mut()
                    .unwrap()
                    .remove("shutdown_boot_id");
            }
        } else {
            other.as_object_mut().unwrap().remove("boot_recovery");
        }
        let dir = state.join("sessions").join(id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("session.json"),
            serde_json::to_vec(&other).unwrap(),
        )
        .unwrap();
    }
    let current_boot = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap()
        .trim()
        .to_owned();
    let mut same_boot = record.clone();
    same_boot["id"] = json!("boot-same");
    same_boot["tmux_session"] = json!("hs-claude-boot-same");
    same_boot["runtime"]["tmux_session"] = same_boot["tmux_session"].clone();
    same_boot["provider_resume"]["session_id"] = json!("conversation-boot-same");
    same_boot["provider_resume"]["resume_args"] = json!(["--resume", "conversation-boot-same"]);
    same_boot["delete_tmux_identity"]["pid_namespace"]["boot_id"] = json!(current_boot);
    same_boot["boot_recovery"]["shutdown_boot_id"] = json!(current_boot);
    let same_dir = state.join("sessions/boot-same");
    fs::create_dir_all(&same_dir).unwrap();
    fs::write(
        same_dir.join("session.json"),
        serde_json::to_vec(&same_boot).unwrap(),
    )
    .unwrap();
    // A stale directory whose name is a prefix must never resolve to another record.
    fs::create_dir_all(state.join("sessions/boot")).unwrap();
    fs::write(state.join("sessions/unrelated-file"), b"fixture").unwrap();
    let malformed_dir = state.join("sessions/malformed-fixture");
    fs::create_dir_all(&malformed_dir).unwrap();
    fs::write(malformed_dir.join("session.json"), b"{").unwrap();
    let mut expired_same = same_boot.clone();
    expired_same["id"] = json!("boot-same-expired");
    expired_same["tmux_session"] = json!("hs-claude-boot-same-expired");
    expired_same["runtime"]["tmux_session"] = expired_same["tmux_session"].clone();
    expired_same["provider_resume"]["session_id"] = json!("conversation-boot-same-expired");
    expired_same["provider_resume"]["resume_args"] =
        json!(["--resume", "conversation-boot-same-expired"]);
    expired_same["boot_recovery"]["marked_at_epoch"] =
        json!(jiff::Timestamp::now().as_second() - 87_000);
    let expired_same_dir = state.join("sessions/boot-same-expired");
    fs::create_dir_all(&expired_same_dir).unwrap();
    fs::write(
        expired_same_dir.join("session.json"),
        serde_json::to_vec(&expired_same).unwrap(),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    struct OwnedServe(Child);
    impl Drop for OwnedServe {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command.env_clear().env("PATH", fixture_path(tmp.path()));
    let stderr_path = tmp.path().join("serve.stderr");
    command
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "serve",
            "--bind",
            &addr.to_string(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
        ])
        .env("HOME", tmp.path())
        .env("AGENT_HOME", tmp.path().join("agent-home"))
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .env("XDG_STATE_HOME", tmp.path().join("xdg-state"))
        .env("XDG_CONFIG_HOME", tmp.path().join("xdg-config"))
        .env("CODEX_HOME", tmp.path().join("codex-config"))
        .env("CLAUDE_CONFIG_DIR", tmp.path().join("claude-config"))
        .env("TMPDIR", tmp.path())
        .env("AGENT_SESSION_FAKE_TMUX_LOG", &log)
        .env("AGENT_SESSION_BOOT_RECOVERY_EXCLUDE", "boot-opted-out")
        .env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(fs::File::create(&stderr_path).unwrap()));
    let fixture_context = agent_session::CliContext {
        state_dir: state.clone(),
        host: None,
    };
    let held =
        agent_session::acquire_session_record_lock(&fixture_context, "boot-recovery").unwrap();
    let mut child = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&child.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(35);
    while TcpStream::connect(addr).is_err() && Instant::now() < deadline {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "serve exited before listening"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert!(TcpStream::connect(addr).is_ok(), "serve did not listen");
    while !fs::read_to_string(&stderr_path)
        .unwrap_or_default()
        .contains("boot recovery for boot-recovery deferred")
    {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < deadline,
            "recovery did not report lock contention"
        );
        thread::sleep(Duration::from_millis(25));
    }
    while !fs::read_to_string(&stderr_path)
        .unwrap_or_default()
        .contains("boot recovery for malformed-fixture failed")
    {
        assert!(child.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < deadline,
            "recovery did not report the malformed record"
        );
        thread::sleep(Duration::from_millis(25));
    }
    // A retry of the locked record must not revisit permanent failures.
    thread::sleep(Duration::from_secs(6));
    let diagnostics = fs::read_to_string(&stderr_path).unwrap();
    assert_eq!(
        diagnostics
            .matches("boot recovery for malformed-fixture failed")
            .count(),
        1,
        "{diagnostics}"
    );
    fs::remove_file(malformed_dir.join("session.json")).unwrap();
    fs::remove_dir(&malformed_dir).unwrap();
    drop(held);
    while Instant::now() < deadline {
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let complete = ["boot-stopped", "boot-opted-out", "boot-unknown"]
            .iter()
            .all(|id| {
                let path = state.join("sessions").join(id).join("session.json");
                fs::read(path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                    .is_some_and(|value| value["boot_recovery"]["receipt"]["state"] == "skipped")
            });
        let expired_same: Value =
            serde_json::from_slice(&fs::read(expired_same_dir.join("session.json")).unwrap())
                .unwrap();
        let expired: Value = serde_json::from_slice(
            &fs::read(state.join("sessions/boot-expired/session.json")).unwrap(),
        )
        .unwrap();
        let failed: Value = serde_json::from_slice(
            &fs::read(state.join("sessions/boot-failed/session.json")).unwrap(),
        )
        .unwrap();
        let no_metadata: Value = serde_json::from_slice(
            &fs::read(state.join("sessions/boot-no-metadata/session.json")).unwrap(),
        )
        .unwrap();
        if current["boot_recovery"]["receipt"]["state"] == "resumed"
            && complete
            && failed["boot_recovery"]["receipt"]["state"] == "failed"
            && no_metadata["boot_recovery"]["receipt"]["state"] == "skipped"
            && expired["boot_recovery"]["receipt"]["reason"] == "shutdown-intent-expired"
            && expired_same["boot_recovery"]["receipt"]["reason"] == "shutdown-intent-expired"
        {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let after: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        after["boot_recovery"]["receipt"]["state"], "resumed",
        "{after}"
    );
    for (id, reason) in [
        ("boot-stopped", "no-shutdown-intent"),
        ("boot-opted-out", "session-opt-out"),
        ("boot-unknown", "no-shutdown-intent"),
    ] {
        let skipped: Value = serde_json::from_slice(
            &fs::read(state.join("sessions").join(id).join("session.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(skipped["runtime"]["generation"], 2);
        assert_eq!(
            skipped["boot_recovery"]["receipt"],
            json!({"state":"skipped", "reason":reason})
        );
    }
    let failed: Value =
        serde_json::from_slice(&fs::read(state.join("sessions/boot-failed/session.json")).unwrap())
            .unwrap();
    assert_eq!(
        failed["boot_recovery"]["receipt"],
        json!({"state":"failed", "error":{"code":"session-provider-mismatch"}})
    );
    let no_metadata: Value = serde_json::from_slice(
        &fs::read(state.join("sessions/boot-no-metadata/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        no_metadata["boot_recovery"]["receipt"]["reason"],
        "resume-metadata-missing"
    );
    let expired: Value = serde_json::from_slice(
        &fs::read(state.join("sessions/boot-expired/session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        expired["boot_recovery"]["receipt"]["reason"],
        "shutdown-intent-expired"
    );
    assert!(expired["boot_recovery"].get("shutdown_boot_id").is_none());
    let expired_same: Value =
        serde_json::from_slice(&fs::read(expired_same_dir.join("session.json")).unwrap()).unwrap();
    assert_eq!(
        expired_same["boot_recovery"]["receipt"]["reason"],
        "shutdown-intent-expired"
    );
    assert!(
        expired_same["boot_recovery"]
            .get("shutdown_boot_id")
            .is_none()
    );
    let same: Value =
        serde_json::from_slice(&fs::read(same_dir.join("session.json")).unwrap()).unwrap();
    assert_eq!(same["boot_recovery"]["shutdown_boot_id"], current_boot);
    assert!(same["boot_recovery"].get("receipt").is_none());
    let before_calls = tmux_calls(&log)
        .iter()
        .filter(|call| call.first().is_some_and(|arg| arg == "new-session"))
        .count();
    let mut second = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&second.0, tmp.path(), &tmux);
    let second_deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(addr).is_err() && Instant::now() < second_deadline {
        assert!(second.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(TcpStream::connect(addr).is_ok());
    thread::sleep(Duration::from_millis(200));
    second.0.kill().unwrap();
    second.0.wait().unwrap();
    let after_calls = tmux_calls(&log)
        .iter()
        .filter(|call| call.first().is_some_and(|arg| arg == "new-session"))
        .count();
    assert_eq!(before_calls, 1);
    assert_eq!(
        after_calls, 1,
        "a repeated daemon start must not launch another runtime"
    );
    let list = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "list",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap()),
            ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
        ],
    );
    assert_eq!(list.code, 0, "{}", list.stderr_text());
    let listed = list.stdout_json();
    let rows = listed["data"].as_array().unwrap();
    for (id, expected) in [
        ("boot-recovery", json!({"state":"resumed"})),
        (
            "boot-opted-out",
            json!({"state":"skipped","reason":"session-opt-out"}),
        ),
        (
            "boot-failed",
            json!({"state":"failed","error":{"code":"session-provider-mismatch"}}),
        ),
    ] {
        let row = rows.iter().find(|row| row["id"] == id).unwrap();
        assert_eq!(row["boot_recovery"], expected, "{row}");
    }
    let board = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "board",
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap()),
            ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
        ],
    );
    assert_eq!(board.code, 0, "{}", board.stderr_text());
    let board_json = board.stdout_json();
    let rows = board_json["data"]["board"]["records"].as_array().unwrap();
    for (id, expected) in [
        ("boot-recovery", json!({"state":"resumed"})),
        (
            "boot-opted-out",
            json!({"state":"skipped","reason":"session-opt-out"}),
        ),
        (
            "boot-failed",
            json!({"state":"failed","error":{"code":"session-provider-mismatch"}}),
        ),
    ] {
        let row = rows
            .iter()
            .find(|row| row["session_id"] == id)
            .unwrap_or_else(|| panic!("missing {id}: {board_json}"));
        assert_eq!(row["boot_recovery"], expected, "{row}");
    }
    let mut host_record = record.clone();
    host_record["boot_recovery"]["marked_at_epoch"] = json!(jiff::Timestamp::now().as_second());
    host_record["id"] = json!("boot-host-disabled");
    host_record["tmux_session"] = json!("hs-claude-host-disabled");
    host_record["runtime"]["tmux_session"] = host_record["tmux_session"].clone();
    host_record["provider_resume"]["session_id"] = json!("conversation-host-disabled");
    host_record["provider_resume"]["resume_args"] =
        json!(["--resume", "conversation-host-disabled"]);
    let host_dir = state.join("sessions/boot-host-disabled");
    fs::create_dir_all(&host_dir).unwrap();
    fs::write(
        host_dir.join("session.json"),
        serde_json::to_vec(&host_record).unwrap(),
    )
    .unwrap();
    let mut stale_mark = host_record.clone();
    stale_mark["id"] = json!("boot-stale-mark");
    stale_mark["tmux_session"] = json!("hs-claude-stale-mark");
    stale_mark["runtime"]["tmux_session"] = stale_mark["tmux_session"].clone();
    stale_mark["provider_resume"]["session_id"] = json!("conversation-stale-mark");
    stale_mark["provider_resume"]["resume_args"] = json!(["--resume", "conversation-stale-mark"]);
    stale_mark["boot_recovery"]["marked_at_epoch"] =
        json!(jiff::Timestamp::now().as_second() - 86_400);
    let stale_dir = state.join("sessions/boot-stale-mark");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(
        stale_dir.join("session.json"),
        serde_json::to_vec(&stale_mark).unwrap(),
    )
    .unwrap();
    // A caller-supplied lifetime must not extend a mark past 24 hours.
    command.env("AGENT_SESSION_SHUTDOWN_INTENT_MAX_AGE_SECONDS", "172800");
    command.env("AGENT_SESSION_BOOT_RECOVERY", "0");
    let mut third = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&third.0, tmp.path(), &tmux);
    let host_deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < host_deadline {
        let current: Value =
            serde_json::from_slice(&fs::read(host_dir.join("session.json")).unwrap()).unwrap();
        let stale: Value =
            serde_json::from_slice(&fs::read(stale_dir.join("session.json")).unwrap()).unwrap();
        if current["boot_recovery"]["receipt"]["reason"] == "host-opt-out"
            && stale["boot_recovery"]["receipt"]["reason"] == "shutdown-intent-expired"
        {
            break;
        }
        assert!(third.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    third.0.kill().unwrap();
    third.0.wait().unwrap();
    let skipped: Value =
        serde_json::from_slice(&fs::read(host_dir.join("session.json")).unwrap()).unwrap();
    assert_eq!(
        skipped["boot_recovery"]["receipt"]["reason"],
        "host-opt-out"
    );
    assert_eq!(skipped["runtime"]["generation"], 2);
    let stale: Value =
        serde_json::from_slice(&fs::read(stale_dir.join("session.json")).unwrap()).unwrap();
    assert_eq!(
        stale["boot_recovery"]["receipt"]["reason"],
        "shutdown-intent-expired"
    );
    let control = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "resume",
            "boot-recovery",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", log.to_str().unwrap()),
            ("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1"),
        ],
    );
    assert_eq!(
        control.code,
        0,
        "manual resume control: {} {}",
        control.stdout_text(),
        control.stderr_text()
    );
    assert_eq!(
        after["runtime"]["generation"], 3,
        "serve must resume the recorded running session"
    );
    assert!(!tmp.path().join("default-tmux-invoked").exists());
}

#[test]
fn recovered_private_tmux_runtime_survives_daemon_exit() {
    let tmp = tempfile::tempdir_in("/tmp").unwrap();
    let socket = tmp.path().join("tmux.sock");
    struct PrivateServer(PathBuf);
    impl Drop for PrivateServer {
        fn drop(&mut self) {
            let _ = Command::new("tmux")
                .arg("-S")
                .arg(&self.0)
                .arg("kill-server")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    let _server = PrivateServer(socket.clone());
    let tmux = tmp.path().join("private-tmux");
    let real_tmux = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|dir| dir.join("tmux"))
        .find(|path| path.is_file())
        .expect("tmux executable");
    write_executable(
        &tmux,
        &format!(
            "#!/bin/sh\nexec {} -S {} \"$@\"\n",
            shell_words::quote(&real_tmux.to_string_lossy()),
            shell_words::quote(&socket.to_string_lossy())
        ),
    );
    let state = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir(&cwd).unwrap();
    let agent = fake_agent(tmp.path(), "claude");
    write_executable(
        &agent,
        "#!/bin/sh\n[ \"$1\" = --version ] && { echo '9.9.9 (Fixture)'; exit 0; }\nsleep 60\n",
    );
    let session = write_resumable_session_record_with_agent_bin(
        &state,
        "private-boot",
        "claude",
        "hs-private-boot",
        &cwd,
        &["--resume", "resume-session-id"],
        Some(&agent),
    );
    let path = session.join("session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record
        .as_object_mut()
        .unwrap()
        .remove("tmux_runtime_never_launched");
    record["runtime"]["generation"] = json!(2);
    record["delete_tmux_identity"] = json!({"launch_id":record["runtime"]["launch_id"], "session_id":"$7", "pane_id":"%7", "pane_pid":2000000000, "process_group_id":2000000000, "pid_namespace":{"device":1, "inode":1, "boot_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"}});
    record["boot_recovery"] = json!({"shutdown_boot_id":"aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa", "marked_at_epoch":jiff::Timestamp::now().as_second()});
    fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    struct OwnedServe(Child);
    impl Drop for OwnedServe {
        fn drop(&mut self) {
            if self.0.try_wait().ok().flatten().is_none() {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command.env_clear().env("PATH", fixture_path(tmp.path()));
    command
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "serve",
            "--bind",
            &addr.to_string(),
            "--tmux-bin",
            tmux.to_str().unwrap(),
        ])
        .env("HOME", tmp.path())
        .env("AGENT_HOME", tmp.path().join("agent-home"))
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .env("XDG_STATE_HOME", tmp.path().join("xdg-state"))
        .env("XDG_CONFIG_HOME", tmp.path().join("xdg-config"))
        .env("CODEX_HOME", tmp.path().join("codex-config"))
        .env("CLAUDE_CONFIG_DIR", tmp.path().join("claude-config"))
        .env("TMPDIR", tmp.path())
        .env(
            "XDG_RUNTIME_DIR",
            std::env::var_os("XDG_RUNTIME_DIR").expect("test user runtime directory"),
        )
        .env("AGENT_SESSION_TMUX_SCOPE", "1")
        .env("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "100")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut daemon = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&daemon.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(20);
    let receipt = loop {
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if current["boot_recovery"]["receipt"]["state"].is_string()
            && current["boot_recovery"]["receipt"]["error"]["code"] != "recovery-interrupted"
        {
            break current["boot_recovery"]["receipt"].clone();
        }
        assert!(daemon.0.try_wait().unwrap().is_none(), "serve exited early");
        assert!(Instant::now() < deadline, "recovery timed out: {current}");
        thread::sleep(Duration::from_millis(25));
    };
    assert_eq!(receipt["state"], "resumed", "{receipt}");
    let bad_dir = state.join("sessions/zz-malformed");
    fs::create_dir_all(&bad_dir).unwrap();
    fs::write(bad_dir.join("session.json"), b"{").unwrap();
    let partial = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_ne!(partial.code, 0);
    let partial_json = partial.stdout_json();
    assert_eq!(partial_json["error"]["code"], "shutdown-mark-incomplete");
    let rows = partial_json["error"]["details"]["records"]
        .as_array()
        .unwrap();
    assert!(
        rows.iter()
            .any(|row| row == &json!({"id":"private-boot","state":"marked"})),
        "{partial_json}"
    );
    assert!(rows.iter().any(|row| row == &json!({"id":"zz-malformed","state":"failed","error":{"code":"session-json-invalid"}})), "{partial_json}");
    fs::remove_file(bad_dir.join("session.json")).unwrap();
    fs::remove_dir(&bad_dir).unwrap();
    let mark = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(
        mark.code,
        0,
        "{} {}",
        mark.stdout_text(),
        mark.stderr_text()
    );
    let mark_json = mark.stdout_json();
    let mark_rows = mark_json["data"]["records"].as_array().unwrap();
    assert!(
        mark_rows
            .iter()
            .any(|row| row == &json!({"id":"private-boot", "state":"marked"})),
        "{mark_json}"
    );
    let marked: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        marked["boot_recovery"]["shutdown_boot_id"],
        fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim(),
        "{}",
        mark.stdout_text()
    );
    let uncertain = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            "/nonexistent/tmux",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(uncertain.code, 0, "{}", uncertain.stderr_text());
    let uncertain_json = uncertain.stdout_json();
    assert!(uncertain_json["data"]["records"].as_array().unwrap().iter().any(|row| row == &json!({"id":"private-boot","state":"skipped","reason":"runtime-identity-unverified"})), "{uncertain_json}");
    let unmarked: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(unmarked["boot_recovery"].get("shutdown_boot_id").is_none());
    let mark_again = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(mark_again.code, 0, "{}", mark_again.stderr_text());
    let marked_again: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert!(marked_again["boot_recovery"]["shutdown_boot_id"].is_string());
    assert!(marked_again["boot_recovery"]["marked_at_epoch"].is_i64());
    let bad_format = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--format",
            "json",
            "--unsupported-option",
        ],
        &[],
    );
    assert_ne!(bad_format.code, 0);
    assert_eq!(
        bad_format.stdout_json()["schema_version"],
        "cli.agent-session.prepare-shutdown.v1"
    );
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    assert!(
        Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .args(["has-session", "-t", "=hs-private-boot"])
            .status()
            .unwrap()
            .success()
    );
    let original_mark = marked_again["boot_recovery"]["shutdown_boot_id"].clone();
    let original_time = marked_again["boot_recovery"]["marked_at_epoch"].clone();
    let original_generation = marked_again["runtime"]["generation"].clone();
    let mut same_boot_daemon = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&same_boot_daemon.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(15);
    while http_get(addr, "/healthz").is_err() && Instant::now() < deadline {
        assert!(same_boot_daemon.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        http_get(addr, "/healthz").is_ok(),
        "same-boot daemon did not start"
    );
    let retained: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(retained["boot_recovery"]["shutdown_boot_id"], original_mark);
    assert_eq!(retained["boot_recovery"]["marked_at_epoch"], original_time);
    assert_eq!(retained["runtime"]["generation"], original_generation);
    same_boot_daemon.0.kill().unwrap();
    same_boot_daemon.0.wait().unwrap();
    let context = agent_session::CliContext {
        state_dir: state.clone(),
        host: None,
    };
    let mut live = agent_session::load_session_record(&context, "private-boot").unwrap();
    agent_session::stop_session_runtime_locked(&context, &mut live, &tmux)
        .expect("stop the private fixture runtime");
    let stopped = agent_session::load_session_record(&context, "private-boot").unwrap();
    assert!(
        stopped.extra["boot_recovery"]
            .get("shutdown_boot_id")
            .is_none()
    );
    let mut rebooted: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    rebooted["delete_tmux_identity"]["pid_namespace"]["boot_id"] =
        json!("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
    fs::write(&path, serde_json::to_vec(&rebooted).unwrap()).unwrap();
    let generation = rebooted["runtime"]["generation"].clone();
    let mut restarted = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&restarted.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let current: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        if current["boot_recovery"]["receipt"]["reason"] == "no-shutdown-intent" {
            assert_eq!(current["runtime"]["generation"], generation);
            break;
        }
        assert!(
            restarted.0.try_wait().unwrap().is_none(),
            "serve exited before reporting skip"
        );
        assert!(
            Instant::now() < deadline,
            "recovery skip timed out: {current}"
        );
        thread::sleep(Duration::from_millis(25));
    }
    restarted.0.kill().unwrap();
    restarted.0.wait().unwrap();
    let mut close_record = record.clone();
    close_record["id"] = json!("private-close");
    close_record["tmux_session"] = json!("hs-private-close");
    close_record["runtime"]["tmux_session"] = json!("hs-private-close");
    close_record["provider_resume"]["session_id"] = json!("close-conversation");
    close_record["provider_resume"]["resume_args"] = json!(["--resume", "close-conversation"]);
    close_record
        .as_object_mut()
        .unwrap()
        .remove("boot_recovery");
    let close_dir = state.join("sessions/private-close");
    fs::create_dir_all(&close_dir).unwrap();
    let close_path = close_dir.join("session.json");
    fs::write(&close_path, serde_json::to_vec(&close_record).unwrap()).unwrap();
    let resumed = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "resume",
            "private-close",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[("AGENT_SESSION_TMUX_SCOPE", "1")],
    );
    assert_eq!(
        resumed.code,
        0,
        "{} {}",
        resumed.stdout_text(),
        resumed.stderr_text()
    );
    let marked = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(marked.code, 0, "{}", marked.stderr_text());
    let marked_record: Value = serde_json::from_slice(&fs::read(&close_path).unwrap()).unwrap();
    assert!(marked_record["boot_recovery"]["shutdown_boot_id"].is_string());
    let closed = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "delete",
            "private-close",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(
        closed.code,
        0,
        "{} {}",
        closed.stdout_text(),
        closed.stderr_text()
    );
    assert!(!close_path.exists());
    let mut after_close = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&after_close.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(15);
    while TcpStream::connect(addr).is_err() && Instant::now() < deadline {
        assert!(after_close.0.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    assert!(TcpStream::connect(addr).is_ok());
    after_close.0.kill().unwrap();
    after_close.0.wait().unwrap();
    assert!(!close_path.exists());
    let board = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "board",
            "--state",
            "closed",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(board.code, 0, "{}", board.stderr_text());
    let board_json = board.stdout_json();
    assert!(
        board_json["data"]["board"]["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["session_id"] == "private-close" && row["state"] == "closed"),
        "{board_json}"
    );

    // Keep the command-produced mark untouched while the fixture daemon reads
    // a synthetic next boot ID. The shim affects only spawned test processes.
    let mut boundary = record.clone();
    boundary["id"] = json!("private-boundary");
    boundary["tmux_session"] = json!("hs-private-boundary");
    boundary["runtime"]["tmux_session"] = json!("hs-private-boundary");
    boundary["provider_resume"]["session_id"] = json!("boundary-conversation");
    boundary["provider_resume"]["resume_args"] = json!(["--resume", "boundary-conversation"]);
    boundary.as_object_mut().unwrap().remove("boot_recovery");
    let boundary_dir = state.join("sessions/private-boundary");
    fs::create_dir_all(&boundary_dir).unwrap();
    let boundary_path = boundary_dir.join("session.json");
    fs::write(&boundary_path, serde_json::to_vec(&boundary).unwrap()).unwrap();
    let resumed = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "resume",
            "private-boundary",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[("AGENT_SESSION_TMUX_SCOPE", "1")],
    );
    assert_eq!(
        resumed.code,
        0,
        "{} {}",
        resumed.stdout_text(),
        resumed.stderr_text()
    );
    let marked = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "prepare-shutdown",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(
        marked.code,
        0,
        "{} {}",
        marked.stdout_text(),
        marked.stderr_text()
    );
    let original: Value = serde_json::from_slice(&fs::read(&boundary_path).unwrap()).unwrap();
    let mark = original["boot_recovery"]["shutdown_boot_id"]
        .as_str()
        .unwrap();
    assert_eq!(
        mark,
        fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim()
    );
    let old_generation = original["runtime"]["generation"].as_u64().unwrap();
    assert!(
        Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .arg("kill-server")
            .status()
            .unwrap()
            .success()
    );
    let fake_boot = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    assert_ne!(fake_boot, mark);
    let boot_file = tmp.path().join("fixture-boot-id");
    fs::write(&boot_file, format!("{fake_boot}\n")).unwrap();
    let shim = tmp.path().join("fixture-boot-id.so");
    let shim_source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/integration/cli/boot_id_shim.c");
    let compile = Command::new("cc")
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .args(["-shared", "-fPIC", "-o"])
        .arg(&shim)
        .arg(&shim_source)
        .arg("-ldl")
        .output()
        .unwrap();
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    command
        .env("LD_PRELOAD", &shim)
        .env("AGENT_SESSION_FIXTURE_BOOT_ID_FILE", &boot_file);
    let mut next_boot = OwnedServe(command.spawn().unwrap());
    assert_fixture_environment(&next_boot.0, tmp.path(), &tmux);
    let deadline = Instant::now() + Duration::from_secs(50);
    loop {
        let current: Value = serde_json::from_slice(&fs::read(&boundary_path).unwrap()).unwrap();
        if current["boot_recovery"]["receipt"]["state"] == "resumed" {
            assert_eq!(current["boot_recovery"]["receipt_boot_id"], fake_boot);
            assert_eq!(current["runtime"]["generation"], old_generation + 1);
            assert!(current["boot_recovery"].get("shutdown_boot_id").is_none());
            break;
        }
        assert!(
            next_boot.0.try_wait().unwrap().is_none(),
            "simulated next-boot daemon exited"
        );
        assert!(
            Instant::now() < deadline,
            "simulated recovery timed out: {current}"
        );
        thread::sleep(Duration::from_millis(25));
    }
    next_boot.0.kill().unwrap();
    next_boot.0.wait().unwrap();
    assert!(
        Command::new("tmux")
            .arg("-S")
            .arg(&socket)
            .args(["has-session", "-t", "=hs-private-boundary"])
            .status()
            .unwrap()
            .success()
    );
    assert!(!tmp.path().join("default-tmux-invoked").exists());
}

//! Keep argument and readiness contracts independent from the wrapper implementation.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};

fn normalize_held_launch(arguments: &[String]) -> Vec<String> {
    let index = arguments
        .iter()
        .position(|arg| arg.contains("gate=$1; broker_gate=$2"))
        .expect("held-launch wrapper");
    assert_eq!(&arguments[index - 2..index], ["sh", "-c"]);
    assert_eq!(arguments[index + 1], "agent-session-held-launch");
    assert!(arguments[index].contains(r#"; "$@"; status=$?;"#));
    let mut normalized = arguments.to_vec();
    normalized[index] = "<held-launch>".into();
    normalized
}

pub(super) fn start_creates_session_state_without_printing_prompt() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo with spaces");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");

    let state_arg = state_dir.to_string_lossy().to_string();
    let cwd_arg = cwd.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let codex_arg = codex_bin.to_string_lossy().to_string();
    let prompt = "sent from telegram with sk-proj-secret";
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "--host",
            "sympoies",
            "start",
            "--agent",
            "codex",
            "--cwd",
            &cwd_arg,
            "--title",
            "Fix API",
            "--prompt",
            prompt,
            "--tmux-bin",
            &tmux_arg,
            "--agent-bin",
            &codex_arg,
            "--agent-arg",
            "dangerous value; $(nope)",
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ],
        &[(
            "AGENT_SESSION_FAKE_TMUX_LOG",
            tmux_log.to_string_lossy().as_ref(),
        )],
    );

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    assert_no_secret(&output, "sk-proj-secret");
    let value = output.stdout_json();
    assert_eq!(value["schema_version"], "cli.agent-session.start.v1");
    let result = data(&value);
    assert_eq!(result["agent"], "codex");
    assert_eq!(result["cwd"], cwd_arg);
    assert!(
        result["attach_command"]
            .as_str()
            .unwrap()
            .starts_with("tmux attach -t hs-codex-")
    );
    assert!(
        result["ssh_attach_command"]
            .as_str()
            .unwrap()
            .starts_with("ssh -t sympoies ")
    );

    let id = result["id"].as_str().expect("id");
    let prompt_file = state_dir.join("sessions").join(id).join("prompt.md");
    assert_eq!(
        fs::read_to_string(prompt_file).expect("prompt file"),
        prompt
    );
    let record: Value = serde_json::from_str(
        &fs::read_to_string(state_dir.join("sessions").join(id).join("session.json"))
            .expect("session record"),
    )
    .expect("session json");
    let runtime_id = record["runtime"]["launch_id"].as_str().expect("runtime id");
    let checkpoint_file = state_dir.join("sessions").join(id).join(format!(
        "coordination/main-agent-checkpoint-{}.json",
        sha256_hex(runtime_id)
    ));
    let checkpoint_metadata =
        fs::symlink_metadata(&checkpoint_file).expect("runtime checkpoint file");
    assert!(checkpoint_metadata.is_file());
    assert_eq!(checkpoint_metadata.permissions().mode() & 0o777, 0o600);
    let agent_session_bin = nils_test_support::bin::resolve("agent-session")
        .to_string_lossy()
        .to_string();

    let calls = tmux_calls(&tmux_log);
    let new_session = calls
        .iter()
        .find(|call| call.first().is_some_and(|arg| arg == "new-session"))
        .expect("new-session call");
    let new_session = &normalize_held_launch(new_session);
    let inherited_path = std::env::var("PATH").expect("test PATH");
    assert_eq!(
        new_session,
        &vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-P".to_string(),
            "-F".to_string(),
            // Space-separated on purpose: some tmux builds rewrite a literal
            // tab in expanded format output, which rejected valid identities.
            "#{session_id} #{pane_id} #{pane_pid}".to_string(),
            "-s".to_string(),
            result["tmux_session"].as_str().unwrap().to_string(),
            "-c".to_string(),
            cwd_arg.clone(),
            "-e".to_string(),
            format!("TMPDIR={}", tmp.path().join("agent-home/tmp").join(id).display()),
            "-e".to_string(),
            "AGENT_SESSION_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_URL".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_INGRESS_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_CONSOLE_COORDINATION_RELAYS".to_string(),
            "-e".to_string(),
            format!("AGENT_SESSION_ID={id}"),
            "-e".to_string(),
            format!("AGENT_SESSION_STATE_DIR={}", state_dir.display()),
            "-e".to_string(),
            format!("AGENT_SESSION_RUNTIME_ID={runtime_id}"),
            "-e".to_string(),
            "AGENT_SESSION_COORDINATION_MODE=advisory".to_string(),
            "-e".to_string(),
            format!(
                "AGENT_SESSION_CAPABILITY_FILE={}",
                state_dir
                    .join("sessions")
                    .join(id)
                    .join(format!("coordination/capability-{}", sha256_hex(runtime_id)))
                    .display()
            ),
            "-e".to_string(),
            format!(
                "AGENT_SESSION_CHECKPOINT_FILE={}",
                state_dir
                    .join("sessions")
                    .join(id)
                    .join(format!(
                        "coordination/main-agent-checkpoint-{}.json",
                        sha256_hex(runtime_id)
                    ))
                    .display()
            ),
            "-e".to_string(),
            "AGENT_SESSION_ATTENTION_AUTHORITY=hook".to_string(),
            "-e".to_string(),
            format!("PATH={inherited_path}"),
            "-e".to_string(),
            format!("AGENT_SESSION_BIN={agent_session_bin}"),
            "--".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "<held-launch>".to_string(),
            "agent-session-held-launch".to_string(),
            state_dir
                .join("sessions")
                .join(id)
                .join("coordination/launch-ready")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join("sessions")
                .join(id)
                .join("coordination/broker-provisioned")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join("sessions")
                .join(id)
                .join("coordination/heartbeat")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join("sessions")
                .join(id)
                .join(format!("coordination/capability-{}", sha256_hex(runtime_id)))
                .to_string_lossy()
                .to_string(),
            runtime_id.to_string(),
            "1".to_string(),
            agent_session_bin,
            "sh".to_string(),
            "-c".to_string(),
            "unset NO_COLOR AGENT_SESSION_TOKEN AGENT_SESSION_RELAY_URL AGENT_SESSION_RELAY_TOKEN AGENT_SESSION_RELAY_INGRESS_TOKEN AGENT_CONSOLE_COORDINATION_RELAYS; exec \"$@\"".to_string(),
            "agent-session-interactive".to_string(),
            codex_arg.clone(),
            "--cd".to_string(),
            cwd_arg.clone(),
            "--no-alt-screen".to_string(),
            "dangerous value; $(nope)".to_string(),
        ]
    );
    assert!(
        calls
            .iter()
            .any(|call| call.first().is_some_and(|arg| arg == "load-buffer")),
        "missing load-buffer call: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call
            == &vec![
                "paste-buffer".to_string(),
                "-p".to_string(),
                "-b".to_string(),
                format!("{id}-prompt"),
                "-d".to_string(),
                "-t".to_string(),
                format!("{}:0.0", result["tmux_session"].as_str().unwrap()),
            ]),
        "missing paste-buffer -d call: {calls:?}"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.first().is_some_and(|arg| arg == "paste-buffer"))
            .count(),
        1,
        "an unchanged pane cannot authorize replaying the initial prompt"
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| {
                call.first().is_some_and(|arg| arg == "send-keys")
                    && call.last().is_some_and(|arg| arg == "Enter")
            })
            .count(),
        1,
        "startup must submit exactly once"
    );
}

pub(super) fn start_captures_stable_codex_session_meta_before_full_timeout() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let codex_home = tmp.path().join("codex-home");
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");
    let codex_session = codex_home.join("sessions/2026/07/05/stable.jsonl");

    let state_arg = state_dir.to_string_lossy().to_string();
    let cwd_arg = cwd.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let codex_arg = codex_bin.to_string_lossy().to_string();
    let tmux_log_arg = tmux_log.to_string_lossy().to_string();
    let codex_home_arg = codex_home.to_string_lossy().to_string();
    let codex_session_arg = codex_session.to_string_lossy().to_string();
    let started = std::time::Instant::now();
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "start",
            "--agent",
            "codex",
            "--cwd",
            &cwd_arg,
            "--tmux-bin",
            &tmux_arg,
            "--agent-bin",
            &codex_arg,
            "--paste-delay-ms",
            "0",
            "--format",
            "json",
        ],
        &[
            ("CODEX_HOME", &codex_home_arg),
            ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_CAPTURE", "› \n\n  status\n"),
            ("AGENT_SESSION_FAKE_CODEX_SESSION_FILE", &codex_session_arg),
            ("AGENT_SESSION_FAKE_CODEX_SESSION_ID", "stable-codex-id"),
            ("AGENT_SESSION_FAKE_CODEX_CWD", &cwd_arg),
            // Large timeout so the "returned before the timeout" signal has a
            // wide margin: a stable session confirms right after the 40ms
            // ambiguity window, so elapsed is dominated by process-spawn
            // overhead (load-dependent), never the timeout.
            ("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "60000"),
            ("AGENT_SESSION_CODEX_CAPTURE_POLL_MS", "10"),
            ("AGENT_SESSION_CODEX_AMBIGUITY_WINDOW_MS", "40"),
        ],
    );
    let elapsed = started.elapsed();

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    // Confirm well within half the 60s timeout. This proves the stable session
    // is captured without waiting for the full timeout, while the 30s margin
    // absorbs process-spawn overhead on loaded hosts (2.5s over a 5s timeout
    // was still timing-flaky). A real "waited the timeout" regression is ~60s
    // and still fails this bound.
    assert!(
        elapsed < std::time::Duration::from_secs(30),
        "stable capture should confirm before the timeout, not wait for it; elapsed={elapsed:?}"
    );
    let value = output.stdout_json();
    let result = data(&value);
    assert_eq!(result["resumable"], true);
}

pub(super) fn resume_recreates_tmux_runtime_from_exact_provider_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state_dir = tmp.path().join("state");
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let (tmux_bin, tmux_log) = fake_tmux(tmp.path());
    let codex_bin = fake_agent(tmp.path(), "codex");
    let session = write_resumable_session_record_with_agent_bin(
        &state_dir,
        "recoverable",
        "codex",
        "hs-codex-recoverable",
        &cwd,
        &[
            "resume",
            "resume-session-id",
            "--cd",
            cwd.to_str().unwrap(),
            "--no-alt-screen",
        ],
        Some(&codex_bin),
    );
    let record_path = session.join("session.json");
    let mut failed_record: Value =
        serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    let previous_runtime_id = failed_record["runtime"]["launch_id"]
        .as_str()
        .expect("previous runtime id")
        .to_string();
    let previous_checkpoint = session.join(format!(
        "coordination/main-agent-checkpoint-{}.json",
        sha256_hex(&previous_runtime_id)
    ));
    fs::create_dir_all(previous_checkpoint.parent().expect("coordination dir")).unwrap();
    fs::write(&previous_checkpoint, b"{\"state\":\"stale\"}\n").unwrap();
    fs::set_permissions(&previous_checkpoint, fs::Permissions::from_mode(0o600)).unwrap();
    failed_record["startup"] = json!({
        "schema_version": "agent-session.startup.v1",
        "state": "failed",
        "stage": "tmux",
        "started_at": "2000-01-01T00:00:00Z",
        "failure_code": "terminal-runtime-create-failed",
        "message": "The terminal runtime could not be created.",
        "occurred_at": "2000-01-01T00:00:01Z",
        "retry_safe": true
    });
    fs::write(
        &record_path,
        serde_json::to_string_pretty(&failed_record).unwrap(),
    )
    .unwrap();
    fs::write(session.join(".startup-stage"), "tmux\n").unwrap();
    fs::write(
        session.join(".startup-failure"),
        "terminal-runtime-create-failed\n",
    )
    .unwrap();
    fs::write(
        session.join(".startup-diagnostic.log"),
        "private prior failure\n",
    )
    .unwrap();
    fs::write(session.join(".runtime-exit-status"), "17\n").unwrap();

    let state_arg = state_dir.to_string_lossy().to_string();
    let tmux_arg = tmux_bin.to_string_lossy().to_string();
    let tmux_log_arg = tmux_log.to_string_lossy().to_string();
    let codex_arg = codex_bin.to_string_lossy().to_string();
    let daemon_roster = "fixture-account-a fixture-account-b";
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "resume",
            "recoverable",
            "--tmux-bin",
            &tmux_arg,
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
            ("NILS_TEST_PANE_LIFETIME_MS", "500"),
            ("CODEX_AUTH_PROFILE_NAMES", daemon_roster),
        ],
    );

    assert_eq!(output.code, 0, "stderr={}", output.stderr_text());
    let value = output.stdout_json();
    assert_eq!(value["schema_version"], "cli.agent-session.resume.v1");
    let result = data(&value);
    assert_eq!(result["id"], "recoverable");
    assert_eq!(result["status"], "running");
    assert_eq!(result["resumable"], true);
    assert_eq!(result["startup"]["state"], "ready");

    let calls = tmux_calls(&tmux_log);
    let new_session = calls
        .iter()
        .find(|call| call.first().is_some_and(|arg| arg == "new-session"))
        .expect("new-session call");
    let new_session = &normalize_held_launch(new_session);
    let record: Value = serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
    let runtime_id = record["runtime"]["launch_id"].as_str().expect("runtime id");
    let agent_session_bin = nils_test_support::bin::resolve("agent-session")
        .to_string_lossy()
        .to_string();
    let inherited_path = std::env::var("PATH").expect("test PATH");
    assert_eq!(
        new_session,
        &vec![
            "new-session".to_string(),
            "-d".to_string(),
            "-P".to_string(),
            "-F".to_string(),
            // Space-separated on purpose: some tmux builds rewrite a literal
            // tab in expanded format output, which rejected valid identities.
            "#{session_id} #{pane_id} #{pane_pid}".to_string(),
            "-s".to_string(),
            "hs-codex-recoverable".to_string(),
            "-c".to_string(),
            cwd.to_string_lossy().to_string(),
            "-e".to_string(),
            format!("TMPDIR={}", tmp.path().join("agent-home/tmp/recoverable").display()),
            "-e".to_string(),
            "AGENT_SESSION_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_URL".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_RELAY_INGRESS_TOKEN".to_string(),
            "-e".to_string(),
            "AGENT_CONSOLE_COORDINATION_RELAYS".to_string(),
            "-e".to_string(),
            "AGENT_SESSION_ID=recoverable".to_string(),
            "-e".to_string(),
            format!("AGENT_SESSION_STATE_DIR={}", state_dir.display()),
            "-e".to_string(),
            format!("AGENT_SESSION_RUNTIME_ID={runtime_id}"),
            "-e".to_string(),
            "AGENT_SESSION_COORDINATION_MODE=advisory".to_string(),
            "-e".to_string(),
            format!(
                "AGENT_SESSION_CAPABILITY_FILE={}",
                state_dir
                    .join(format!(
                        "sessions/recoverable/coordination/capability-{}",
                        sha256_hex(runtime_id)
                    ))
                    .display()
            ),
            "-e".to_string(),
            format!(
                "AGENT_SESSION_CHECKPOINT_FILE={}",
                state_dir
                    .join(format!(
                        "sessions/recoverable/coordination/main-agent-checkpoint-{}.json",
                        sha256_hex(runtime_id)
                    ))
                    .display()
            ),
            "-e".to_string(),
            "AGENT_SESSION_ATTENTION_AUTHORITY=hook".to_string(),
            "-e".to_string(),
            format!("PATH={inherited_path}"),
            "-e".to_string(),
            format!("CODEX_AUTH_PROFILE_NAMES={daemon_roster}"),
            "-e".to_string(),
            format!("AGENT_SESSION_BIN={agent_session_bin}"),
            "--".to_string(),
            "sh".to_string(),
            "-c".to_string(),
            "<held-launch>".to_string(),
            "agent-session-held-launch".to_string(),
            state_dir
                .join("sessions/recoverable/coordination/launch-ready")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join("sessions/recoverable/coordination/broker-provisioned")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join("sessions/recoverable/coordination/heartbeat")
                .to_string_lossy()
                .to_string(),
            state_dir
                .join(format!(
                    "sessions/recoverable/coordination/capability-{}",
                    sha256_hex(runtime_id)
                ))
                .to_string_lossy()
                .to_string(),
            runtime_id.to_string(),
            "2".to_string(),
            agent_session_bin,
            "sh".to_string(),
            "-c".to_string(),
            "unset NO_COLOR AGENT_SESSION_TOKEN AGENT_SESSION_RELAY_URL AGENT_SESSION_RELAY_TOKEN AGENT_SESSION_RELAY_INGRESS_TOKEN AGENT_CONSOLE_COORDINATION_RELAYS; exec \"$@\"".to_string(),
            "agent-session-interactive".to_string(),
            codex_arg.clone(),
            "resume".to_string(),
            "resume-session-id".to_string(),
            "--cd".to_string(),
            cwd.to_string_lossy().to_string(),
            "--no-alt-screen".to_string(),
            "--model".to_string(),
            "fixture-model".to_string(),
        ]
    );

    assert_eq!(record["id"], "recoverable");
    assert_eq!(record["runtime"]["generation"], 2);
    let replacement_checkpoint = session.join(format!(
        "coordination/main-agent-checkpoint-{}.json",
        sha256_hex(runtime_id)
    ));
    assert!(
        !previous_checkpoint.exists(),
        "resume must remove the superseded incarnation checkpoint"
    );
    assert!(
        replacement_checkpoint.is_file(),
        "resume must create the replacement incarnation checkpoint"
    );
    assert_eq!(
        fs::symlink_metadata(&replacement_checkpoint)
            .expect("replacement checkpoint metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_ne!(record["updated_at"], "2000-01-01T00:00:00Z");
    assert_eq!(record["agent_bin"], codex_arg);
    assert_eq!(record["startup"]["state"], "ready");
    assert!(!session.join(".startup-failure").exists());
    assert!(!session.join(".startup-diagnostic.log").exists());
    assert!(!session.join(".runtime-exit-status").exists());
    assert!(
        record_path.with_file_name("resume.json").is_file(),
        "resume should refresh the durable sidecar"
    );

    let capability = session.join(format!(
        "coordination/capability-{}",
        sha256_hex(runtime_id)
    ));
    let stopped = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "broker",
            "stop",
            "--session",
            "recoverable",
            "--exit-code",
            "23",
            "--capability-file",
            capability.to_str().unwrap(),
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(stopped.code, 0, "{}", stopped.stderr_text());

    let list = run(
        tmp.path(),
        &["--state-dir", &state_arg, "list", "--format", "json"],
        &[
            ("AGENT_SESSION_TMUX_BIN", &tmux_arg),
            ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_HAS_SESSION", "0"),
        ],
    );
    assert_eq!(list.code, 0, "stderr={}", list.stderr_text());
    let list_value = list.stdout_json();
    let listed = &data(&list_value)[0];
    assert_eq!(listed["status"], "stopped");
    assert_eq!(listed["startup"]["state"], "ready");
    // Inventory may observe this disappearance again; wrapper evidence remains singular.
    let context = agent_session::CliContext {
        state_dir: state_dir.clone(),
        host: None,
    };
    let persisted = agent_session::load_session_record(&context, "recoverable").unwrap();
    assert_eq!(persisted.runtime.as_ref().unwrap().generation, 2);
    let logs = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "logs",
            "--lifecycle",
            "recoverable",
            "--format",
            "json",
        ],
        &[],
    );
    assert_eq!(logs.code, 0, "{}", logs.stderr_text());
    let journal = logs.stdout_json();
    let rows = journal["data"]["records"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r["operation"] == "resume" && r["result"]["ok"] == true)
            .count(),
        1
    );
    assert_eq!(
        rows.iter()
            .filter(|r| r["operation"] == "broker-stop" && r["result"]["ok"] == true)
            .count(),
        1
    );
    let exits = rows
        .iter()
        .filter(|r| r["operation"] == "exit-observed")
        .collect::<Vec<_>>();
    assert_eq!(exits.len(), 1, "{journal}");
    assert_eq!(exits[0]["exit"]["code"], 23);
    assert_eq!(exits[0]["exit"]["stopped_by"], Value::Null);
    let group = record["delete_tmux_identity"]["process_group_id"]
        .as_i64()
        .unwrap() as libc::pid_t;
    let deadline = Instant::now() + Duration::from_secs(2);
    while unsafe { libc::kill(-group, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_ne!(
        unsafe { libc::kill(-group, 0) },
        0,
        "bounded resumed pane must stop before deletion"
    );
    let deleted = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "delete",
            "recoverable",
            "--tmux-bin",
            &tmux_arg,
            "--format",
            "json",
        ],
        &[
            ("AGENT_SESSION_FAKE_TMUX_LOG", &tmux_log_arg),
            ("AGENT_SESSION_FAKE_TMUX_ABSENT", "1"),
        ],
    );
    assert_eq!(
        deleted.code,
        0,
        "{} {}",
        deleted.stdout_text(),
        deleted.stderr_text()
    );
    let logs = run(
        tmp.path(),
        &[
            "--state-dir",
            &state_arg,
            "logs",
            "--lifecycle",
            "recoverable",
            "--format",
            "json",
        ],
        &[],
    );
    let journal = logs.stdout_json();
    let rows = journal["data"]["records"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|r| r["operation"] == "delete" && r["result"]["ok"] == true)
            .count(),
        1
    );
}

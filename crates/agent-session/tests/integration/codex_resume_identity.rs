use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use nils_test_support::cmd::{run_resolved, run_with};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use super::cli::{RealTmuxServer, data, sha256_hex, write_executable};
use super::hermetic_provider_options;

const THREAD: &str = "9fb830b1-30d5-45b2-91ea-c9ac1946fa12";
const TURN: &str = "71f0e080-d1c5-474a-9b18-721e4ec62cf6";
const POLICY: &str = r#"schema_version = "agent-hook.policy.v1"
bundle_id = "runtime-kit"
version = "2026.07.20.1"

[[rules]]
id = "coord.activity.prompt-stop"
products = ["codex"]
events = ["UserPromptSubmit", "Stop"]
priority = 100
mode = "enforce"
failure_posture = "closed"
override_class = "locked"
capability = { id = "agent-session.activity.v1", reason_code = "activity-recorded" }
"#;

#[test]
fn codex_dispatch_preserves_identity_through_normal_exit_and_resume() {
    exercise_codex_resume(None);
}

#[test]
fn codex_dispatch_reports_resume_capture_timeout_after_activity_admission() {
    exercise_codex_resume(Some("sleep 10"));
}

#[test]
fn codex_dispatch_reports_resume_write_failure_despite_fail_open_helper_exit() {
    exercise_codex_resume(Some(
        r#"mkdir "$AGENT_SESSION_STATE_DIR/sessions/$AGENT_SESSION_ID/resume.json""#,
    ));
}

fn exercise_codex_resume(capture_fault: Option<&str>) {
    let Some(agent_hook) = nils_test_support::bin::sibling_or_skip("agent-hook", "nils-agent-hook")
    else {
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let root_arg = root.to_str().unwrap();
    let Some(tmux) = RealTmuxServer::start(root) else {
        return;
    };
    let state = root.join("sessions-state");
    let codex_home = root.join("codex");
    let provider = root.join("fake-codex");
    let policy = root.join("policy.toml");
    let config = root.join("hook.toml");
    let home = root.join("home");
    fs::create_dir(&home).unwrap();
    fs::write(&policy, POLICY).unwrap();
    fs::write(
        &config,
        format!(
            "schema_version = \"agent-hook.config.v1\"\n[policy]\npath = {}\ndigest = \"sha256:{}\"\n",
            json!(policy),
            sha256_hex(POLICY),
        ),
    )
    .unwrap();
    for path in [&policy, &config] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }
    write_executable(
        &provider,
        &format!(
            r#"#!/bin/sh
set -eu
if [ "${{1:-}}" = resume ]; then
    test "$#" -eq 5
    test "$2" = {THREAD}
    test "$3" = --cd
    test "$4" = "$PWD"
    test "$5" = --no-alt-screen
    printf '%s\n' "$2" > resumed-thread
fi
printf '› \033[2mAsk Codex to do anything\033[22m\n\n  100%% left\n'
IFS= read -r line
test "$line" = /exit
"#,
        ),
    );
    let options = hermetic_provider_options(root)
        .without_ambient_managed_session_env()
        .with_envs(&[
            ("HOME", home.to_str().unwrap()),
            ("XDG_CONFIG_HOME", root_arg),
            ("XDG_DATA_HOME", root_arg),
            ("XDG_STATE_HOME", root_arg),
            ("AGENT_SESSION_TMUX_BIN", tmux.wrapper.to_str().unwrap()),
            ("AGENT_SESSION_STATE_DIR", state.to_str().unwrap()),
            ("CODEX_HOME", codex_home.to_str().unwrap()),
            ("AGENT_SESSION_CODEX_CAPTURE_TIMEOUT_MS", "500"),
        ]);
    let run = |args: &[&str]| run_resolved("agent-session", args, &options);
    let start = run(&[
        "start",
        "--agent",
        "codex",
        "--id",
        "codex-fixture",
        "--cwd",
        root_arg,
        "--agent-bin",
        provider.to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(
        start.code,
        0,
        "{} {}",
        start.stdout_text(),
        start.stderr_text()
    );
    let started = data(&start.stdout_json()).clone();
    assert_eq!(started["status"], "running");
    assert_eq!(started["startup"]["state"], "ready");
    assert!(started["provider_resume"].is_null());
    let id = started["id"].as_str().unwrap();
    let incarnation = started["session_incarnation"].as_str().unwrap();
    let tmux_session = started["tmux_session"].as_str().unwrap();
    let resume = || run(&["resume", id, "--format", "json"]);
    let live = resume();
    assert_eq!(live.code, 0, "{}", live.stdout_text());
    assert_eq!(
        data(&live.stdout_json())["session_incarnation"],
        incarnation
    );
    assert!(!root.join("resumed-thread").exists());

    // Both histories become durable after startup. Matching by cwd and time
    // alone cannot identify the conversation that ran in this managed pane.
    let histories = codex_home.join("sessions");
    fs::create_dir_all(&histories).unwrap();
    for thread in [THREAD, "05d48212-4323-4453-88c7-2d1fad586f92"] {
        let timestamp = jiff::Timestamp::now().to_string();
        let meta = json!({
            "timestamp": timestamp, "type": "session_meta",
            "payload": {"id": thread, "cwd": root, "timestamp": timestamp, "source": "cli"},
        });
        fs::write(
            histories.join(format!("rollout-{thread}.jsonl")),
            format!("{meta}\n"),
        )
        .unwrap();
    }

    let real_helper = nils_test_support::bin::resolve("agent-session");
    let helper = if let Some(fault) = capture_fault {
        let wrapper = root.join("faulted-activity-helper");
        write_executable(
            &wrapper,
            &format!(
                r#"#!/bin/sh
set -eu
if [ "$1" = activity ] && [ "$2" = hook ]; then
    : > "$AGENT_SESSION_STATE_DIR/receipt-attempt"
    {fault}
fi
exec "$FIXTURE_REAL_HELPER" "$@"
"#,
            ),
        );
        wrapper
    } else {
        real_helper.clone()
    };
    let hook_options = options.clone().with_envs(&[
        ("AGENT_SESSION_BIN", helper.to_str().unwrap()),
        ("FIXTURE_REAL_HELPER", real_helper.to_str().unwrap()),
        ("AGENT_SESSION_ID", id),
        ("AGENT_SESSION_RUNTIME_ID", incarnation),
    ]);
    let mut prompt_decision = Value::Null;
    for (name, fields) in [
        (
            "UserPromptSubmit",
            json!({"prompt": "synthetic-turn-input"}),
        ),
        (
            "Stop",
            json!({"stop_hook_active": false, "last_assistant_message": "synthetic-turn-output"}),
        ),
    ] {
        let mut payload = fields;
        payload["hook_event_name"] = json!(name);
        payload["session_id"] = json!(THREAD);
        payload["turn_id"] = json!(TURN);
        payload["cwd"] = json!(root);
        payload["transcript_path"] = json!(histories.join(format!("rollout-{THREAD}.jsonl")));
        let hook = run_with(
            &agent_hook,
            &[
                "--config",
                config.to_str().unwrap(),
                "dispatch",
                "--product",
                "codex",
                "--format",
                "json",
            ],
            &hook_options.clone().with_stdin_str(&payload.to_string()),
        );
        assert_eq!(
            hook.code,
            0,
            "{} {}",
            hook.stdout_text(),
            hook.stderr_text()
        );
        let decision = data(&hook.stdout_json()).clone();
        if name == "UserPromptSubmit" && capture_fault.is_some() {
            prompt_decision = decision;
        } else {
            assert_eq!(decision["reasons"][0]["code"], "activity-recorded");
        }
        assert!(!hook.stdout_text().contains(THREAD));
        assert!(!hook.stderr_text().contains(THREAD));
    }
    let completion = json!({
        "type": "agent-turn-complete", "thread-id": THREAD, "turn-id": TURN,
        "last-assistant-message": "synthetic-turn-output",
    });
    let notified = run_resolved(
        "agent-session",
        &[
            "activity",
            "notify",
            "--agent",
            "codex",
            &completion.to_string(),
        ],
        &hook_options,
    );
    assert_eq!(notified.code, 0, "{}", notified.stderr_text());
    let session_view = || {
        let output = run(&["list", "--format", "json"]);
        assert_eq!(
            output.code,
            0,
            "{} {}",
            output.stdout_text(),
            output.stderr_text()
        );
        let session = data(&output.stdout_json())[0].clone();
        assert_eq!(session["id"], id);
        session
    };
    let completed = session_view();
    assert_eq!(completed["turn_state"]["phase"], "waiting");
    assert_eq!(completed["turn_state"]["last_turn"]["outcome"], "completed");
    assert_eq!(completed["turn_state"]["source"]["kind"], "provider_hook");
    let record_dir = state.join("sessions").join(id);
    for file in ["activity.json", "activity.journal.jsonl"] {
        let activity = fs::read_to_string(record_dir.join(file)).unwrap();
        for private in [
            THREAD,
            TURN,
            "synthetic-turn-input",
            "synthetic-turn-output",
            "transcript_path",
        ] {
            assert!(
                !activity.contains(private),
                "{file} contains private hook data"
            );
        }
    }

    assert!(
        Command::new(&tmux.wrapper)
            .args(["send-keys", "-t", tmux_session, "/exit", "Enter"])
            .status()
            .unwrap()
            .success()
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let stopped = loop {
        let view = session_view();
        if view["status"] == "stopped" {
            break view;
        }
        assert!(Instant::now() < deadline, "fixture provider did not exit");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        fs::read_to_string(record_dir.join(".runtime-exit-status"))
            .unwrap()
            .trim(),
        "0",
    );

    let relaunched = resume();
    if capture_fault.is_some() {
        assert!(state.join("receipt-attempt").is_file());
        assert_eq!(stopped["resumable"], false);
        assert!(stopped["provider_resume"].is_null());
        assert_eq!(relaunched.code, 65, "{}", relaunched.stdout_text());
        assert_eq!(
            relaunched.stdout_json()["error"]["code"],
            "session-not-resumable"
        );
        assert_eq!(
            prompt_decision["reasons"][0]["code"],
            "codex-resume-identity-capture-failed",
        );
        assert_eq!(prompt_decision["action"], "warn");
        let diagnosis = run(&["diagnose", "--format", "json"]);
        assert_eq!(diagnosis.code, 0, "{}", diagnosis.stdout_text());
        let diagnostic = diagnosis.stdout_json();
        assert!(
            data(&diagnostic)["observation"]["summary"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["code"] == "codex-resume-identity-capture-failed")
        );
        assert!(!diagnosis.stdout_text().contains(THREAD));
        return;
    }
    assert_eq!(
        relaunched.code,
        0,
        "{} {}",
        relaunched.stdout_text(),
        relaunched.stderr_text()
    );
    assert_eq!(stopped["resumable"], true);
    assert_eq!(stopped["provider_resume"]["session_id"], THREAD);
    assert_eq!(
        stopped["provider_resume"]["capture_method"],
        "codex-user-prompt-submit-hook",
    );
    let resumed = data(&relaunched.stdout_json()).clone();
    assert_eq!(resumed["id"], id);
    assert_eq!(resumed["status"], "running");
    assert_ne!(resumed["session_incarnation"], incarnation);
    assert_eq!(resumed["provider_resume"]["session_id"], THREAD);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(thread) = fs::read_to_string(root.join("resumed-thread"))
            && thread.trim() == THREAD
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture provider did not resume the exact thread"
        );
        thread::sleep(Duration::from_millis(20));
    }
    let sidecar: Value =
        serde_json::from_slice(&fs::read(record_dir.join("resume.json")).unwrap()).unwrap();
    assert_eq!(sidecar["provider_resume"]["session_id"], THREAD);
}

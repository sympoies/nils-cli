use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

#[test]
fn move_requests_require_local_trust_and_direct_transport() {
    let fixture = tempfile::tempdir().unwrap();
    let state = fixture.path().join("state");
    for mode in [None, Some("--check-resume"), Some("--via-console")] {
        let mut args = vec![
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--account",
            "fixture-account",
            "--resume",
            "00000000-0000-4000-8000-000000000001",
            "--move-id",
            "00000000-0000-4000-8000-000000000002",
            "--format",
            "json",
        ];
        args.extend(mode);
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        let response = output.stdout_json();
        assert_ne!(output.code, 0);
        let expected = if mode == Some("--via-console") {
            "resume-move-unavailable"
        } else {
            "resume-move-trust-unavailable"
        };
        assert_eq!(response["error"]["code"], expected);
        assert_eq!(response["error"]["details"]["session_created"], false);
        assert!(!state.exists());
    }
}

#[test]
fn source_mutations_require_verifier_before_reading_proof_or_writing_state() {
    let fixture = tempfile::tempdir().unwrap();
    let state = fixture.path().join("state");
    let unavailable_file = fixture.path().join("unavailable");
    let path = unavailable_file.to_str().unwrap();
    for command in [
        vec![
            "export",
            "--session",
            "source",
            "--incarnation",
            "00000000-0000-4000-8000-000000000001",
            "--destination",
            "host-b",
            "--ticket-file",
            path,
        ],
        vec![
            "recover",
            "--move-id",
            "00000000-0000-4000-8000-000000000002",
            "--receipt-file",
            path,
        ],
    ] {
        let mut args = vec!["--state-dir", state.to_str().unwrap(), "resume-move"];
        args.extend(command);
        args.extend(["--format", "json"]);
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        assert_ne!(output.code, 0);
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "resume-move-ssh-keygen-unavailable"
        );
        assert!(!state.exists());
    }
}

#[test]
fn destination_plan_validates_checkout_and_writer_absence_without_taking_a_lock() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use agent_session::cli::resume_move::sha256;
    use serde_json::json;

    const THREAD: &str = "00000000-0000-4000-8000-000000000011";
    const MOVE_ID: &str = "00000000-0000-4000-8000-000000000012";
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let cwd = root.join("workspace");
    let other_cwd = root.join("other-workspace");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(&other_cwd).unwrap();
    let state = root.join("state");
    let plan_file = root.join("plan.json");
    let plan = json!({
        "schema_version":"cli.agent-session.resume-move-plan.v1",
        "ok":true,
        "data":{
            "source_host":"host-a","destination_host":"host-b",
            "source_session_id":"source",
            "source_incarnation":"00000000-0000-4000-8000-000000000013",
            "source_generation":1,"provider":"codex","provider_session_id":THREAD,
            "account":"fixture-account","account_binding_revision":1,
            "source_checkout_sha256":sha256(cwd.as_os_str().as_encoded_bytes()),
            "history":{"role":"codex_rollout",
                "relative_path":format!("sessions/2026/01/01/rollout-{THREAD}.jsonl"),
                "source_relative_path":format!("sessions/2026/01/01/rollout-{THREAD}.jsonl"),
                "bytes":40}
        }
    });
    fs::write(&plan_file, plan.to_string()).unwrap();
    fs::set_permissions(&plan_file, fs::Permissions::from_mode(0o600)).unwrap();

    let broker = root.join("broker");
    fs::write(&broker, r#"#!/bin/sh
[ "$1" = list ] || exit 64
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","accounts":[{"account":"fixture-account"}]}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let codex = root.join("codex");
    fs::write(
        &codex,
        r#"#!/bin/sh
case "$*" in
--version) echo 'codex-cli 0.162.1';;
'app-server --help') echo '--listen <URL> unix://PATH';;
*) exit 1;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
    let check = |target: &std::path::Path| {
        run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "resume-move",
                "prepare",
                "--check",
                "--move-id",
                MOVE_ID,
                "--plan-file",
                plan_file.to_str().unwrap(),
                "--cwd",
                target.to_str().unwrap(),
                "--format",
                "json",
            ],
            &CmdOptions::new()
                .with_env_remove_prefix("AGENT_SESSION_")
                .with_env("AGENT_SESSION_RESUME_MOVE_HOST", "host-b")
                .with_env("AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap())
                .with_env(
                    "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
                    &json!([broker]).to_string(),
                )
                .with_env("HOME", root.to_str().unwrap()),
        )
    };

    let admitted = check(&cwd);
    assert_eq!(admitted.code, 0, "{}", admitted.stderr_text());
    let admitted_json = admitted.stdout_json();
    assert_eq!(
        admitted_json["schema_version"],
        "cli.agent-session.resume-move-prepare.v1"
    );
    assert_eq!(admitted_json["data"]["move_id"], MOVE_ID);
    assert_eq!(admitted_json["data"]["state"], "ready");
    assert!(!state.exists());

    let mismatch = check(&other_cwd);
    assert_ne!(mismatch.code, 0);
    assert_eq!(
        mismatch.stdout_json()["error"]["code"],
        "resume-move-path-mismatch"
    );
    assert!(!state.exists());

    let locks = state.join("coordination");
    fs::create_dir_all(&locks).unwrap();
    let writer = locks.join(format!(
        "provider-resume-{}.lock",
        sha256(format!("codex:{THREAD}").as_bytes())
    ));
    fs::write(&writer, "").unwrap();
    let blocked = check(&cwd);
    assert_ne!(blocked.code, 0);
    assert_eq!(
        blocked.stdout_json()["error"]["code"],
        "resume-move-destination-writer-present"
    );
    assert_eq!(fs::read_dir(&locks).unwrap().count(), 1);
    assert!(!state.join("sessions").exists());
}

#[path = "start_resume_move/claude.rs"]
mod claude;
#[path = "start_resume_move/codex.rs"]
mod codex;
#[cfg(target_os = "linux")]
#[path = "start_resume_move/stop.rs"]
mod stop;
#[path = "start_resume_move/support.rs"]
mod support;

#[path = "start_resume_move/console_destination.rs"]
mod console_destination;

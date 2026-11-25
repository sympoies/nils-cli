use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agent_session::cli::resume_move::{
    History, HistoryRole, STOP_PROOF_SCHEMA, SignedRecord, StopProof, Verifier, sha256,
};

use super::support::write_private;
use nils_test_support::cmd::{CmdOptions, run_resolved, run_with};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const THREAD: &str = "00000000-0000-4000-8000-000000000018";
const MOVE_ID: &str = "00000000-0000-4000-8000-000000000019";

#[test]
fn staged_claude_move_creates_one_native_session_after_verified_admission() {
    let ssh_keygen = Path::new("/usr/bin/ssh-keygen");
    if !ssh_keygen.is_file() {
        eprintln!("skipping Claude move fixture: ssh-keygen unavailable");
        return;
    }
    let fixture = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let state = root.join("destination-state");
    let cwd = root.join("workspace");
    let account_root = root.join("account");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(&account_root).unwrap();
    fs::set_permissions(&account_root, fs::Permissions::from_mode(0o700)).unwrap();
    write_private(&account_root.join(".credentials.json"), b"{}");

    let signers = super::support::Signers::new(&root, ssh_keygen);
    let (b_key, allowlist) = (&signers.b_key, &signers.allowlist);
    let broker = root.join("broker");
    let list = json!({
        "schema_version":"agent-session.account-broker.v2",
        "provider":"claude",
        "accounts":[{"account":"fixture-account"}],
    });
    let account = json!({
        "schema_version":"agent-session.account-broker.v2",
        "provider":"claude",
        "account":"fixture-account",
        "config_dir":account_root,
        "history_scope":"account",
    });
    fs::write(&broker, format!(
        "#!/bin/sh\ncase \"$1\" in\nlist) printf '%s\\n' '{list}';;\nhistory-root|materialize) printf '%s\\n' '{account}';;\n*) exit 64;;\nesac\n"
    )).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let (tmux, tmux_log) = super::super::cli::fake_tmux(&root);
    let provider_bin = super::super::cli::fake_agent(&root, "claude");
    let options = CmdOptions::new()
        .with_env_remove_prefix("AGENT_SESSION_")
        .with_env("HOME", root.to_str().unwrap())
        .with_env("AGENT_HOME", root.join("agent-home").to_str().unwrap())
        .with_env("CLAUDE_CONFIG_DIR", account_root.to_str().unwrap())
        .with_env("AGENT_SESSION_RESUME_MOVE_HOST", "host-b")
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN",
            ssh_keygen.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE",
            allowlist.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE",
            b_key.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
            &json!([broker]).to_string(),
        )
        .with_env("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap())
        .with_env("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap())
        .with_env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
        .with_env("AGENT_SESSION_CLAUDE_BIN", provider_bin.to_str().unwrap());
    let run = |args: &[&str]| {
        let mut command = vec!["--state-dir", state.to_str().unwrap()];
        command.extend_from_slice(args);
        command.extend(["--format", "json"]);
        run_resolved("agent-session", &command, &options)
    };

    let relative_path = format!("{THREAD}.jsonl");
    let transcript = format!(
        "{}\n",
        json!({
            "type":"user", "sessionId":THREAD, "cwd":cwd,
            "message":{"role":"user", "content":"synthetic move test"},
        })
    );
    let source_checkout = sha256(cwd.as_os_str().as_encoded_bytes());
    let plan = root.join("plan.json");
    write_private(
        &plan,
        &serde_json::to_vec(&json!({
            "schema_version":"cli.agent-session.resume-move-plan.v1",
            "ok": true, "data":{
                "source_host":"host-a","destination_host":"host-b","source_session_id":"source",
                "source_incarnation":"00000000-0000-4000-8000-000000000020",
                "source_generation":1,"provider":"claude","provider_session_id":THREAD,
                "account":"fixture-account","account_binding_revision":1,
                "source_checkout_sha256":source_checkout,
                "history":{"role":"claude_transcript","relative_path":relative_path,
                           "source_relative_path":format!("projects/workspace/{relative_path}"),
                           "bytes":transcript.len()},
            },
        }))
        .unwrap(),
    );
    let out = run(&[
        "resume-move",
        "prepare",
        "--move-id",
        MOVE_ID,
        "--plan-file",
        plan.to_str().unwrap(),
        "--cwd",
        cwd.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr_text());
    let prepared = out.stdout_json();
    let data = &prepared["data"];
    let staged = Path::new(data["provider_home"].as_str().unwrap()).join(&relative_path);
    let ticket: SignedRecord = serde_json::from_value(data["ticket"].clone()).unwrap();
    let verifier = Verifier::new(ssh_keygen, allowlist).unwrap();
    let verified = verifier.verify_ticket(&ticket, "host-b").unwrap();
    let proof = StopProof {
        schema_version: STOP_PROOF_SCHEMA.into(),
        ticket_sha256: verified.sha256().into(),
        binding: verified.record().binding.clone(),
        source_generation: 1,
        account_binding_revision: 1,
        source_checkout_sha256: source_checkout,
        stopped: true,
        issued_at: verified.record().issued_at,
        expires_at: verified.record().expires_at,
        history: History {
            role: HistoryRole::ClaudeTranscript,
            relative_path,
            bytes: transcript.len() as u64,
            sha256: sha256(transcript.as_bytes()),
        },
    };
    let signed_path = signers.sign_proof(&root, &proof);
    let stage = || {
        run(&[
            "resume-move",
            "stage",
            "--move-id",
            MOVE_ID,
            "--proof-file",
            signed_path.to_str().unwrap(),
        ])
    };
    write_private(
        &staged,
        transcript.replace("synthetic", "different").as_bytes(),
    );
    let tampered = stage();
    assert_ne!(tampered.code, 0);
    assert_eq!(
        tampered.stdout_json()["error"]["code"],
        "resume-move-history-stale"
    );
    assert!(!state.join("sessions").exists());
    write_private(&staged, transcript.as_bytes());
    let staged_result = stage();
    assert_eq!(staged_result.code, 0, "{}", staged_result.stderr_text());

    let start = |account: &str, check: bool| {
        let mut args = vec![
            "start",
            "--agent",
            "claude",
            "--resume",
            THREAD,
            "--move-id",
            MOVE_ID,
            "--account",
            account,
            "--cwd",
            cwd.to_str().unwrap(),
            "--id",
            "moved",
            "--no-parent",
            "--paste-delay-ms",
            "0",
        ];
        if check {
            args.push("--check-resume");
        }
        run(&args)
    };
    // An unusable destination account root refuses before admission, so the
    // move stays staged and no writer or session record is created.
    let failing_broker = root.join("failing-broker");
    fs::write(&failing_broker, "#!/bin/sh\nexit 70\n").unwrap();
    fs::set_permissions(&failing_broker, fs::Permissions::from_mode(0o700)).unwrap();
    let broken = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "start",
            "--agent",
            "claude",
            "--resume",
            THREAD,
            "--move-id",
            MOVE_ID,
            "--account",
            "fixture-account",
            "--cwd",
            cwd.to_str().unwrap(),
            "--id",
            "moved",
            "--no-parent",
            "--format",
            "json",
        ],
        &options.clone().with_env(
            "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
            &json!([failing_broker]).to_string(),
        ),
    );
    assert_ne!(broken.code, 0);
    assert_eq!(broken.stdout_json()["error"]["code"], "resume-unsupported");
    let status = run(&["resume-move", "status", "--move-id", MOVE_ID]);
    assert_eq!(status.stdout_json()["data"]["state"], "staged");
    assert!(!state.join("sessions").exists());
    let mismatch = start("different-account", true);
    assert_eq!(
        mismatch.stdout_json()["error"]["code"],
        "resume-account-mismatch"
    );
    let checked = start("fixture-account", true);
    assert_eq!(checked.code, 0, "{}", checked.stderr_text());
    assert!(!state.join("sessions").exists());
    assert!(!account_root.join("projects").exists());

    let started = start("fixture-account", false);
    assert_eq!(
        started.code,
        0,
        "{} {}",
        started.stdout_text(),
        started.stderr_text()
    );
    assert_eq!(started.stdout_json()["data"]["session"]["id"], "moved");
    let record_path = state.join("sessions/moved/session.json");
    let record: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(record["provider_resume"]["session_id"], THREAD);
    assert_eq!(
        record["claude_account_binding"]["selected_account"],
        "fixture-account"
    );
    let launch: Vec<String> = super::super::cli::tmux_calls(&tmux_log)
        .into_iter()
        .flatten()
        .collect();
    assert!(
        launch.contains(&format!("CLAUDE_CONFIG_DIR={}", account_root.display())),
        "launch must export the materialized destination account directory"
    );
    let mut projects = fs::read_dir(account_root.join("projects")).unwrap();
    let installed = projects
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join(&proof.history.relative_path);
    assert_eq!(fs::read_to_string(installed).unwrap(), transcript);
    assert!(projects.next().is_none());

    let replay = start("fixture-account", false);
    assert_eq!(
        replay.code,
        0,
        "{} {}",
        replay.stdout_text(),
        replay.stderr_text()
    );
    assert_eq!(replay.stdout_json()["data"]["session"]["id"], "moved");
    let replayed: Value = serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(
        replayed["runtime"]["launch_id"],
        record["runtime"]["launch_id"]
    );
    assert!(replayed["runtime"]["launch_id"].is_string());
    assert_eq!(fs::read_dir(state.join("sessions")).unwrap().count(), 1);
    // Stop the fixture-owned pane before TempDir removes its state. The
    // fake pane has no provider process environment for delete to verify.
    let tmux_session = record["tmux_session"].as_str().unwrap();
    let stopped = run_with(&tmux, &["kill-session", "-t", tmux_session], &options);
    assert_eq!(stopped.code, 0, "{}", stopped.stderr_text());
    let gone = run_with(&tmux, &["has-session", "-t", tmux_session], &options);
    assert_eq!(gone.code, 1, "{}", gone.stderr_text());
}

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use agent_session::cli::resume_move::{
    History, HistoryRole, STOP_PROOF_SCHEMA, SignedRecord, StopProof, Verifier, sha256,
};
use nils_test_support::cmd::{CmdOptions, run_resolved, run_with};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use super::support::{Signers, write_private};

const THREAD: &str = "00000000-0000-4000-8000-000000000028";
const MOVE_ID: &str = "00000000-0000-4000-8000-000000000029";

#[test]
fn staged_codex_move_launches_in_the_per_move_home_and_replays_one_session() {
    let ssh_keygen = Path::new("/usr/bin/ssh-keygen");
    if !ssh_keygen.is_file() {
        eprintln!("skipping Codex move fixture: ssh-keygen unavailable");
        return;
    }
    // Short root: the app-server socket path must fit the Unix socket budget.
    let fixture = tempfile::tempdir_in("/tmp").unwrap();
    let root = fs::canonicalize(fixture.path()).unwrap();
    let state = root.join("destination-state");
    let cwd = root.join("workspace");
    let runtime = root.join("run");
    for dir in [&cwd, &runtime] {
        fs::create_dir(dir).unwrap();
    }
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    let signers = Signers::new(&root, ssh_keygen);
    let broker = root.join("broker");
    let list = json!({
        "schema_version":"agent-session.codex-auth-broker.v1",
        "accounts":[{"account":"fixture-account"}],
    });
    fs::write(
        &broker,
        format!("#!/bin/sh\n[ \"$1\" = list ] || exit 64\nprintf '%s\\n' '{list}'\n"),
    )
    .unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let codex = root.join("codex");
    fs::write(
        &codex,
        "#!/bin/sh\ncase \"$*\" in\n--version) echo 'codex-cli 0.162.1';;\n'app-server --help') echo '--listen <URL> unix://PATH';;\n*) sleep 60;;\nesac\n",
    )
    .unwrap();
    fs::set_permissions(&codex, fs::Permissions::from_mode(0o700)).unwrap();
    let (tmux, tmux_log) = super::super::cli::fake_tmux(&root);
    let options = CmdOptions::new()
        .with_env_remove_prefix("AGENT_SESSION_")
        .with_env("HOME", root.to_str().unwrap())
        .with_env("AGENT_HOME", root.join("agent-home").to_str().unwrap())
        .with_env("XDG_RUNTIME_DIR", runtime.to_str().unwrap())
        .with_env("AGENT_SESSION_RESUME_MOVE_HOST", "host-b")
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN",
            ssh_keygen.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE",
            signers.allowlist.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE",
            signers.b_key.to_str().unwrap(),
        )
        .with_env(
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            &json!([broker]).to_string(),
        )
        .with_env("AGENT_SESSION_TMUX_BIN", tmux.to_str().unwrap())
        .with_env("AGENT_SESSION_FAKE_TMUX_LOG", tmux_log.to_str().unwrap())
        .with_env("AGENT_SESSION_FAKE_TMUX_ABSENT_BEFORE_LAUNCH", "1")
        .with_env("AGENT_SESSION_CODEX_BIN", codex.to_str().unwrap());
    let run = |args: &[&str]| {
        let mut command = vec!["--state-dir", state.to_str().unwrap()];
        command.extend_from_slice(args);
        command.extend(["--format", "json"]);
        run_resolved("agent-session", &command, &options)
    };

    let relative_path = format!("sessions/2026/01/01/rollout-{THREAD}.jsonl");
    let rollout = json!({"type":"session_meta","payload":{
        "id":THREAD,"cwd":cwd,"source":"cli",
        "timestamp":"2026-01-01T00:00:00Z"
    }})
    .to_string();
    let source_checkout = sha256(cwd.as_os_str().as_encoded_bytes());
    let plan = root.join("plan.json");
    write_private(
        &plan,
        &serde_json::to_vec(&json!({
            "schema_version":"cli.agent-session.resume-move-plan.v1",
            "ok": true, "data":{
                "source_host":"host-a","destination_host":"host-b","source_session_id":"source",
                "source_incarnation":"00000000-0000-4000-8000-000000000030",
                "source_generation":1,"provider":"codex","provider_session_id":THREAD,
                "account":"fixture-account","account_binding_revision":1,
                "source_checkout_sha256":source_checkout,
                "history":{"role":"codex_rollout","relative_path":relative_path,
                           "source_relative_path":relative_path,"bytes":rollout.len()},
            },
        }))
        .unwrap(),
    );
    let prepared = run(&[
        "resume-move",
        "prepare",
        "--move-id",
        MOVE_ID,
        "--plan-file",
        plan.to_str().unwrap(),
        "--cwd",
        cwd.to_str().unwrap(),
    ]);
    assert_eq!(prepared.code, 0, "{}", prepared.stderr_text());
    let prepared = prepared.stdout_json();
    let data = &prepared["data"];
    let provider_home = Path::new(data["provider_home"].as_str().unwrap()).to_path_buf();
    let staged = provider_home.join(&relative_path);
    fs::create_dir_all(staged.parent().unwrap()).unwrap();
    write_private(&staged, rollout.as_bytes());
    let ticket: SignedRecord = serde_json::from_value(data["ticket"].clone()).unwrap();
    let verifier = Verifier::new(ssh_keygen, &signers.allowlist).unwrap();
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
            role: HistoryRole::CodexRollout,
            relative_path,
            bytes: rollout.len() as u64,
            sha256: sha256(rollout.as_bytes()),
        },
    };
    let signed_path = signers.sign_proof(&root, &proof);
    let staged_result = run(&[
        "resume-move",
        "stage",
        "--move-id",
        MOVE_ID,
        "--proof-file",
        signed_path.to_str().unwrap(),
    ]);
    assert_eq!(staged_result.code, 0, "{}", staged_result.stderr_text());

    let start = || {
        run(&[
            "start",
            "--agent",
            "codex",
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
            "--paste-delay-ms",
            "0",
        ])
    };
    let started = start();
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
        record["provider_resume"]["capture_method"],
        "cross-host-stop-proof-v1"
    );
    let home = provider_home.to_str().unwrap();
    assert_eq!(
        record["runtime"]["agent_profile_provider_config_dir"], home,
        "the session runs in the per-move provider home"
    );
    assert_eq!(fs::read_to_string(&staged).unwrap(), rollout);
    let launch: Vec<String> = super::super::cli::tmux_calls(&tmux_log)
        .into_iter()
        .flatten()
        .collect();
    for assignment in [
        format!("CODEX_HOME={home}"),
        format!("CODEX_SQLITE_HOME={home}"),
    ] {
        assert!(
            launch.contains(&assignment),
            "launch must export {assignment}"
        );
    }

    let replay = start();
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
    // Stop the fixture-owned pane before TempDir removes its state.
    let tmux_session = record["tmux_session"].as_str().unwrap();
    let stopped = run_with(&tmux, &["kill-session", "-t", tmux_session], &options);
    assert_eq!(stopped.code, 0, "{}", stopped.stderr_text());
}

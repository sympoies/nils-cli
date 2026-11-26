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
const SOURCE_INCARNATION: &str = "00000000-0000-4000-8000-000000000020";

#[test]
fn staged_claude_move_creates_one_native_session_after_verified_admission() {
    staged_claude_move(None, None);
}

#[derive(Clone, Copy, Debug)]
enum Failure {
    Materialization,
    BeforePublication,
    PublicationCollision,
    Launch,
}

fn write_stopped_source(root: &Path, state: &Path, cwd: &Path, transcript: &str) {
    let account = root.join("source-account");
    let project = account.join("projects/workspace");
    fs::create_dir_all(&project).unwrap();
    fs::set_permissions(&account, fs::Permissions::from_mode(0o700)).unwrap();
    write_private(&account.join(".credentials.json"), b"{}");
    write_private(
        &project.join(format!("{THREAD}.jsonl")),
        transcript.as_bytes(),
    );
    let session = state.join("sessions/source");
    fs::create_dir_all(&session).unwrap();
    fs::create_dir_all(state.join("session-locks")).unwrap();
    write_private(&state.join("session-locks/source.lock"), b"");
    write_private(
        &session.join("session.json"),
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.session.v1",
            "id": "source", "agent": "claude", "mode": "interactive",
            "cwd": cwd, "tmux_session": "hs-claude-source",
            "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z",
            "provider_resume": {
                "provider": "claude", "session_id": THREAD,
                "captured_at": "2026-01-01T00:00:00Z", "capture_method": "fixture",
                "resume_args": ["--resume", THREAD]
            },
            "runtime": {
                "kind": "tmux", "tmux_session": "hs-claude-source", "generation": 1,
                "started_at": "2026-01-01T00:00:00Z", "launch_id": SOURCE_INCARNATION
            },
            "tmux_runtime_never_launched": SOURCE_INCARNATION,
            "claude_account_binding": {
                "schema_version": "agent-session.claude-account-binding.v1",
                "selected_account": "fixture-account", "revision": 1, "state": "bound",
                "applied_runtime_id": SOURCE_INCARNATION, "config_dir": account,
                "updated_at": "2026-01-01T00:00:00Z"
            }
        }))
        .unwrap(),
    );
}

#[test]
fn claude_move_recovers_after_writer_free_preparation_failure() {
    staged_claude_move(Some(Failure::Materialization), None);
    staged_claude_move(Some(Failure::BeforePublication), None);
}

#[test]
fn claude_move_keeps_uncertain_publication_and_launch_fenced() {
    staged_claude_move(Some(Failure::PublicationCollision), None);
    staged_claude_move(Some(Failure::Launch), None);
}

pub(super) fn with_console_fixture(test: fn(super::console_destination::Fixture<'_>)) {
    staged_claude_move(None, Some(test));
}

fn staged_claude_move(
    failure: Option<Failure>,
    console: Option<fn(super::console_destination::Fixture<'_>)>,
) {
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
    let fault = root.join("materialize-fixture");
    let fault_path = shell_words::quote(fault.to_str().unwrap());
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
        "#!/bin/sh\ncase \"$1\" in\nlist) printf '%s\\n' '{list}';;\nhistory-root) printf '%s\\n' '{account}';;\nmaterialize)\nif [ -f {fault_path} ]; then /bin/sh {fault_path} || exit $?; fi\nprintf '%s\\n' '{account}';;\n*) exit 64;;\nesac\n"
    )).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let (tmux, tmux_log) = super::super::cli::fake_tmux(&root);
    let provider_bin = super::super::cli::fake_agent(&root, "claude");
    let mut options = CmdOptions::new()
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
    if matches!(failure, Some(Failure::Launch)) {
        options = options.with_env("AGENT_SESSION_FAKE_TMUX_FAIL", "new-session");
    }
    let run = |args: &[&str]| {
        let mut command = vec!["--state-dir", state.to_str().unwrap()];
        command.extend_from_slice(args);
        command.extend(["--format", "json"]);
        run_resolved("agent-session", &command, &options)
    };

    let relative_path = format!("{THREAD}.jsonl");
    // Claude records the shell's current directory on each row, so a session
    // that changed into a subdirectory of the checkout has several distinct cwds.
    let row = |directory: &Path| {
        json!({
            "type":"user", "sessionId":THREAD, "cwd":directory,
            "message":{"role":"user", "content":"synthetic move test"},
        })
    };
    let transcript = format!("{}\n{}\n", row(&cwd), row(&cwd.join("subdirectory")));
    let source_checkout = sha256(cwd.as_os_str().as_encoded_bytes());
    let plan = root.join("plan.json");
    write_private(
        &plan,
        &serde_json::to_vec(&json!({
            "schema_version":"cli.agent-session.resume-move-plan.v1",
            "ok": true, "data":{
                "source_host":"host-a","destination_host":"host-b","source_session_id":"source",
                "source_incarnation":SOURCE_INCARNATION,
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
    let source_state = root.join("source-state");
    let source_options = options
        .clone()
        .with_env("AGENT_SESSION_RESUME_MOVE_HOST", "host-a")
        .with_env(
            "AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE",
            signers.a_key.to_str().unwrap(),
        );
    let run_source = |args: &[&str]| {
        let mut command = vec!["--state-dir", source_state.to_str().unwrap()];
        command.extend_from_slice(args);
        command.extend(["--format", "json"]);
        run_resolved("agent-session", &command, &source_options)
    };
    let signed_path = if matches!(failure, Some(Failure::Materialization)) {
        write_stopped_source(&root, &source_state, &cwd, &transcript);
        let ticket_path = root.join("ticket.json");
        write_private(&ticket_path, &serde_json::to_vec(&ticket).unwrap());
        let exported = run_source(&[
            "resume-move",
            "export",
            "--session",
            "source",
            "--incarnation",
            SOURCE_INCARNATION,
            "--destination",
            "host-b",
            "--ticket-file",
            ticket_path.to_str().unwrap(),
        ]);
        assert_eq!(exported.code, 0, "{}", exported.stdout_text());
        let path = root.join("proof.json");
        write_private(
            &path,
            &serde_json::to_vec(&exported.stdout_json()["data"]).unwrap(),
        );
        path
    } else {
        signers.sign_proof(&root, &proof)
    };
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

    if let Some(test) = console {
        test(super::console_destination::Fixture {
            root: &root,
            state: &state,
            cwd: &cwd,
            tmux: &tmux,
            tmux_log: &tmux_log,
            broker: &broker,
            staged_history: &staged,
            provider_bin: &provider_bin,
            signers: &signers,
            proof: &proof,
            ssh_keygen,
        });
        return;
    }

    let start_as = |id: &str, account: &str, check: bool| {
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
            id,
            "--no-parent",
            "--paste-delay-ms",
            "0",
        ];
        if check {
            args.push("--check-resume");
        }
        run(&args)
    };
    let start = |account: &str, check: bool| start_as("moved", account, check);
    if let Some(failure) = failure {
        let quote = |path: &Path| shell_words::quote(path.to_str().unwrap()).into_owned();
        let store = state.join("coordination/resume-move-destination.v1.json");
        let admitted = root.join("materialization-admitted");
        let collision = account_root
            .join("projects")
            .join(
                cwd.to_str()
                    .unwrap()
                    .chars()
                    .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
                    .collect::<String>(),
            )
            .join(format!("{THREAD}.jsonl"));
        let action = match failure {
            Failure::Materialization => "exit 70".to_owned(),
            Failure::BeforePublication => format!(
                "mv -- {} {}",
                quote(&staged),
                quote(&root.join("unavailable-staged-history"))
            ),
            Failure::PublicationCollision => format!(
                "mkdir -p {} && printf '%s\\n' '{}' > {}",
                quote(collision.parent().unwrap()),
                "unrelated destination history",
                quote(&collision),
            ),
            Failure::Launch => ":".to_owned(),
        };
        fs::write(
            &fault,
            format!(
                "cp {} {} || exit 67\n{action}\n",
                quote(&store),
                quote(&admitted)
            ),
        )
        .unwrap();

        let failed = start("fixture-account", false);
        assert_ne!(failed.code, 0, "{failure:?}");
        let at_materialization: Value =
            serde_json::from_slice(&fs::read(&admitted).unwrap()).unwrap();
        assert_eq!(at_materialization[MOVE_ID]["outcome"], "writer_admitted");
        let launched = || {
            tmux_log.is_file()
                && super::super::cli::tmux_calls(&tmux_log)
                    .iter()
                    .any(|call| call.first().is_some_and(|arg| arg == "new-session"))
        };
        assert_eq!(
            launched(),
            matches!(failure, Failure::Launch),
            "{failure:?}"
        );
        let writer_free = matches!(
            failure,
            Failure::Materialization | Failure::BeforePublication
        );
        let status = run(&["resume-move", "status", "--move-id", MOVE_ID]);
        assert_eq!(
            status.stdout_json()["data"]["state"],
            if writer_free {
                "aborted_no_writer"
            } else {
                "writer_admitted"
            },
            "{failure:?}"
        );
        let expected_refusal = if writer_free {
            assert!(!state.join("sessions/moved").exists());
            "resume-move-aborted"
        } else {
            "resume-move-outcome-unknown"
        };
        assert_eq!(failed.stdout_json()["error"]["code"], expected_refusal);
        let replay = start("fixture-account", false);
        assert_eq!(replay.stdout_json()["error"]["code"], expected_refusal);
        let abort = run(&["resume-move", "abort", "--move-id", MOVE_ID]);
        if !writer_free {
            assert_eq!(
                abort.stdout_json()["error"]["code"],
                "resume-move-destination-writer-present"
            );
            if matches!(failure, Failure::PublicationCollision) {
                assert_eq!(
                    fs::read_to_string(collision).unwrap(),
                    "unrelated destination history\n"
                );
            }
            return;
        }
        assert_eq!(abort.code, 0, "{}", abort.stdout_text());
        let receipt: SignedRecord =
            serde_json::from_value(abort.stdout_json()["data"].clone()).unwrap();
        let exact_proof: SignedRecord =
            serde_json::from_slice(&fs::read(&signed_path).unwrap()).unwrap();
        let exact_proof = verifier.verify_stop_proof(&exact_proof, "host-a").unwrap();
        verifier
            .verify_no_writer_receipt(&receipt, "host-b")
            .unwrap()
            .check(&exact_proof)
            .unwrap();
        if matches!(failure, Failure::Materialization) {
            let recovered = || {
                let ledger: Value = serde_json::from_slice(
                    &fs::read(source_state.join("coordination/resume-move-source.v1.json"))
                        .unwrap(),
                )
                .unwrap();
                ledger[MOVE_ID]["recovered"].clone()
            };
            let recover = |receipt_path: &Path| {
                run_source(&[
                    "resume-move",
                    "recover",
                    "--move-id",
                    MOVE_ID,
                    "--receipt-file",
                    receipt_path.to_str().unwrap(),
                ])
            };
            assert_eq!(recovered(), false);
            assert_ne!(recover(&signed_path).code, 0);
            assert_eq!(recovered(), false);
            let receipt_path = root.join("receipt.json");
            write_private(&receipt_path, &serde_json::to_vec(&receipt).unwrap());
            let restored = recover(&receipt_path);
            assert_eq!(restored.code, 0, "{}", restored.stdout_text());
            assert_eq!(recovered(), true);
        }
        assert!(!launched());
        return;
    }
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

    // An existing session ID and a transcript for the conversation in another
    // project directory both refuse before admission: the move stays staged.
    let status = || {
        run(&["resume-move", "status", "--move-id", MOVE_ID]).stdout_json()["data"]["state"].clone()
    };
    let taken_dir = state.join("sessions/existing");
    fs::create_dir_all(&taken_dir).unwrap();
    for check in [true, false] {
        let taken = start_as("existing", "fixture-account", check);
        assert_ne!(taken.code, 0);
        assert_eq!(taken.stdout_json()["error"]["code"], "session-exists");
        assert_eq!(status(), "staged");
    }
    fs::remove_dir(&taken_dir).unwrap();
    let other_project = account_root.join("projects/unrelated-project");
    fs::create_dir_all(&other_project).unwrap();
    write_private(&other_project.join(format!("{THREAD}.jsonl")), b"{}\n");
    for check in [true, false] {
        let elsewhere = start_as("moved", "fixture-account", check);
        assert_ne!(elsewhere.code, 0);
        assert_eq!(
            elsewhere.stdout_json()["error"]["code"],
            "resume-move-destination-writer-present"
        );
        assert_eq!(status(), "staged");
    }
    assert!(!state.join("sessions/moved").exists());
    fs::remove_dir_all(account_root.join("projects")).unwrap();

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

use std::fs;
use std::os::unix::fs::PermissionsExt;

use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::cli::resume_move::source_fence::{self, Export};
use crate::{AgentKind, CliContext};

use super::{
    AbortVerdict, Fixture, NO_WRITER_SCHEMA, NoWriterReceipt, Provider, Refusal, SignedRecord,
    TICKET_SCHEMA, Ticket, sha256, sign,
};

const SOURCE_HISTORY: &str = "sessions/2026/01/01";

#[test]
fn source_export_holds_the_writer_fence_until_exact_destination_abort() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let root = fs::canonicalize(fixture.directory.path()).unwrap();
    let context = CliContext {
        state_dir: root.join("state"),
        host: Some("host-a".into()),
    };
    let config = root.join("codex");
    let binding = &fixture.binding;
    let history = config
        .join(SOURCE_HISTORY)
        .join(format!("rollout-{}.jsonl", binding.provider_session_id));
    fs::create_dir_all(history.parent().unwrap()).unwrap();
    fs::write(
        &history,
        json!({"type":"session_meta","payload":{
            "id":binding.provider_session_id,"cwd":root,"source":"cli",
            "timestamp":"2026-01-01T00:00:00Z","history_mode":"legacy"
        }})
        .to_string(),
    )
    .unwrap();

    let record_path = context.state_dir.join("sessions/source/session.json");
    fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    let lock_path = context.state_dir.join("session-locks/source.lock");
    fs::create_dir_all(lock_path.parent().unwrap()).unwrap();
    fs::write(lock_path, "").unwrap();
    let record = json!({
        "schema_version":"agent-session.session.v1", "id":"source", "agent":"codex",
        "mode":"interactive", "cwd":root, "tmux_session":"hs-codex-source",
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
        "provider_resume":{"provider":"codex","session_id":binding.provider_session_id,
            "captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture",
            "resume_args":["resume",binding.provider_session_id]},
        "runtime":{"kind":"tmux","tmux_session":"hs-codex-source","generation":1,
            "started_at":"2026-01-01T00:00:00Z","launch_id":binding.source_incarnation,
            "agent_profile_provider_config_dir":config},
        "tmux_runtime_never_launched":binding.source_incarnation,
        "codex_account_binding":{"schema_version":"agent-session.codex-account-binding.v1",
            "selected_account":binding.account,"revision":1,"state":"bound",
            "applied_runtime_id":binding.source_incarnation,"updated_at":"2026-01-01T00:00:00Z"}
    });
    fs::write(&record_path, record.to_string()).unwrap();
    let before = fs::read(&record_path).unwrap();
    let tmux = root.join("tmux");
    fs::write(&tmux, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();

    let now = jiff::Timestamp::now().as_second();
    let ticket = sign(
        &fixture.directory,
        "host-b",
        TICKET_SCHEMA,
        &Ticket {
            schema_version: TICKET_SCHEMA.into(),
            binding: binding.clone(),
            issued_at: now,
            expires_at: now + 300,
        },
    );
    let key = root.join("host-a");
    let export = || {
        source_fence::export(
            &context,
            &fixture.verifier,
            Export {
                session: &binding.source_session_id,
                incarnation: &binding.source_incarnation,
                source_host: &binding.source_host,
                destination: &binding.destination_host,
                ticket: &ticket,
                signing_key: &key,
            },
            &tmux,
        )
    };
    assert_eq!(export().err().unwrap().code(), Refusal::SourceLive.code());
    assert!(
        !context
            .state_dir
            .join("coordination/resume-move-source.v1.json")
            .exists()
    );
    fs::write(
        &tmux,
        "#!/bin/sh\nprintf 'no server running on fixture\\n' >&2\nexit 1\n",
    )
    .unwrap();

    let signed = export().unwrap();
    let proof = fixture
        .verifier
        .verify_stop_proof(&signed, "host-a")
        .unwrap();
    assert_eq!(
        proof.record().history.sha256,
        sha256(&fs::read(&history).unwrap())
    );
    assert_eq!(export().unwrap().payload, signed.payload);

    let ensure_unfenced =
        || source_fence::ensure_unfenced(&context, AgentKind::Codex, &binding.provider_session_id);
    assert_eq!(
        ensure_unfenced().unwrap_err().code(),
        Refusal::SourceFenced.code()
    );

    let options = CmdOptions::new()
        .without_ambient_managed_session_env()
        .with_cwd(&root)
        .with_env_os("HOME".as_ref(), root.as_os_str())
        .with_env_os("AGENT_HOME".as_ref(), root.join("agent-home").as_os_str())
        .with_env_os("CODEX_HOME".as_ref(), config.as_os_str());
    for check in [true, false] {
        let mut args = vec![
            "--state-dir",
            context.state_dir.to_str().unwrap(),
            "start",
            "--agent",
            "codex",
            "--resume",
            &binding.provider_session_id,
            "--account",
            &binding.account,
            "--id",
            "replacement",
            "--no-parent",
            "--tmux-bin",
            tmux.to_str().unwrap(),
            "--agent-bin",
            tmux.to_str().unwrap(),
            "--format",
            "json",
        ];
        if check {
            args.push("--check-resume");
        }
        let outcome = run_resolved("agent-session", &args, &options);
        assert_ne!(outcome.code, 0);
        let response = outcome.stdout_json();
        assert_eq!(response["error"]["code"], Refusal::SourceFenced.code());
        assert_eq!(response["error"]["details"]["session_created"], false);
        assert!(!context.state_dir.join("sessions/replacement").exists());
        assert!(
            !context
                .state_dir
                .join("session-locks/replacement.lock")
                .exists()
        );
    }

    let abort = |sha256: &str| {
        sign(
            &fixture.directory,
            "host-b",
            NO_WRITER_SCHEMA,
            &NoWriterReceipt {
                schema_version: NO_WRITER_SCHEMA.into(),
                binding: binding.clone(),
                stop_proof_sha256: sha256.into(),
                verdict: AbortVerdict::AbortedNoWriter,
            },
        )
    };
    let recover = |receipt: &SignedRecord| {
        source_fence::recover(
            &context,
            &fixture.verifier,
            &binding.move_id,
            &binding.source_host,
            receipt,
        )
    };

    let wrong_receipt = abort(&sha256(b"wrong export"));
    assert_eq!(
        recover(&wrong_receipt).unwrap_err().code(),
        Refusal::MoveMismatch.code()
    );
    assert!(ensure_unfenced().is_err());

    let receipt = abort(proof.sha256());
    let original = fs::read(&history).unwrap();
    let new_event = b"\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\"}}\n";
    fs::write(
        &history,
        [original.as_slice(), new_event.as_slice()].concat(),
    )
    .unwrap();
    assert_eq!(
        recover(&receipt).unwrap_err().code(),
        Refusal::HistoryStale.code()
    );
    assert_eq!(
        ensure_unfenced().unwrap_err().code(),
        Refusal::SourceFenced.code()
    );
    fs::write(&history, original).unwrap();

    recover(&receipt).unwrap();
    recover(&receipt).unwrap();
    ensure_unfenced().unwrap();
    assert_eq!(export().err().unwrap().code(), Refusal::MoveMismatch.code());
    assert_eq!(fs::read(&record_path).unwrap(), before);
}

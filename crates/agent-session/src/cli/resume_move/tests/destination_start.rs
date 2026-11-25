use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use clap::Parser;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;

use crate::cli::resume_move;
use crate::cli::resume_move::destination::{self, Admission};
use crate::{AgentKind, cli, start_resume};

use super::destination_work::Staged;
use super::{Fixture, Provider, Refusal, SSH_KEYGEN};

const SESSION: &str = "move-fixture";
const INCARNATION: &str = "00000000-0000-4000-8000-0000000000aa";

fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn status(staged: &Staged) -> String {
    let view = destination::status(&staged.context, &staged.binding.move_id).unwrap();
    view["state"].as_str().unwrap().to_owned()
}

#[test]
fn unready_runtime_refuses_before_writer_admission_and_abort_still_issues_a_receipt() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let staged = Staged::prepare(&fixture, "destination");
    staged.stage(&fixture);
    let root = fs::canonicalize(fixture.directory.path()).unwrap();
    let codex = root.join("codex");
    executable(
        &codex,
        "#!/bin/sh\ncase \"$*\" in\n--version) echo 'codex-cli 0.162.1';;\n'app-server --help') echo '--listen <URL> unix://PATH';;\n*) exit 1;;\nesac\n",
    );
    let broker = root.join("broker");
    executable(
        &broker,
        "#!/bin/sh\n[ \"$1\" = list ] || exit 64\nprintf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"accounts\":[{\"account\":\"fixture-account\"}]}'\n",
    );
    // A private runtime root whose socket path can never fit the Unix socket budget.
    let runtime = root.join("r".repeat(120));
    fs::create_dir(&runtime).unwrap();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();

    let lock = GlobalStateLock::new();
    let _guards = [
        EnvGuard::set(&lock, "AGENT_SESSION_RESUME_MOVE_HOST", "host-b"),
        EnvGuard::set(&lock, "AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN", SSH_KEYGEN),
        EnvGuard::set(
            &lock,
            "AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE",
            root.join("allowed-signers").to_str().unwrap(),
        ),
        EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            &json!([broker]).to_string(),
        ),
        EnvGuard::set(&lock, "XDG_RUNTIME_DIR", runtime.to_str().unwrap()),
        EnvGuard::set(&lock, "HOME", root.to_str().unwrap()),
    ];
    let cli::Command::Start(args) = cli::Cli::try_parse_from([
        "agent-session",
        "start",
        "--agent",
        "codex",
        "--account",
        &staged.binding.account,
        "--resume",
        &staged.binding.provider_session_id,
        "--move-id",
        &staged.binding.move_id,
        "--cwd",
        staged.cwd.to_str().unwrap(),
        "--agent-bin",
        codex.to_str().unwrap(),
    ])
    .unwrap()
    .command
    else {
        panic!("start command");
    };

    let error = resume_move::start(&staged.context, *args).err().unwrap();
    assert_eq!(error.code(), "resume-unsupported");
    assert_eq!(status(&staged), "staged");
    assert!(!staged.writer_path().exists());
    assert!(!staged.context.state_dir.join("sessions").exists());
    let receipt = destination::abort(
        &staged.context,
        &staged.binding.move_id,
        &fixture.verifier,
        &staged.destination_key,
        &staged.binding.destination_host,
        None,
    )
    .unwrap();
    fixture
        .verifier
        .verify_no_writer_receipt(&receipt, "host-b")
        .unwrap();
}

#[test]
fn admitted_move_is_outcome_unknown_and_refuses_abort() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let staged = Staged::prepare(&fixture, "destination");
    staged.stage(&fixture);
    let now = staged.now;
    let admitted = destination::admit(&staged.context, &fixture.verifier, staged.request(), now)
        .ok()
        .unwrap();
    assert!(matches!(admitted, Admission::New { .. }));
    drop(admitted);
    assert_eq!(status(&staged), "writer_admitted");

    for error in [
        destination::admit(&staged.context, &fixture.verifier, staged.request(), now)
            .err()
            .unwrap(),
        destination::mark_created(
            &staged.context,
            &staged.binding.move_id,
            SESSION,
            "not-a-uuid",
        )
        .err()
        .unwrap(),
    ] {
        assert_eq!(error.code(), "resume-move-outcome-unknown");
    }
    assert_eq!(status(&staged), "writer_admitted");
    let refused = destination::abort(
        &staged.context,
        &staged.binding.move_id,
        &fixture.verifier,
        &staged.destination_key,
        &staged.binding.destination_host,
        None,
    )
    .err()
    .unwrap();
    assert_eq!(refused.code(), Refusal::DestinationWriterPresent.code());
}

#[test]
fn created_move_replays_the_same_session() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let staged = Staged::prepare(&fixture, "destination");
    staged.stage(&fixture);
    let now = staged.now;
    let id = &staged.binding.move_id;
    assert_eq!(
        destination::mark_created(&staged.context, id, SESSION, INCARNATION)
            .err()
            .unwrap()
            .code(),
        "resume-move-outcome-unknown",
        "only a writer-admitted move may record creation"
    );
    drop(
        destination::admit(&staged.context, &fixture.verifier, staged.request(), now)
            .ok()
            .unwrap(),
    );
    destination::mark_created(&staged.context, id, SESSION, INCARNATION).unwrap();
    assert_eq!(status(&staged), "created");

    let Admission::Existing {
        session_id,
        incarnation,
    } = destination::admit(&staged.context, &fixture.verifier, staged.request(), now)
        .ok()
        .unwrap()
    else {
        panic!("a created move replays its existing session");
    };
    assert_eq!(
        (session_id.as_str(), incarnation.as_str()),
        (SESSION, INCARNATION)
    );
    assert_eq!(
        destination::mark_created(&staged.context, id, "other", INCARNATION)
            .err()
            .unwrap()
            .code(),
        "resume-move-outcome-unknown"
    );
}

#[test]
fn prepared_and_staged_moves_reserve_the_conversation_from_ordinary_start() {
    let Some(fixture) = Fixture::new(Provider::Codex) else {
        return;
    };
    let staged = Staged::prepare(&fixture, "destination");
    let thread = &staged.binding.provider_session_id;
    let refuse = || {
        let error = start_resume::raw_conversation_lock(&staged.context, AgentKind::Codex, thread)
            .err()
            .unwrap();
        assert_eq!(error.code(), "resume-move-destination-reserved");
        assert!(!staged.writer_path().exists());
        assert!(!staged.context.state_dir.join("sessions").exists());
    };
    refuse();
    staged.stage(&fixture);
    refuse();
}

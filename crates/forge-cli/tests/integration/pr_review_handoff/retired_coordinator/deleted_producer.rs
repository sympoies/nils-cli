//! Current binary producer output crosses the recovery boundary unchanged.
use super::*;
use pretty_assertions::{assert_eq, assert_ne};
use std::os::unix::fs::PermissionsExt;

#[test]
fn current_deleted_producer_recovers_after_staleness_and_reassigns_at_exact_tip() {
    let producer = nils_test_support::bin::required_sibling("agent-session", "nils-agent-session");
    let seeded = records(retired_coordinator_handoff(), Some(HEAD));
    let tip = &seeded.last().unwrap().record_digest;
    let stub = writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded);
    let state = stub.tempdir.path().join("producer-state");
    let session_dir = state.join("sessions").join(COORDINATOR_UUID);
    fs::create_dir_all(&session_dir).unwrap();
    fs::set_permissions(&session_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let now = jiff::Timestamp::now().to_string();
    // Synthetic never-launched state follows the existing board delete fixture.
    // Only the real delete command produces retirement evidence and timestamps.
    fs::write(
        session_dir.join("session.json"),
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.session.v1", "id":COORDINATOR_UUID,
            "agent":"claude", "mode":"interactive", "title":"Fixture coordinator",
            "cwd":stub.tempdir.path(), "tmux_session":"fixture-coordinator",
            "prompt_file":stub.tempdir.path().join("prompt.txt"),
            "log_file":stub.tempdir.path().join("session.log"),
            "created_at":now, "updated_at":now,
            "runtime":{"kind":"tmux", "tmux_session":"fixture-coordinator",
                "generation":1, "started_at":now, "launch_id":"never-launched-fixture"},
            "tmux_runtime_never_launched":"never-launched-fixture"
        }))
        .unwrap(),
    )
    .unwrap();
    let tmux = stub.write_stub(
        "tmux",
        "#!/bin/sh\necho 'no server running on fixture' >&2\nexit 1\n",
    );
    let mut delete = Command::new(&producer);
    let _home = nils_test_support::hermetic::isolate(&mut delete);
    let before = jiff::Timestamp::now();
    let deleted = delete
        .args(["--state-dir"])
        .arg(&state)
        .args(["delete", COORDINATOR_UUID, "--format", "json"])
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .current_dir(stub.tempdir.path())
        .output()
        .unwrap();
    assert!(deleted.status.success(), "{deleted:?}");
    assert!(!session_dir.exists());
    let ledger: Value =
        serde_json::from_slice(&fs::read(state.join("board/closed-ledger.json")).unwrap()).unwrap();
    let retained = &ledger["entries"][0]["record"];
    assert_eq!(retained["state"], "closed");
    assert_eq!(retained["close_reason"], "deleted");
    let activity: jiff::Timestamp = retained["last_terminal_activity_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(activity >= before && activity <= jiff::Timestamp::now());
    let mut board = Command::new(&producer);
    let _board_home = nils_test_support::hermetic::isolate(&mut board);
    let produced = board
        .arg("--state-dir")
        .arg(&state)
        .args(["board", "--state", "all", "--format", "json"])
        .env("AGENT_SESSION_MACHINE", "source")
        .env("AGENT_SESSION_TMUX_BIN", &tmux)
        .output()
        .unwrap();
    assert!(produced.status.success(), "{produced:?}");
    let produced: Value = serde_json::from_slice(&produced.stdout).unwrap();
    let produced_row = &produced["data"]["board"]["records"][0];
    assert_eq!(
        produced_row["last_terminal_activity_at"],
        retained["last_terminal_activity_at"]
    );
    let board_output = stub.tempdir.path().join("producer-board.json");
    let adapter = stub.write_stub("producer-board", r#"#!/bin/sh
unset AGENT_SESSION_ID AGENT_SESSION_RUNTIME_ID AGENT_SESSION_CAPABILITY_FILE AGENT_SESSION_CHECKPOINT_FILE
"$DELETED_PRODUCER_BIN" --state-dir "$DELETED_PRODUCER_STATE" "$@" > "$DELETED_PRODUCER_BOARD"
result=$?
cat "$DELETED_PRODUCER_BOARD"
exit "$result"
"#);
    let stub = stub
        .env("FORGE_CLI_AGENT_SESSION_BIN", adapter.to_string_lossy())
        .env("DELETED_PRODUCER_BIN", producer.to_string_lossy())
        .env("DELETED_PRODUCER_STATE", state.to_string_lossy())
        .env("DELETED_PRODUCER_BOARD", board_output.to_string_lossy())
        .env("AGENT_SESSION_TMUX_BIN", tmux.to_string_lossy())
        .env("AGENT_SESSION_MACHINE", "source")
        .env("AGENT_SESSION_ID", "successor-session")
        .env("AGENT_REVIEWER_SESSION", "");
    let ledger_path = stub.tempdir.path().join("ledger.json");
    let unchanged = fs::read(&ledger_path).unwrap();
    let recover_args = [
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
        "pr",
        "review-handoff",
        "recover",
        "7",
        "--expected-head",
        HEAD,
        "--expected-state",
        tip,
        "--base-sha",
        OLD,
        "--reason",
        "coordinator-retired",
    ];
    let fresh = run_with_supervisor_policy(&stub, &recover_args, true, "origin", "60s");
    assert_refusal(&fresh, 65, "review_coordinator_unproven");
    assert_eq!(
        parse_envelope(&fresh.stdout)["error"]["details"]["prerequisite"],
        "coordinator_not_stale"
    );
    assert_eq!(fs::read(&ledger_path).unwrap(), unchanged);
    let board: Value = serde_json::from_slice(&fs::read(&board_output).unwrap()).unwrap();
    assert_eq!(&board["data"]["board"]["records"][0], produced_row);

    // Use the minimum supported policy and real elapsed time, not a rewritten
    // terminal timestamp or a production clock/trust override. The extra second
    // clears the consumer's whole-second boundary deterministically.
    std::thread::sleep(std::time::Duration::from_secs(61));
    assert!(jiff::Timestamp::now().as_second() - activity.as_second() >= 60);
    let repo = stub.tempdir.path().join("repo");
    let base = nils_test_support::git::git(&repo, &["rev-parse", "HEAD"]);
    let base = base.trim();
    let stub = stub
        .env("PROVIDER_BASE", base)
        .env("PROVIDER_BASE_REF", "main");
    let args: Vec<_> = recover_args
        .iter()
        .map(|arg| if *arg == OLD { base } else { *arg })
        .collect();
    let recovered = super::super::super::support::run_forge_cli_in(&stub, &args, Some(&repo));
    assert_eq!(
        recovered.code, 0,
        "{} {}",
        recovered.stdout, recovered.stderr
    );
    let recovered = parse_envelope(&recovered.stdout)["data"].clone();
    let recovered_tip = recovered["state_tip_digest"].as_str().unwrap();
    assert_ne!(recovered_tip, tip);
    assert_eq!(recovered["handoff"]["assignment_generation"], 2);
    assert_eq!(
        recovered["handoff"]["coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    assert_eq!(
        recovered["handoff"]["reviewer_digest"],
        review_state::sha256_digest(b"reviewer-session")
    );
    assert_eq!(recovered["handoff"]["assigned_head"], HEAD);
    assert_eq!(recovered["handoff"]["base_sha"], base);
    assert_eq!(
        recovered["handoff"]["coordinator_transfer"]["retirement_evidence"]["last_activity_at"],
        retained["last_terminal_activity_at"]
    );
    let board: Value = serde_json::from_slice(&fs::read(&board_output).unwrap()).unwrap();
    assert_eq!(&board["data"]["board"]["records"][0], produced_row);
    let still_retained: Value =
        serde_json::from_slice(&fs::read(state.join("board/closed-ledger.json")).unwrap()).unwrap();
    assert_eq!(still_retained, ledger);
    let saved: Vec<String> = serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), seeded.len() + 1);
    assert_eq!(chain.records.last().unwrap().record_digest, recovered_tip);
    let ReviewStatePayload::ReviewLoop { state: original } = &seeded[1].payload else {
        panic!("seeded review loop");
    };
    assert_eq!(
        review_state::latest_review_loop_state(&chain),
        Some(original)
    );

    let assigned = assign_handoff(&stub, REVIEWER_UUID, base, recovered_tip, false);
    assert_eq!(assigned.code, 0, "{} {}", assigned.stdout, assigned.stderr);
    let assigned = parse_envelope(&assigned.stdout)["data"].clone();
    let inspected = inspect_handoff(&stub);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    let inspected = parse_envelope(&inspected.stdout)["data"].clone();
    assert_eq!(inspected["state_tip_digest"], assigned["state_tip_digest"]);
    assert_ne!(inspected["state_tip_digest"], recovered_tip);
    assert_eq!(inspected["handoff"]["assignment_generation"], 3);
    assert_eq!(inspected["handoff"]["assigned_head"], HEAD);
    assert_eq!(inspected["handoff"]["base_sha"], base);
    assert_eq!(
        inspected["handoff"]["reviewer_digest"],
        review_state::sha256_digest(REVIEWER_UUID.as_bytes())
    );
    assert_eq!(
        inspected["handoff"]["coordinator_digest"],
        review_state::sha256_digest(b"successor-session")
    );
    let saved: Vec<String> = serde_json::from_slice(&fs::read(&ledger_path).unwrap()).unwrap();
    let chain =
        review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
    assert_eq!(chain.records.len(), seeded.len() + 2);
    assert_eq!(
        chain.records.last().unwrap().record_digest,
        inspected["state_tip_digest"]
    );
}

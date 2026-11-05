//! Role supervisors require operator provenance bound to the acting session.
use super::*;
use pretty_assertions::assert_eq;

fn binding(actor: &str, attested_role: Option<&str>) -> Value {
    json!({"schema_version":"agent-session.forge-binding.v1",
        "session_id":actor,"session_incarnation":"runtime-one",
        "session_created_at":"2026-07-20T10:00:00Z",
        "root":{"machine":"source","session_id":actor,
            "session_created_at":"2026-07-20T10:00:00Z"},
        "parent":null,"initiator":"operator","role":attested_role.unwrap_or("steward"),
        "attested_role":attested_role})
}

fn broker(stub: StubEnv, data: Value) -> StubEnv {
    let envelope =
        json!({"ok":true,"schema_version":"cli.agent-session.broker-identity.v1","data":data});
    let path = stub.write_stub(
        "identity-broker",
        &format!("#!/bin/sh\ncat <<'JSON'\n{envelope}\nJSON\n"),
    );
    stub.env("FORGE_IDENTITY_AGENT_SESSION_BIN", path.to_string_lossy())
        .env("AGENT_SESSION_RUNTIME_ID", "runtime-one")
}

fn recover(stub: &StubEnv, tip: &str, dry: bool) -> super::super::super::support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry {
        args.push("--dry-run");
    }
    args.extend([
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
    ]);
    run_with_supervisor_entries(stub, &args, r#"["role:steward"]"#, "origin", "2h")
}

#[test]
fn attested_supervisor_role_survives_replacement_and_retains_digest_proof() {
    let seeded = records(retired_coordinator_handoff(), None);
    for actor in ["successor-one", "successor-two"] {
        let stub = coordinator_board(
            writable_ledger(fixture(HEAD, &seeded, vec![]), &seeded),
            "stopped",
            "",
        )
        .env("AGENT_SESSION_ID", actor);
        let authenticated = binding(actor, Some("steward"));
        let stub = broker(stub, authenticated.clone());
        let out = recover(&stub, &seeded[0].record_digest, false);
        assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
        let data = parse_envelope(&out.stdout);
        let transfer = &data["data"]["handoff"]["coordinator_transfer"];
        assert_eq!(
            transfer["supervisor_digest"],
            review_state::sha256_digest(actor.as_bytes())
        );
        assert_eq!(transfer["supervisor_attestation"]["role"], "steward");
        let typed: nils_common::forge_identity::session::SessionBinding =
            serde_json::from_value(authenticated).unwrap();
        assert_eq!(
            transfer["supervisor_attestation"]["binding_digest"],
            review_state::sha256_digest(&serde_json::to_vec(&typed).unwrap())
        );
        assert_eq!(transfer["retirement_evidence"]["session_status"], "stopped");
        let saved: Vec<String> = serde_json::from_str(
            &fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap(),
        )
        .unwrap();
        let chain =
            review_state::parse_chain(saved.iter().map(String::as_str), "acme/widgets", 7).unwrap();
        let ReviewStatePayload::ReviewHandoff { handoff: persisted } =
            &chain.records.last().unwrap().payload
        else {
            panic!("takeover must persist a handoff record");
        };
        assert_eq!(
            serde_json::to_value(persisted.coordinator_transfer.as_ref().unwrap()).unwrap(),
            *transfer
        );

        assert!(!out.stdout.contains(actor));
        assert!(!out.stdout.contains("runtime-one"));
        assert!(!out.stdout.contains("\"source\""));
    }
}

#[test]
fn supervisor_role_refuses_unattested_wrong_and_mismatched_binding() {
    let seeded = records(retired_coordinator_handoff(), None);
    for kind in [
        "missing",
        "unattested",
        "wrong",
        "session",
        "incarnation",
        "label-only",
    ] {
        let mut data = binding("successor-session", Some("steward"));
        match kind {
            "missing" => data = Value::Null,
            "unattested" => data
                .as_object_mut()
                .unwrap()
                .remove("attested_role")
                .map(|_| ())
                .unwrap(),
            "wrong" => {
                data["role"] = json!("author");
                data["attested_role"] = json!("author");
            }
            "session" => data["session_id"] = json!("another-session"),
            "incarnation" => data["session_incarnation"] = json!("another-runtime"),
            "label-only" => data["attested_role"] = Value::Null,
            _ => unreachable!(),
        }
        let stub = broker(
            coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", ""),
            data,
        )
        .env("AGENT_ROLE", "steward");
        let out = recover(&stub, &seeded[0].record_digest, true);
        assert_refusal(&out, 65, "review_writer_conflict");
    }
}

#[test]
fn attested_supervisor_still_requires_proven_stopped_coordinator() {
    let seeded = records(retired_coordinator_handoff(), None);
    for (state, expected) in [
        ("live", "review_coordinator_live"),
        ("unknown", "review_coordinator_unproven"),
    ] {
        let stub = broker(
            coordinator_board(fixture(HEAD, &seeded, vec![]), state, ""),
            binding("successor-session", Some("steward")),
        );
        assert_refusal(
            &recover(&stub, &seeded[0].record_digest, true),
            65,
            expected,
        );
    }
}

#[test]
fn attested_role_does_not_allow_the_recorded_coordinator_or_reviewer_to_take_over() {
    let seeded = records(retired_coordinator_handoff(), None);
    for actor in [COORDINATOR_UUID, "reviewer-session"] {
        let stub = broker(
            coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", "")
                .env("AGENT_SESSION_ID", actor),
            binding(actor, Some("steward")),
        );
        assert_refusal(
            &recover(&stub, &seeded[0].record_digest, true),
            65,
            "review_writer_conflict",
        );
    }
}

#[test]
fn supervisor_role_is_reauthenticated_before_ledger_append() {
    let seeded = records(retired_coordinator_handoff(), None);
    let stub = broker(
        coordinator_board(fixture(HEAD, &seeded, vec![]), "stopped", ""),
        binding("successor-session", Some("steward")),
    );
    let first = json!({"ok":true,"schema_version":"cli.agent-session.broker-identity.v1",
        "data":binding("successor-session", Some("steward"))});
    let second = json!({"ok":true,"schema_version":"cli.agent-session.broker-identity.v1",
        "data":binding("successor-session", Some("author"))});
    stub.write_stub(
        "identity-broker",
        &format!(
            r#"#!/bin/sh
if [ -e '{marker}' ]; then
cat <<'JSON'
{second}
JSON
else
touch '{marker}'
cat <<'JSON'
{first}
JSON
fi
"#,
            marker = stub.tempdir.path().join("identity-read").display()
        ),
    );
    let out = recover(&stub, &seeded[0].record_digest, false);
    assert_refusal(&out, 65, "review_writer_conflict");
    assert!(
        !fs::read_to_string(stub.tempdir.path().join("calls.log"))
            .unwrap()
            .contains("--method POST")
    );
}

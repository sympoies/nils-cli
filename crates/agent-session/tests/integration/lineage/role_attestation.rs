//! Operator launch role provenance is distinct from a session's display role.
use super::*;
use pretty_assertions::assert_eq;

#[test]
fn operator_can_attest_its_new_sessions_role() {
    let fixture = Fixture::new();
    fixture.started(
        "supervisor",
        &[
            "--forge-initiator",
            "operator",
            "--role",
            "steward",
            "--attest-role",
        ],
        None,
    );
    let record = fixture.record("supervisor");
    assert_eq!(record["lineage"]["forge_context"]["initiator"], "operator");
    assert_eq!(record["id"], "supervisor");
    assert!(
        record["created_at"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .is_ok()
    );
    assert_eq!(
        record["lineage"]["forge_context"]["attested_role"],
        "steward"
    );
}

#[test]
fn managed_sessions_cannot_self_attest_even_as_a_new_root() {
    let fixture = Fixture::new();
    fixture.started(
        "parent",
        &[
            "--forge-initiator",
            "operator",
            "--role",
            "steward",
            "--attest-role",
        ],
        None,
    );
    let parent = fixture.record("parent");
    let runtime = parent["runtime"]["launch_id"].as_str().unwrap();
    for (id, extra) in [("child", vec![]), ("new-root", vec!["--no-parent"])] {
        let mut args = vec![
            "--forge-initiator",
            "operator",
            "--role",
            "steward",
            "--attest-role",
        ];
        args.extend(extra);
        let out = fixture.start(id, &args, Some(("parent", runtime)));
        assert_eq!(out.code, 65, "{}", out.stdout_text());
        assert_eq!(
            out.stdout_json()["error"]["code"],
            "role-attestation-forbidden"
        );
        assert!(!Path::new(&fixture.state).join("sessions").join(id).exists());
    }
}

#[test]
fn role_attestation_requires_an_operator_initiator_and_role() {
    let fixture = Fixture::new();
    for args in [
        vec!["--attest-role"],
        vec!["--attest-role", "--role", "steward"],
        vec!["--attest-role", "--forge-initiator", "operator"],
    ] {
        let out = fixture.start("invalid", &args, None);
        assert_eq!(out.code, 64, "{}", out.stdout_text());
        assert!(!Path::new(&fixture.state).join("sessions/invalid").exists());
    }
}

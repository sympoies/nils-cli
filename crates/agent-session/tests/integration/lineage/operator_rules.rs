use super::*;
use pretty_assertions::assert_eq;

fn rules(fixture: &Fixture, raw: &str) {
    fs::write(fixture.root.join("rules.json"), raw).unwrap();
}

#[test]
fn operator_rules_allow_both_coordinator_roles_in_a_managed_cli_child() {
    let fixture = Fixture::new();
    fixture.started("parent", &[], None);
    let record = fixture.record("parent");
    let launch = record["runtime"]["launch_id"].as_str().unwrap();
    rules(
        &fixture,
        r#"{"schema_version":"agent-session.operator-rules.v1","coordinator_requires_root":false}"#,
    );
    for role in ["coordinator", "domain-coordinator"] {
        let child = fixture.started(role, &["--role", role], Some(("parent", launch)));
        assert_eq!(child["role"], role);
        assert_eq!(
            fixture.record(role)["lineage"]["parent"]["session_id"],
            "parent"
        );
    }
}

#[test]
fn corrupt_operator_rules_warn_once_and_do_not_partially_relax_cli_start() {
    let fixture = Fixture::new();
    fixture.started("parent", &[], None);
    let record = fixture.record("parent");
    let launch = record["runtime"]["launch_id"].as_str().unwrap();
    rules(
        &fixture,
        r#"{"schema_version":"agent-session.operator-rules.v1","coordinator_requires_root":false,"unexpected":true}"#,
    );
    let output = fixture.start("ordinary-child", &[], Some(("parent", launch)));
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stderr_text().matches("warning:").count(), 1);
    assert!(output.stderr_text().contains("rules.json"));
    assert!(output.stderr_text().contains("unexpected"));
    for role in ["coordinator", "domain-coordinator"] {
        let output = fixture.start(role, &["--role", role], Some(("parent", launch)));
        assert_eq!(output.stdout_json()["error"]["code"], "role-requires-root");
        assert_eq!(output.stderr_text().matches("warning:").count(), 1);
    }
}

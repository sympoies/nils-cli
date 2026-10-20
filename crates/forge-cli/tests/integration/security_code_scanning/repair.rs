//! Closed-set regressions for evidence budgets and scanner/configuration binding.
use super::{
    COMMIT, REFERENCE, StubEnv, args, fixture, fixture_with_tool, instance, parse_envelope,
    run_forge_cli,
};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

fn analysis() -> Value {
    json!({"id": 9, "ref": REFERENCE, "commit_sha": COMMIT,
        "analysis_key": "workflow:analyze", "environment": "{}",
        "category": "language:rust", "tool": {"name": "CodeQL"}})
}

fn read(stub: &StubEnv) -> (i32, Value) {
    let mut argv = args();
    argv.push("--include-sarif");
    let out = run_forge_cli(stub, &argv);
    (out.code, parse_envelope(&out.stdout))
}

fn assert_no_analysis(stub: &StubEnv) {
    let (code, value) = read(stub);
    assert_eq!(code, 65, "{value}");
    assert_eq!(value["error"]["code"], "security_analysis_not_found");
    assert!(value.get("data").is_none());
}

fn replace_script(stub: &StubEnv, change: impl FnOnce(String) -> String) {
    let path = &stub
        .envs
        .iter()
        .find(|(key, _)| key == "FORGE_CLI_GH_BIN")
        .unwrap()
        .1;
    std::fs::write(path, change(std::fs::read_to_string(path).unwrap())).unwrap();
}

fn padded_sarif(stub: &StubEnv, bytes: usize) {
    let path = stub.tempdir.path().join("evidence.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"version": "2.1.0", "runs": [],
        "padding": "x".repeat(bytes)}))
        .unwrap(),
    )
    .unwrap();
    replace_script(stub, |script| {
        script.replace("#!/bin/sh", &format!(r#"#!/bin/sh
case "$*" in
 'api -X GET repos/acme/widget/code-scanning/analyses/'*' -H Accept:application/sarif+json') cat '{}'; exit 0;;
esac
"#, path.display()))
    });
}

#[test]
fn repair_rejects_oversized_backend_capture_without_partial_evidence() {
    let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!([analysis()]));
    padded_sarif(&stub, 9 * 1024 * 1024);
    let (code, value) = read(&stub);
    assert_ne!(code, 0);
    assert_eq!(value["error"]["code"], "backend_output_limit");
    assert!(value.get("data").is_none());
}

#[test]
fn repair_rejects_cumulative_evidence_bytes_without_partial_evidence() {
    let mut rows = Vec::new();
    for id in 9..12 {
        let mut row = analysis();
        row["id"] = json!(id);
        rows.push(row);
    }
    let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!(rows));
    padded_sarif(&stub, 6 * 1024 * 1024);
    let (code, value) = read(&stub);
    assert_ne!(code, 0);
    assert_eq!(value["error"]["code"], "security_read_limit");
    assert!(value.get("data").is_none());
}

#[test]
fn repair_bounds_the_total_number_of_evidence_requests() {
    let rows: Vec<_> = (9..50)
        .map(|id| {
            let mut row = analysis();
            row["id"] = json!(id);
            row
        })
        .collect();
    let stub = fixture(json!([instance(REFERENCE, COMMIT)]), json!(rows));
    padded_sarif(&stub, 0);
    let (code, value) = read(&stub);
    assert_ne!(code, 0);
    assert_eq!(value["error"]["code"], "security_read_limit");
    assert!(value.get("data").is_none());
}

#[test]
fn repair_requires_each_configuration_binding_to_be_a_present_string() {
    for field in ["analysis_key", "environment", "category"] {
        for invalid in [None, Some(Value::Null), Some(json!(42))] {
            let mut selected = instance(REFERENCE, COMMIT);
            let mut row = analysis();
            for item in [&mut selected, &mut row] {
                if let Some(value) = &invalid {
                    item[field] = value.clone();
                } else {
                    item.as_object_mut().unwrap().remove(field);
                }
            }
            assert_no_analysis(&fixture(json!([selected]), json!([row])));
        }
    }
}

#[test]
fn repair_rejects_independently_mismatched_configuration_fields() {
    for field in [
        "ref",
        "commit_sha",
        "analysis_key",
        "environment",
        "category",
    ] {
        let mut row = analysis();
        row[field] = json!("different");
        assert_no_analysis(&fixture(json!([instance(REFERENCE, COMMIT)]), json!([row])));
    }
}

#[test]
fn repair_requires_present_nonempty_matching_tool_names() {
    for tool in [
        json!({}),
        json!({"name": null}),
        json!({"name": 42}),
        json!({"name": ""}),
    ] {
        let mut row = analysis();
        row["tool"] = tool.clone();
        assert_no_analysis(&fixture_with_tool(
            json!([instance(REFERENCE, COMMIT)]),
            json!([row]),
            tool,
        ));
    }
    let mut row = analysis();
    row["tool"]["name"] = json!("Another scanner");
    assert_no_analysis(&fixture(json!([instance(REFERENCE, COMMIT)]), json!([row])));
}

#[test]
fn repair_correlates_nonempty_tool_guid_and_allows_null_name_fallback() {
    for (alert_guid, analysis_guid, accepted) in [
        (json!("scanner-a"), json!("scanner-a"), true),
        (json!("scanner-a"), json!("scanner-b"), false),
        (json!("scanner-a"), Value::Null, false),
        (Value::Null, Value::Null, true),
        (Value::Null, json!("scanner-a"), true),
    ] {
        let mut row = analysis();
        row["tool"]["guid"] = analysis_guid;
        row["tool"]["version"] = json!("2");
        let tool = json!({"name": "CodeQL", "guid": alert_guid, "version": "1"});
        let stub = fixture_with_tool(json!([instance(REFERENCE, COMMIT)]), json!([row]), tool);
        if accepted {
            let (code, value) = read(&stub);
            assert_eq!(code, 0, "{value}");
        } else {
            assert_no_analysis(&stub);
        }
    }
}

#[test]
fn repair_accepts_empty_environment_when_present_on_both_sides() {
    let mut selected = instance(REFERENCE, COMMIT);
    selected["environment"] = json!("");
    let mut row = analysis();
    row["environment"] = json!("");
    let (code, value) = read(&fixture(json!([selected]), json!([row])));
    assert_eq!(code, 0, "{value}");
}

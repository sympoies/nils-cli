//! `pr checks` keeps GitHub's gate semantics for skipped or neutral required
//! checks but reports them separately from real passes (offline backend).
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use pretty_assertions::assert_eq;
use serde_json::json;
use support::{StubEnv, parse_envelope, run_forge_cli};

const REQUIRED: &str = r#"[{"name":"build","bucket":"pass","state":"SUCCESS"},{"name":"macos-full","bucket":"skipping","state":"SKIPPED","link":"https://example.test/run/2"},{"name":"advisory","bucket":"pass","state":"SUCCESS","conclusion":"neutral"}]"#;

fn backend(required: &str, all: &str) -> StubEnv {
    StubEnv::new().gh_stub(&format!(
        r#"#!/bin/sh
set -eu
case "$1 $2" in
  "pr checks")
    case " $* " in
      *" --required "*) echo '{required}';;
      *) echo '{all}';;
    esac;;
  *) echo "unexpected fixture request: $*" >&2; exit 99;;
esac
"#
    ))
}

fn all_checks() -> String {
    let mut rows: Vec<serde_json::Value> = serde_json::from_str(REQUIRED).unwrap();
    rows.push(json!({"name":"optional-docs","bucket":"skipping","state":"SKIPPED"}));
    serde_json::to_string(&rows).unwrap()
}

fn checks(stub: &StubEnv, extra: &[&str]) -> support::CmdOutput {
    let mut args = vec!["--provider", "github", "--repo", "example/project"];
    args.extend(["pr", "checks", "1"]);
    args.extend_from_slice(extra);
    run_forge_cli(stub, &args)
}

#[test]
fn skipped_and_neutral_required_checks_are_reported_apart_from_passes() {
    let stub = backend(REQUIRED, &all_checks());
    let out = checks(&stub, &["--format", "json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];

    // Gate semantics are unchanged: skipped and neutral still satisfy the rule.
    assert_eq!(data["state"], "success");
    assert_eq!(data["gate_state"], "success");
    assert_eq!(data["required_count"], 3);
    assert_eq!(data["success_count"], 1);

    assert_eq!(data["required_skipped"], 1);
    assert_eq!(data["required_neutral"], 1);
    assert_eq!(
        data["skipped_required"],
        json!([
            {"name":"macos-full","state":"skipped","url":"https://example.test/run/2"},
            {"name":"advisory","state":"neutral","conclusion":"neutral"},
        ])
    );
}

#[test]
fn all_success_required_checks_report_zero_skipped_counts_and_an_empty_list() {
    let all = r#"[{"name":"build","bucket":"pass","state":"SUCCESS"},{"name":"test","bucket":"pass","state":"SUCCESS"}]"#;
    let stub = backend(all, all);
    let out = checks(&stub, &["--format", "json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["gate_state"], "success");
    assert_eq!(data["required_count"], 2);
    assert_eq!(data["success_count"], 2);
    assert_eq!(data["required_skipped"], 0);
    assert_eq!(data["required_neutral"], 0);
    assert_eq!(data["skipped_required"], json!([]));

    let text = checks(&stub, &[]);
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(!text.stdout.contains("skipped-required"), "{}", text.stdout);
    assert!(!text.stdout.contains("required_skipped"), "{}", text.stdout);
}

#[test]
fn text_output_marks_skipped_required_checks_by_name() {
    let stub = backend(REQUIRED, &all_checks());
    let out = checks(&stub, &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout
            .starts_with("success [github] required=3 success=1")
    );
    assert!(out.stdout.contains("gate_state=success"), "{}", out.stdout);
    assert!(
        out.stdout.contains("required_skipped=1 required_neutral=1"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("not proof the work ran"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("skipped-required: macos-full (skipped)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("skipped-required: advisory (neutral)"),
        "{}",
        out.stdout
    );
    assert!(!out.stdout.contains("optional-docs"), "{}", out.stdout);
}

#[test]
fn non_required_skipped_checks_are_not_listed_as_skipped_required() {
    let stub = backend(
        r#"[{"name":"build","bucket":"pass","state":"SUCCESS"}]"#,
        &all_checks(),
    );
    let out = checks(&stub, &["--format", "json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["gate_scope"], "required");
    assert_eq!(data["required_skipped"], 0);
    assert_eq!(data["skipped_required"], json!([]));
}

#[test]
fn strict_rejects_each_skipped_or_neutral_required_check() {
    for state in ["skipped", "neutral"] {
        let rows = json!([
            {"name":"build", "bucket":"pass", "state":"SUCCESS"},
            {"name":"required-job", "bucket":"pass", "conclusion":state,
             "link":"https://example.test/run/2"}
        ])
        .to_string();
        let stub = backend(&rows, &rows);
        let out = checks(&stub, &["--format", "json", "--strict"]);
        assert_eq!(out.code, 1, "{state}: {}", out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["ok"], false);
        assert_eq!(envelope["error"]["code"], "checks_failed");
        let data = &envelope["data"];
        assert_eq!(data["state"], "failure");
        assert_eq!(data["gate_state"], "failure");
        assert_eq!(data["success_count"], 1);
        assert_eq!(data["checks"][1]["state"], state);
        assert_eq!(data["skipped_required"][0]["state"], state);
        assert_eq!(data["failed"][0]["name"], "required-job");
        assert_eq!(data["failed"][0]["url"], "https://example.test/run/2");
        assert_eq!(data["failed"][0]["conclusion"], state);

        let default = checks(&stub, &["--format", "json"]);
        assert_eq!(default.code, 0, "{}", default.stderr);
        assert_eq!(
            parse_envelope(&default.stdout)["data"]["gate_state"],
            "success"
        );
    }
}

#[test]
fn strict_all_success_and_optional_skips_still_pass() {
    let rows = r#"[{"name":"build","bucket":"pass","state":"SUCCESS"}]"#;
    let stub = backend(rows, &all_checks());
    let out = checks(&stub, &["--format", "json", "--strict"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let envelope = parse_envelope(&out.stdout);
    assert_eq!(envelope["data"]["gate_state"], "success");
    assert_eq!(envelope["data"]["failed"], json!([]));
}

#[test]
fn strict_applies_to_all_visible_checks_when_they_gate() {
    for (required, extra) in [
        ("[]", vec!["--strict"]),
        (
            r#"[{"name":"build","bucket":"pass","state":"SUCCESS"}]"#,
            vec!["--strict", "--required-only=false"],
        ),
    ] {
        let stub = backend(required, &all_checks());
        let mut args = vec!["--format", "json"];
        args.extend(extra);
        let out = checks(&stub, &args);
        assert_eq!(out.code, 1, "{}", out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["gate_state"], "failure");
        assert!(
            envelope["data"]["failed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["name"] == "optional-docs")
        );
    }
}

#[test]
fn strict_text_reports_failure_and_preserves_skipped_names() {
    let stub = backend(REQUIRED, &all_checks());
    let out = checks(&stub, &["--strict"]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert!(out.stdout.starts_with("failure [github]"), "{}", out.stdout);
    assert!(out.stdout.contains("gate_state=failure"), "{}", out.stdout);
    assert!(
        out.stdout
            .contains("skipped-required: macos-full (skipped)"),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("skipped-required: advisory (neutral)"),
        "{}",
        out.stdout
    );
}

#[test]
fn strict_existing_failure_and_pending_are_not_passing() {
    for (state, code) in [("FAILURE", "checks_failed"), ("PENDING", "checks_pending")] {
        let rows = json!([{"name":"build", "state":state}]).to_string();
        let stub = backend(&rows, &rows);
        let out = checks(&stub, &["--format", "json", "--strict"]);
        assert_eq!(out.code, 1, "{}", out.stderr);
        assert_eq!(parse_envelope(&out.stdout)["error"]["code"], code);
    }
}

#[test]
fn strict_help_explains_default_and_execution_semantics() {
    let stub = backend("[]", "[]");
    let out = checks(&stub, &["--help"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.contains("--strict"), "{}", out.stdout);
    assert!(out.stdout.contains("Skipped and neutral"), "{}", out.stdout);
    assert!(out.stdout.contains("By default"), "{}", out.stdout);
}

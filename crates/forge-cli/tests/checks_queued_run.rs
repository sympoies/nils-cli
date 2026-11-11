//! A head whose workflow run is queued with no jobs yet has not been checked.
//! Offline backend: synthetic repository, head and run identifiers only.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use pretty_assertions::assert_eq;
use support::{StubEnv, parse_envelope, run_forge_cli};

const BUILD_RUN: &str =
    r#"{"name":"build","status":"completed","conclusion":"success","check_suite":{"id":11}}"#;

/// `gh` stub answering both the one-shot rollup read and the exact-head gate
/// reads for the same head. `check_runs` and `workflow_runs` are JSON arrays.
fn backend(check_runs: &str, workflow_runs: &str) -> StubEnv {
    let rollup = if check_runs == "[]" {
        "[]".to_string()
    } else {
        r#"[{"name":"build","bucket":"pass","state":"SUCCESS"}]"#.to_string()
    };
    StubEnv::new().gh_stub(&format!(
        r#"#!/bin/sh
set -eu
case "$1 $2" in
  "pr view")
    echo '{{"headRefOid":"test-head","baseRefName":"main","url":"https://github.com/example/project/pull/1"}}';;
  "pr checks") echo '{rollup}';;
  "api graphql")
    echo '{{"data":{{"repository":{{"ref":{{"branchProtectionRule":null}}}}}}}}';;
  "api repos/example/project/rules/branches/main?per_page=100") echo '[]';;
  "api repos/example/project/commits/test-head/check-runs?per_page=100")
    echo '{{"total_count":1,"check_runs":{check_runs}}}';;
  "api repos/example/project/commits/test-head/status?per_page=100")
    echo '{{"total_count":0,"statuses":[]}}';;
  "api repos/example/project/actions/runs?head_sha=test-head&per_page=100")
    echo '{{"total_count":1,"workflow_runs":{workflow_runs}}}';;
  *) echo "unexpected fixture request: $*" >&2; exit 99;;
esac
"#
    ))
}

fn queued_run_without_jobs() -> &'static str {
    r#"[{"id":777,"name":"CI","status":"queued","conclusion":null,"check_suite_id":22,"created_at":"2020-01-01T00:00:00Z","html_url":"https://github.com/example/project/actions/runs/777"}]"#
}

fn forge(stub: &StubEnv, tail: &[&str]) -> support::CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "example/project",
        "--format",
        "json",
        "pr",
    ];
    args.extend_from_slice(tail);
    run_forge_cli(stub, &args)
}

fn assert_names_queued_run(data: &serde_json::Value) {
    let name = data["pending"][0]["name"].as_str().unwrap_or_default();
    assert!(
        name.contains("777") && name.contains("queued with no jobs for"),
        "pending row must name the run and its age: {data}"
    );
}

#[test]
fn wait_does_not_pass_while_a_queued_run_has_no_jobs() {
    let stub = backend(&format!("[{BUILD_RUN}]"), queued_run_without_jobs());
    let out = forge(
        &stub,
        &["wait-checks", "1", "--timeout", "3s", "--interval", "100ms"],
    );
    assert_eq!(out.code, 69, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["ok"], false);
    assert_eq!(env["error"]["code"], "checks_timeout");
    assert_eq!(env["data"]["state"], "pending");
    assert_names_queued_run(&env["data"]);
}

#[test]
fn strict_read_with_no_check_rows_names_the_queued_run() {
    let stub = backend("[]", queued_run_without_jobs());
    let out = forge(&stub, &["checks", "1", "--strict"]);
    assert_eq!(out.code, 1, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["error"]["code"], "checks_pending");
    assert_eq!(env["data"]["state"], "pending");
    assert_names_queued_run(&env["data"]);
}

#[test]
fn default_read_with_no_check_rows_reports_the_queued_run_as_pending() {
    let stub = backend("[]", queued_run_without_jobs());
    let out = forge(&stub, &["checks", "1"]);
    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["state"], "pending");
    assert_names_queued_run(&env["data"]);
}

/// A run that already has jobs, and a finished run, are not "queued with no
/// jobs": the existing green result must not change.
#[test]
fn runs_with_jobs_or_already_finished_do_not_block_a_green_head() {
    let runs = r#"[
        {"id":5,"name":"CI","status":"in_progress","conclusion":null,"check_suite_id":11,"created_at":"2020-01-01T00:00:00Z"},
        {"id":6,"name":"Docs","status":"completed","conclusion":"success","check_suite_id":33,"created_at":"2020-01-01T00:00:00Z"}
    ]"#
    .replace('\n', "");
    let stub = backend(&format!("[{BUILD_RUN}]"), &runs);
    let out = forge(
        &stub,
        &["wait-checks", "1", "--timeout", "5s", "--interval", "100ms"],
    );
    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(parse_envelope(&out.stdout)["data"]["state"], "success");
}

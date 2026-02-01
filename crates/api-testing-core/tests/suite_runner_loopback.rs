use std::collections::HashSet;
use std::path::PathBuf;

use api_testing_core::suite::runner::{run_suite, SuiteRunOptions};
use api_testing_core::suite::safety::MSG_WRITE_CASES_DISABLED;
use api_testing_core::suite::schema::load_and_validate_suite;
use nils_test_support::fixtures::write_text;
use nils_test_support::http::{HttpResponse, LoopbackServer};
use tempfile::TempDir;

fn resolve_output_path(root: &std::path::Path, rel: &str) -> PathBuf {
    let path = PathBuf::from(rel);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

#[test]
fn suite_runner_loopback_runs_and_cleans_up() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();

    write_text(
        &root.join("requests/health.request.json"),
        r#"{"method":"GET","path":"/health","expect":{"status":200}}"#,
    );
    write_text(
        &root.join("requests/write.request.json"),
        r#"{"method":"POST","path":"/write","body":{"ok":true},"expect":{"status":200}}"#,
    );

    let suite_json = serde_json::json!({
        "version": 1,
        "defaults": { "noHistory": true },
        "cases": [
            {
                "id": "rest.health",
                "type": "rest",
                "request": "requests/health.request.json",
                "cleanup": {
                    "type": "rest",
                    "method": "DELETE",
                    "pathTemplate": "/cleanup/{{id}}",
                    "vars": { "id": ".data.id" }
                }
            },
            {
                "id": "rest.write",
                "type": "rest",
                "allowWrite": true,
                "request": "requests/write.request.json"
            }
        ]
    });
    let suite_path = root.join("suite.json");
    write_text(
        &suite_path,
        &serde_json::to_string_pretty(&suite_json).unwrap(),
    );

    let server = LoopbackServer::new().expect("server");
    server.add_route(
        "GET",
        "/health",
        HttpResponse::new(200, r#"{"data":{"id":"123"}}"#),
    );
    server.add_route("POST", "/write", HttpResponse::new(200, r#"{"ok":true}"#));
    server.add_route("DELETE", "/cleanup/123", HttpResponse::new(204, ""));

    let loaded = load_and_validate_suite(&suite_path).expect("load suite");
    let options = SuiteRunOptions {
        required_tags: Vec::new(),
        only_ids: HashSet::new(),
        skip_ids: HashSet::new(),
        allow_writes_flag: false,
        fail_fast: false,
        output_dir_base: root.join("out-disabled"),
        env_rest_url: server.url(),
        env_gql_url: String::new(),
    };

    let run_disabled = run_suite(root, loaded.clone(), options).expect("run suite");
    let health = run_disabled
        .results
        .cases
        .iter()
        .find(|c| c.id == "rest.health")
        .expect("health case");
    if let Some(stderr_rel) = health.stderr_file.as_deref() {
        let stderr_path = resolve_output_path(root, stderr_rel);
        let contents = std::fs::read_to_string(&stderr_path).expect("stderr read");
        assert!(contents.contains("cleanup skipped (writes disabled)"));
    }
    assert_eq!(run_disabled.results.summary.total, 2);
    assert_eq!(run_disabled.results.summary.passed, 1);
    assert_eq!(run_disabled.results.summary.skipped, 1);
    assert_eq!(run_disabled.results.summary.failed, 0);
    assert_eq!(health.status, "passed");
    assert!(health
        .command
        .as_deref()
        .unwrap_or("")
        .contains("--no-history"));
    let stdout_rel = health.stdout_file.as_deref().expect("stdout file");
    let stdout_path = resolve_output_path(root, stdout_rel);
    let stdout_body = std::fs::read_to_string(&stdout_path).expect("stdout read");
    assert!(stdout_body.contains("\"id\":\"123\""));

    let write_case = run_disabled
        .results
        .cases
        .iter()
        .find(|c| c.id == "rest.write")
        .expect("write case");
    assert_eq!(write_case.status, "skipped");
    assert_eq!(
        write_case.message.as_deref(),
        Some(MSG_WRITE_CASES_DISABLED)
    );

    let options = SuiteRunOptions {
        required_tags: Vec::new(),
        only_ids: HashSet::new(),
        skip_ids: HashSet::new(),
        allow_writes_flag: true,
        fail_fast: false,
        output_dir_base: root.join("out-enabled"),
        env_rest_url: server.url(),
        env_gql_url: String::new(),
    };

    let run_enabled = run_suite(root, loaded, options).expect("run suite writes enabled");
    assert_eq!(run_enabled.results.summary.failed, 0);
    assert_eq!(run_enabled.results.summary.passed, 2);

    let requests = server.take_requests();
    assert!(requests
        .iter()
        .any(|r| r.method == "DELETE" && r.path == "/cleanup/123"));
}

use std::collections::HashSet;
use std::path::PathBuf;

use api_testing_core::suite::runner::{SuiteRunOptions, run_suite};
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

fn write_suite(root: &std::path::Path, name: &str, cases: serde_json::Value) -> PathBuf {
    let suite = serde_json::json!({
        "version": 1,
        "name": name,
        "cases": cases
    });
    let suite_path = root.join(format!("{name}.suite.json"));
    write_text(&suite_path, &serde_json::to_string_pretty(&suite).unwrap());
    suite_path
}

fn base_options(root: &std::path::Path, server: &LoopbackServer) -> SuiteRunOptions {
    SuiteRunOptions {
        required_tags: Vec::new(),
        only_ids: HashSet::new(),
        skip_ids: HashSet::new(),
        allow_writes_flag: false,
        fail_fast: false,
        output_dir_base: root.join("out"),
        env_rest_url: server.url(),
        env_gql_url: format!("{}/graphql", server.url()),
        env_grpc_url: String::new(),
        progress: None,
    }
}

fn assert_outputs(
    root: &std::path::Path,
    output: &api_testing_core::suite::runner::SuiteRunOutput,
) {
    for case in &output.results.cases {
        if case.status == "skipped" {
            continue;
        }
        let stdout = case.stdout_file.as_deref().expect("stdout file");
        let stderr = case.stderr_file.as_deref().expect("stderr file");
        assert!(resolve_output_path(root, stdout).is_file());
        assert!(resolve_output_path(root, stderr).is_file());
    }
}

#[test]
fn suite_runner_handles_rest_graphql_matrix() {
    let tmp = TempDir::new().expect("tempdir");
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".git")).expect("repo marker");

    write_text(
        &root.join("requests/health.request.json"),
        r#"{"method":"GET","path":"/health","expect":{"status":200}}"#,
    );
    write_text(
        &root.join("requests/ping.request.json"),
        r#"{"method":"GET","path":"/ping","expect":{"status":200}}"#,
    );
    write_text(&root.join("ops/health.graphql"), "query Health { ok }\n");

    let server = LoopbackServer::new().expect("server");
    server.add_route("GET", "/health", HttpResponse::new(200, r#"{"ok":true}"#));
    server.add_route("GET", "/ping", HttpResponse::new(200, r#"{"ok":true}"#));
    server.add_route(
        "POST",
        "/graphql",
        HttpResponse::new(200, r#"{"data":{"ok":true}}"#),
    );

    let rest_suite = write_suite(
        root,
        "rest-only",
        serde_json::json!([
            { "id": "rest.health", "type": "rest", "request": "requests/health.request.json" }
        ]),
    );
    let gql_suite = write_suite(
        root,
        "graphql-only",
        serde_json::json!([
            { "id": "graphql.health", "type": "graphql", "op": "ops/health.graphql" }
        ]),
    );
    let mixed_suite = write_suite(
        root,
        "mixed",
        serde_json::json!([
            { "id": "rest.health", "type": "rest", "request": "requests/health.request.json" },
            { "id": "rest.ping", "type": "rest", "request": "requests/ping.request.json" },
            { "id": "graphql.health", "type": "graphql", "op": "ops/health.graphql" }
        ]),
    );

    let base = base_options(root, &server);

    let mut rest_options = base.clone();
    rest_options.output_dir_base = root.join("out-rest");
    let rest_out = run_suite(
        root,
        load_and_validate_suite(&rest_suite).expect("load rest suite"),
        rest_options,
    )
    .expect("run rest suite");
    assert_eq!(rest_out.results.summary.total, 1);
    assert_eq!(rest_out.results.summary.passed, 1);
    assert_eq!(rest_out.results.summary.failed, 0);
    assert_eq!(rest_out.results.summary.skipped, 0);
    assert_outputs(root, &rest_out);

    let mut gql_options = base.clone();
    gql_options.output_dir_base = root.join("out-gql");
    let gql_out = run_suite(
        root,
        load_and_validate_suite(&gql_suite).expect("load gql suite"),
        gql_options,
    )
    .expect("run gql suite");
    assert_eq!(gql_out.results.summary.total, 1);
    assert_eq!(gql_out.results.summary.passed, 1);
    assert_eq!(gql_out.results.summary.failed, 0);
    assert_eq!(gql_out.results.summary.skipped, 0);
    assert_outputs(root, &gql_out);

    let mut mixed_options = base;
    mixed_options.output_dir_base = root.join("out-mixed");
    let mixed_out = run_suite(
        root,
        load_and_validate_suite(&mixed_suite).expect("load mixed suite"),
        mixed_options,
    )
    .expect("run mixed suite");
    assert_eq!(mixed_out.results.summary.total, 3);
    assert_eq!(mixed_out.results.summary.passed, 3);
    assert_eq!(mixed_out.results.summary.failed, 0);
    assert_eq!(mixed_out.results.summary.skipped, 0);
    assert_outputs(root, &mixed_out);
}

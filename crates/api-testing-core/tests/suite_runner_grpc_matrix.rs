use std::collections::HashSet;

use api_testing_core::suite::runner::{SuiteRunOptions, run_suite};
use api_testing_core::suite::schema::load_and_validate_suite;
use tempfile::TempDir;

fn write_file(path: &std::path::Path, body: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

#[test]
fn suite_runner_executes_grpc_case_with_mock_transport() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".git")).expect("git marker");

    let mock = root.join("grpcurl-mock.sh");
    std::fs::write(&mock, "#!/bin/sh\necho '{\"ok\":true}'\nexit 0\n").expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&mock).expect("stat").permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&mock, perms).expect("chmod");
    }

    write_file(
        &root.join("requests/health.grpc.json"),
        r#"{
  "method": "health.HealthService/Check",
  "body": {"ping":"pong"},
  "expect": {"status": 0, "jq": ".ok == true"}
}"#,
    );

    write_file(
        &root.join("grpc.suite.json"),
        r#"{
  "version": 1,
  "defaults": {
    "grpc": { "url": "127.0.0.1:50051" }
  },
  "cases": [
    { "id": "grpc.health", "type": "grpc", "request": "requests/health.grpc.json" }
  ]
}"#,
    );

    let loaded = load_and_validate_suite(root.join("grpc.suite.json")).expect("load suite");

    // SAFETY: test-only env mutation in isolated process.
    unsafe { std::env::set_var("GRPCURL_BIN", &mock) };

    let out = run_suite(
        root,
        loaded,
        SuiteRunOptions {
            required_tags: Vec::new(),
            only_ids: HashSet::new(),
            skip_ids: HashSet::new(),
            allow_writes_flag: false,
            fail_fast: false,
            output_dir_base: root.join("out"),
            env_rest_url: String::new(),
            env_gql_url: String::new(),
            env_grpc_url: String::new(),
            progress: None,
        },
    )
    .expect("run suite");

    // SAFETY: test-only env mutation in isolated process.
    unsafe { std::env::remove_var("GRPCURL_BIN") };

    assert_eq!(out.results.summary.total, 1);
    assert_eq!(out.results.summary.passed, 1);
    assert_eq!(out.results.summary.failed, 0);
    let case = &out.results.cases[0];
    assert_eq!(case.id, "grpc.health");
    assert_eq!(case.status, "passed");
    assert!(case.command.as_deref().unwrap_or("").contains("api-grpc"));
}

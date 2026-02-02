use std::path::Path;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::runtime::{
    path_relative_to_repo_or_abs, plan_case_output_paths, resolve_effective_env,
    resolve_effective_no_history, resolve_gql_url, resolve_rest_base_url,
    resolve_rest_token_profile, sanitize_id,
};
use crate::suite::schema::SuiteDefaults;

fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

#[test]
fn runtime_helpers_rest_url_precedence_override_defaults_env_endpoints_fallback() {
    let tmp = TempDir::new().expect("tmp");
    let repo_root = tmp.path();
    let setup_dir = repo_root.join("setup/rest");

    write(
        &setup_dir.join("endpoints.env"),
        "REST_URL_DEV=http://rest.example/dev\n",
    );
    write(
        &setup_dir.join("endpoints.local.env"),
        "REST_URL_DEV=http://rest.example/dev-local\n",
    );

    let mut defaults = SuiteDefaults::default();
    defaults.rest.url = "http://rest.example/defaults".to_string();

    assert_eq!(
        resolve_rest_base_url(
            repo_root,
            "setup/rest",
            "http://rest.example/override",
            "dev",
            &defaults,
            "http://rest.example/env",
        )
        .unwrap(),
        "http://rest.example/override".to_string()
    );

    assert_eq!(
        resolve_rest_base_url(
            repo_root,
            "setup/rest",
            "",
            "dev",
            &defaults,
            "http://rest.example/env",
        )
        .unwrap(),
        "http://rest.example/defaults".to_string()
    );

    defaults.rest.url.clear();
    assert_eq!(
        resolve_rest_base_url(
            repo_root,
            "setup/rest",
            "",
            "dev",
            &defaults,
            "http://rest.example/env",
        )
        .unwrap(),
        "http://rest.example/env".to_string()
    );

    assert_eq!(
        resolve_rest_base_url(repo_root, "setup/rest", "", "dev", &defaults, "").unwrap(),
        "http://rest.example/dev-local".to_string()
    );

    assert_eq!(
        resolve_rest_base_url(repo_root, "setup/rest", "", "", &defaults, "").unwrap(),
        "http://localhost:6700".to_string()
    );
}

#[test]
fn runtime_helpers_graphql_url_precedence_override_defaults_env_endpoints_fallback() {
    let tmp = TempDir::new().expect("tmp");
    let repo_root = tmp.path();
    let setup_dir = repo_root.join("setup/graphql");

    write(
        &setup_dir.join("endpoints.env"),
        "GQL_URL_DEV=http://gql.example/dev\n",
    );
    write(
        &setup_dir.join("endpoints.local.env"),
        "GQL_URL_DEV=http://gql.example/dev-local\n",
    );

    let mut defaults = SuiteDefaults::default();
    defaults.graphql.url = "http://gql.example/defaults".to_string();

    assert_eq!(
        resolve_gql_url(
            repo_root,
            "setup/graphql",
            "http://gql.example/override",
            "dev",
            &defaults,
            "http://gql.example/env",
        )
        .unwrap(),
        "http://gql.example/override".to_string()
    );

    assert_eq!(
        resolve_gql_url(
            repo_root,
            "setup/graphql",
            "",
            "dev",
            &defaults,
            "http://gql.example/env",
        )
        .unwrap(),
        "http://gql.example/defaults".to_string()
    );

    defaults.graphql.url.clear();
    assert_eq!(
        resolve_gql_url(
            repo_root,
            "setup/graphql",
            "",
            "dev",
            &defaults,
            "http://gql.example/env",
        )
        .unwrap(),
        "http://gql.example/env".to_string()
    );

    assert_eq!(
        resolve_gql_url(repo_root, "setup/graphql", "", "dev", &defaults, "").unwrap(),
        "http://gql.example/dev-local".to_string()
    );

    assert_eq!(
        resolve_gql_url(repo_root, "setup/graphql", "", "", &defaults, "").unwrap(),
        "http://localhost:6700/graphql".to_string()
    );
}

#[test]
fn runtime_helpers_token_profile_resolution_reads_tokens_env_and_overrides_with_local() {
    let tmp = TempDir::new().expect("tmp");
    let setup_dir = tmp.path();

    write(
        &setup_dir.join("tokens.env"),
        "REST_TOKEN_MY_PROFILE=from-env\n",
    );
    write(
        &setup_dir.join("tokens.local.env"),
        "REST_TOKEN_MY_PROFILE=from-local\n",
    );

    assert_eq!(
        resolve_rest_token_profile(setup_dir, "my profile").unwrap(),
        "from-local".to_string()
    );
}

#[test]
fn runtime_helpers_token_profile_resolution_errors_for_missing_profile() {
    let tmp = TempDir::new().expect("tmp");
    let setup_dir = tmp.path();

    write(&setup_dir.join("tokens.env"), "REST_TOKEN_OTHER=ok\n");

    let err = resolve_rest_token_profile(setup_dir, "missing-profile").unwrap_err();
    assert_eq!(
        err.to_string(),
        "Token profile 'missing-profile' is empty/missing.".to_string()
    );
}

#[test]
fn runtime_helpers_sanitize_id_normalizes_to_safe_identifier() {
    assert_eq!(sanitize_id("Hello World"), "Hello-World".to_string());
    assert_eq!(sanitize_id("a!!!b"), "a-b".to_string());
    assert_eq!(sanitize_id("abc!!!"), "abc".to_string());
    assert_eq!(sanitize_id("a.b_c-1"), "a.b_c-1".to_string());
    assert_eq!(sanitize_id("!!!"), "case".to_string());
}

#[test]
fn runtime_helpers_path_relative_to_repo_or_abs_strips_prefix_or_keeps_abs() {
    let repo = TempDir::new().expect("repo");
    let repo_root = repo.path();

    assert_eq!(
        path_relative_to_repo_or_abs(repo_root, repo_root),
        ".".to_string()
    );

    let inside = repo_root.join("a").join("b");
    let expected_inside = Path::new("a").join("b").to_string_lossy().to_string();
    assert_eq!(
        path_relative_to_repo_or_abs(repo_root, &inside),
        expected_inside
    );

    let outside_dir = TempDir::new().expect("outside");
    let outside = outside_dir.path().join("x");
    assert_eq!(
        path_relative_to_repo_or_abs(repo_root, &outside),
        outside.to_string_lossy().to_string()
    );
}

#[test]
fn runtime_helpers_resolve_effective_env_prefers_case_over_defaults() {
    let defaults = SuiteDefaults {
        env: "default".to_string(),
        ..SuiteDefaults::default()
    };

    assert_eq!(resolve_effective_env("staging", &defaults), "staging");
    assert_eq!(resolve_effective_env("   ", &defaults), "default");
}

#[test]
fn runtime_helpers_resolve_effective_no_history_prefers_case_override() {
    let defaults = SuiteDefaults {
        no_history: false,
        ..SuiteDefaults::default()
    };

    assert!(!resolve_effective_no_history(None, &defaults));
    assert!(resolve_effective_no_history(Some(true), &defaults));
}

#[test]
fn runtime_helpers_plan_case_output_paths_is_deterministic() {
    let tmp = TempDir::new().expect("tmp");
    let run_dir = tmp.path().join("out/20260202-000000Z");
    let outputs = plan_case_output_paths(&run_dir, "case-1");
    assert_eq!(outputs.stdout_path, run_dir.join("case-1.response.json"));
    assert_eq!(outputs.stderr_path, run_dir.join("case-1.stderr.log"));
}

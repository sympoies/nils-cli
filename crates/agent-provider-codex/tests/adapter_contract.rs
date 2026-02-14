use agent_provider_codex::CodexProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateStatus, CapabilitiesRequest, ExecuteRequest, HealthStatus,
    HealthcheckRequest, LimitsRequest, ProviderErrorCategory, ProviderMaturity,
};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::path::Path;

const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";

fn token(payload: &str) -> String {
    format!("{HEADER}.{payload}.sig")
}

fn write_auth_json(path: &Path, payload: &str, account_id: &str) {
    let body = format!(
        r#"{{"tokens":{{"id_token":"{}","access_token":"{}","account_id":"{}"}}}}"#,
        token(payload),
        token(payload),
        account_id
    );
    std::fs::write(path, body).expect("write auth json");
}

#[test]
fn metadata_uses_normalized_provider_identity() {
    let adapter = CodexProviderAdapter::new();
    let metadata = adapter.metadata();

    assert_eq!(metadata.id, "codex");
    assert_eq!(metadata.contract_version.as_str(), "provider-adapter.v1");
    assert_eq!(metadata.maturity, ProviderMaturity::Stable);
}

#[test]
fn capabilities_disable_execute_when_binary_is_missing() {
    let lock = GlobalStateLock::new();
    let _path = EnvGuard::set(&lock, "PATH", "");
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "false");

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");

    let execute = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, false);
    assert_eq!(
        execute.description.as_deref(),
        Some("codex binary is not available on PATH")
    );
}

#[test]
fn capabilities_include_experimental_when_requested() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest {
            include_experimental: true,
        })
        .expect("capabilities");

    let execute = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, true);
    assert_eq!(
        execute.description.as_deref(),
        Some("codex execution is enabled")
    );

    let diag = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "diag.rate-limits")
        .expect("diag capability");
    assert_eq!(diag.available, true);

    let auth_commands = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "auth.commands")
        .expect("auth.commands capability");
    assert_eq!(auth_commands.available, true);
}

#[test]
fn healthcheck_is_unhealthy_when_binary_is_missing() {
    let lock = GlobalStateLock::new();
    let _path = EnvGuard::set(&lock, "PATH", "");

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest {
            timeout_ms: Some(1500),
        })
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Unhealthy);
    assert_eq!(
        response.summary.as_deref(),
        Some("codex adapter is unavailable")
    );
    let details = response.details.expect("details");
    assert_eq!(details["codex_binary_available"], false);
    assert_eq!(details["requested_timeout_ms"], 1500);
}

#[test]
fn healthcheck_is_degraded_when_policy_is_disabled() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");

    let auth_file = stub.path().join("auth.json");
    write_auth_json(&auth_file, PAYLOAD_ALPHA, "acct_001");

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "false");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        auth_file.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Degraded);
    assert_eq!(
        response.summary.as_deref(),
        Some("codex adapter is partially ready")
    );
    let details = response.details.expect("details");
    assert_eq!(details["codex_binary_available"], true);
    assert_eq!(details["dangerous_policy_enabled"], false);
    assert_eq!(details["auth_file_exists"], true);
}

#[test]
fn healthcheck_is_healthy_when_policy_enabled_and_auth_file_exists() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");

    let auth_file = stub.path().join("auth.json");
    write_auth_json(&auth_file, PAYLOAD_ALPHA, "acct_001");

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        auth_file.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Healthy);
    assert_eq!(response.summary.as_deref(), Some("codex adapter is ready"));
    let details = response.details.expect("details");
    assert_eq!(details["codex_binary_available"], true);
    assert_eq!(details["dangerous_policy_enabled"], true);
    assert_eq!(details["auth_file_exists"], true);
}

#[test]
fn execute_returns_validation_error_when_task_and_input_are_blank() {
    let adapter = CodexProviderAdapter::new();
    let request = ExecuteRequest {
        task: "  ".to_string(),
        input: Some("   ".to_string()),
        timeout_ms: None,
    };
    let error = adapter
        .execute(request)
        .expect_err("expected missing-task error");

    assert_eq!(error.category, ProviderErrorCategory::Validation);
    assert_eq!(error.code, "missing-task");
}

#[test]
fn execute_returns_dependency_error_when_binary_is_unavailable() {
    let lock = GlobalStateLock::new();
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");
    let _path = EnvGuard::set(&lock, "PATH", "");

    let adapter = CodexProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected missing-binary error");

    assert_eq!(error.category, ProviderErrorCategory::Dependency);
    assert_eq!(error.code, "missing-binary");
    assert!(error.message.contains("codex"));
}

#[test]
fn execute_returns_validation_error_when_policy_is_disabled() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\nexit 0\n");

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "false");

    let adapter = CodexProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected disabled-policy error");

    assert_eq!(error.category, ProviderErrorCategory::Validation);
    assert_eq!(error.code, "disabled-policy");
    assert!(error.message.contains("CODEX_ALLOW_DANGEROUS_ENABLED=true"));
}

#[test]
fn execute_returns_internal_error_when_codex_exits_non_zero() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("codex", "#!/bin/sh\necho execution failed >&2\nexit 17\n");

    let _path = prepend_path(&lock, stub.path());
    let _danger = EnvGuard::set(&lock, "CODEX_ALLOW_DANGEROUS_ENABLED", "true");

    let adapter = CodexProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected execute-failed error");

    assert_eq!(error.category, ProviderErrorCategory::Internal);
    assert_eq!(error.code, "execute-failed");
    let details = error.details.expect("error details");
    assert_eq!(details["exit_code"], 17);
    assert_eq!(details["stderr"], "");
}

#[test]
fn limits_report_single_concurrency() {
    let adapter = CodexProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_concurrency, Some(1));
    assert_eq!(response.max_timeout_ms, None);
    assert_eq!(response.max_input_bytes, None);
}

#[test]
fn auth_state_is_unknown_when_auth_file_cannot_be_resolved() {
    let lock = GlobalStateLock::new();
    let _home = EnvGuard::remove(&lock, "HOME");
    let _auth = EnvGuard::remove(&lock, "CODEX_AUTH_FILE");

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth_state");

    assert_eq!(response.state, AuthStateStatus::Unknown);
    assert_eq!(response.subject, None);
}

#[test]
fn auth_state_is_unauthenticated_when_file_is_missing() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    let missing = stub.path().join("missing-auth.json");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        missing.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth_state");

    assert_eq!(response.state, AuthStateStatus::Unauthenticated);
    assert_eq!(response.subject, None);
}

#[test]
fn auth_state_uses_email_subject_when_present() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    let auth_file = stub.path().join("auth.json");
    write_auth_json(&auth_file, PAYLOAD_ALPHA, "acct_001");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        auth_file.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth_state");

    assert_eq!(response.state, AuthStateStatus::Authenticated);
    assert_eq!(response.subject.as_deref(), Some("alpha@example.com"));
}

#[test]
fn auth_state_falls_back_to_account_id_when_token_claims_are_missing() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    let auth_file = stub.path().join("auth.json");
    std::fs::write(
        &auth_file,
        r#"{"tokens":{"account_id":"acct_only"},"last_refresh":"2026-01-01T00:00:00Z"}"#,
    )
    .expect("write auth json");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        auth_file.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let response = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth_state");

    assert_eq!(response.state, AuthStateStatus::Authenticated);
    assert_eq!(response.subject.as_deref(), Some("acct_only"));
}

#[test]
fn auth_state_returns_invalid_auth_file_error_for_malformed_json() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    let auth_file = stub.path().join("broken.json");
    std::fs::write(&auth_file, "{not-json").expect("write malformed json");
    let _auth = EnvGuard::set(
        &lock,
        "CODEX_AUTH_FILE",
        auth_file.to_str().expect("utf-8 path"),
    );

    let adapter = CodexProviderAdapter::new();
    let error = adapter
        .auth_state(AuthStateRequest::default())
        .expect_err("expected invalid auth file error");

    assert_eq!(error.category, ProviderErrorCategory::Auth);
    assert_eq!(error.code, "invalid-auth-file");
    assert!(error.message.contains("failed to parse auth file"));
    let details = error.details.expect("details");
    assert_eq!(
        details["path"],
        auth_file.to_str().expect("utf-8 path").to_string()
    );
}

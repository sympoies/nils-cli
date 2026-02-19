use agent_provider_claude::ClaudeProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{
    AuthStateRequest, AuthStateStatus, CapabilitiesRequest, ExecuteRequest, HealthStatus,
    HealthcheckRequest, LimitsRequest, ProviderErrorCategory, ProviderMaturity,
};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn metadata_reports_stable_maturity() {
    let adapter = ClaudeProviderAdapter::new();
    let metadata = adapter.metadata();

    assert_eq!(metadata.id, "claude");
    assert_eq!(metadata.contract_version.as_str(), "provider-adapter.v1");
    assert_eq!(metadata.maturity, ProviderMaturity::Stable);
}

#[test]
fn capabilities_require_api_key_for_execute() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
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
        Some("set ANTHROPIC_API_KEY to enable execute capability")
    );
}

#[test]
fn capabilities_explain_invalid_config_when_execute_is_unavailable() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", "not-a-url");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .capabilities(CapabilitiesRequest::default())
        .expect("capabilities");

    let execute = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "execute")
        .expect("execute capability");
    assert_eq!(execute.available, false);
    assert!(
        execute
            .description
            .as_deref()
            .unwrap_or_default()
            .contains("invalid-config")
    );
}

#[test]
fn capabilities_include_experimental_local_cli_flag() {
    let lock = GlobalStateLock::new();
    let stub = StubBinDir::new();
    stub.write_exe("claude", "#!/bin/sh\necho claude 0.0.0\n");
    let _path = prepend_path(&lock, stub.path());
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");

    let adapter = ClaudeProviderAdapter::new();
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

    let local_cli = response
        .capabilities
        .iter()
        .find(|capability| capability.name == "characterization.local-cli")
        .expect("local cli capability");
    assert_eq!(local_cli.available, true);
}

#[test]
fn healthcheck_is_degraded_when_api_key_is_missing() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Degraded);
    assert_eq!(
        response.summary.as_deref(),
        Some("claude adapter is partially ready")
    );
    let details = response.details.expect("details");
    assert_eq!(details["api_key_configured"], false);
    assert_eq!(details["execute_available"], false);
    assert_eq!(details["readiness_reason_code"], "missing-api-key");
    assert_eq!(details["config_error_code"], "missing-api-key");
    assert!(
        details["readiness_reason"]
            .as_str()
            .unwrap_or_default()
            .contains("ANTHROPIC_API_KEY")
    );
}

#[test]
fn healthcheck_is_healthy_when_api_key_is_present() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest {
            timeout_ms: Some(1500),
        })
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Healthy);
    assert_eq!(response.summary.as_deref(), Some("claude adapter is ready"));
    let details = response.details.expect("details");
    assert_eq!(details["api_key_configured"], true);
    assert_eq!(details["requested_timeout_ms"], 1500);
    assert_eq!(details["readiness_reason_code"], "ready");
    assert_eq!(
        details["readiness_reason"],
        "claude adapter is ready for execute"
    );
    assert!(details["config_error_code"].is_null());
}

#[test]
fn healthcheck_is_unhealthy_when_config_is_invalid() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(&lock, "ANTHROPIC_BASE_URL", "not-a-url");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter
        .healthcheck(HealthcheckRequest::default())
        .expect("healthcheck");

    assert_eq!(response.status, HealthStatus::Unhealthy);
    assert_eq!(
        response.summary.as_deref(),
        Some("claude adapter is unavailable")
    );
    let details = response.details.expect("details");
    assert_eq!(details["execute_available"], false);
    assert_eq!(details["api_key_configured"], true);
    assert_eq!(details["readiness_reason_code"], "invalid-config");
    assert_eq!(details["config_error_code"], "invalid-config");
}

#[test]
fn execute_returns_validation_error_for_blank_prompt() {
    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest {
            task: "   ".to_string(),
            input: Some("".to_string()),
            timeout_ms: None,
        })
        .expect_err("expected missing-task");

    assert_eq!(error.category, ProviderErrorCategory::Validation);
    assert_eq!(error.code, "missing-task");
}

#[test]
fn execute_returns_auth_error_when_api_key_is_missing() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let error = adapter
        .execute(ExecuteRequest::new("ping"))
        .expect_err("expected auth error");

    assert_eq!(error.category, ProviderErrorCategory::Auth);
    assert_eq!(error.code, "missing-api-key");
}

#[test]
fn limits_report_configurable_concurrency() {
    let lock = GlobalStateLock::new();
    let _concurrency = EnvGuard::set(&lock, "CLAUDE_MAX_CONCURRENCY", "4");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_concurrency, Some(4));
}

#[test]
fn limits_fall_back_to_default_concurrency_when_env_is_invalid() {
    let lock = GlobalStateLock::new();
    let _concurrency = EnvGuard::set(&lock, "CLAUDE_MAX_CONCURRENCY", "invalid");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_concurrency, Some(2));
}

#[test]
fn limits_report_timeout_when_execute_config_is_parseable() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _timeout = EnvGuard::set(&lock, "CLAUDE_TIMEOUT_MS", "4321");

    let adapter = ClaudeProviderAdapter::new();
    let response = adapter.limits(LimitsRequest::default()).expect("limits");

    assert_eq!(response.max_timeout_ms, Some(4321));
}

#[test]
fn auth_state_tracks_api_key_presence() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let adapter = ClaudeProviderAdapter::new();
    let unauth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(unauth.state, AuthStateStatus::Unauthenticated);

    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key-1234");
    let _subject = EnvGuard::set(&lock, "ANTHROPIC_AUTH_SUBJECT", "claude-user@example.com");
    let _scopes = EnvGuard::set(
        &lock,
        "ANTHROPIC_AUTH_SCOPES",
        "messages:read,messages:write",
    );
    let auth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");
    assert_eq!(auth.state, AuthStateStatus::Authenticated);
    assert_eq!(auth.subject.as_deref(), Some("claude-user@example.com"));
    assert_eq!(
        auth.scopes,
        vec!["messages:read".to_string(), "messages:write".to_string()]
    );
}

#[test]
fn auth_state_uses_masked_subject_when_subject_env_is_absent() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "sk-ant-12345678");
    let _subject = EnvGuard::remove(&lock, "ANTHROPIC_AUTH_SUBJECT");

    let adapter = ClaudeProviderAdapter::new();
    let auth = adapter
        .auth_state(AuthStateRequest::default())
        .expect("auth-state");

    assert_eq!(auth.state, AuthStateStatus::Authenticated);
    assert_eq!(auth.subject.as_deref(), Some("key:***5678"));
}

#[test]
fn provider_core_source_remains_free_of_cli_coupling_markers() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src_dir = manifest_dir.join("src");
    let forbidden_markers = ["nils-agentctl", "clap::"];
    let mut violations = Vec::new();

    for source_file in rust_sources_under(&src_dir) {
        let file_contents = fs::read_to_string(&source_file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", source_file.display()));

        for marker in forbidden_markers {
            if file_contents.contains(marker) {
                let relative = source_file
                    .strip_prefix(&manifest_dir)
                    .unwrap_or(source_file.as_path());
                violations.push(format!("{} contains `{marker}`", relative.display()));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "provider core must stay decoupled from CLI parsing/rendering:\n{}",
        violations.join("\n")
    );
}

#[test]
fn cargo_manifest_has_no_cli_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    let forbidden_entries = [
        "nils-agentctl",
        "clap =",
        "clap.workspace",
        "package = \"clap\"",
    ];
    let mut violations = Vec::new();

    for entry in forbidden_entries {
        if manifest.contains(entry) {
            violations.push(entry);
        }
    }

    assert!(
        violations.is_empty(),
        "provider crate manifest must stay CLI-independent; found forbidden entries: {}",
        violations.join(", ")
    );
}

fn rust_sources_under(root: &Path) -> Vec<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("failed to read directory {}: {err}", dir.display()));

        for entry in entries {
            let entry = entry.unwrap_or_else(|err| {
                panic!("failed to read directory entry in {}: {err}", dir.display())
            });
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }

    files.sort();
    files
}

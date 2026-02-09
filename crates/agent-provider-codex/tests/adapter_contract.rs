use agent_provider_codex::CodexProviderAdapter;
use agent_runtime_core::provider::ProviderAdapterV1;
use agent_runtime_core::schema::{ExecuteRequest, ProviderErrorCategory, ProviderMaturity};
use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
use pretty_assertions::assert_eq;

#[test]
fn metadata_uses_normalized_provider_identity() {
    let adapter = CodexProviderAdapter::new();
    let metadata = adapter.metadata();

    assert_eq!(metadata.id, "codex");
    assert_eq!(metadata.contract_version.as_str(), "provider-adapter.v1");
    assert_eq!(metadata.maturity, ProviderMaturity::Stable);
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

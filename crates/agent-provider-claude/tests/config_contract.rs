use agent_provider_claude::config::{self, ClaudeConfig};
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

#[test]
fn config_requires_api_key() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");

    let error = ClaudeConfig::from_env().expect_err("missing key");
    assert_eq!(error.code, "missing-api-key");
}

#[test]
fn config_uses_defaults_when_optional_values_are_unset() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::remove(&lock, "ANTHROPIC_BASE_URL");
    let _model = EnvGuard::remove(&lock, "CLAUDE_MODEL");
    let _fallback_model = EnvGuard::remove(&lock, "ANTHROPIC_MODEL");
    let _timeout = EnvGuard::remove(&lock, "CLAUDE_TIMEOUT_MS");
    let _max_tokens = EnvGuard::remove(&lock, "CLAUDE_MAX_TOKENS");
    let _retry_max = EnvGuard::remove(&lock, "CLAUDE_RETRY_MAX");

    let config = ClaudeConfig::from_env().expect("config");
    assert_eq!(config.base_url, config::DEFAULT_BASE_URL);
    assert_eq!(config.model, config::DEFAULT_MODEL);
    assert_eq!(config.api_version, config::DEFAULT_API_VERSION);
    assert_eq!(config.timeout_ms, config::DEFAULT_TIMEOUT_MS);
    assert_eq!(config.max_tokens, config::DEFAULT_MAX_TOKENS);
    assert_eq!(config.retry_max, config::DEFAULT_RETRY_MAX);
}

#[test]
fn config_rejects_invalid_numeric_values() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _timeout = EnvGuard::set(&lock, "CLAUDE_TIMEOUT_MS", "invalid");

    let error = ClaudeConfig::from_env().expect_err("invalid config");
    assert_eq!(error.code, "invalid-config");
}

#[test]
fn auth_subject_uses_explicit_subject_then_masked_key() {
    let lock = GlobalStateLock::new();
    let _subject = EnvGuard::set(&lock, "ANTHROPIC_AUTH_SUBJECT", "claude@example.com");
    assert_eq!(
        config::auth_subject().as_deref(),
        Some("claude@example.com")
    );

    let _subject = EnvGuard::remove(&lock, "ANTHROPIC_AUTH_SUBJECT");
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "my-secret-9876");
    assert_eq!(config::auth_subject().as_deref(), Some("key:***9876"));
}

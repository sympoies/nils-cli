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
    let _base_url = EnvGuard::remove(&lock, "ANTHROPIC_BASE_URL");
    let _timeout = EnvGuard::set(&lock, "CLAUDE_TIMEOUT_MS", "invalid");

    let error = ClaudeConfig::from_env().expect_err("invalid config");
    assert_eq!(error.code, "invalid-config");
    assert_eq!(
        error.message,
        "CLAUDE_TIMEOUT_MS must be an integer in milliseconds"
    );
}

#[test]
fn config_rejects_invalid_base_url_without_leaking_raw_value() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _base_url = EnvGuard::set(
        &lock,
        "ANTHROPIC_BASE_URL",
        "https://api.anthropic.com/?token=super-secret",
    );
    let _timeout = EnvGuard::remove(&lock, "CLAUDE_TIMEOUT_MS");
    let _max_tokens = EnvGuard::remove(&lock, "CLAUDE_MAX_TOKENS");
    let _retry_max = EnvGuard::remove(&lock, "CLAUDE_RETRY_MAX");

    let error = ClaudeConfig::from_env().expect_err("invalid base URL");
    assert_eq!(error.code, "invalid-config");
    assert_eq!(
        error.message,
        "ANTHROPIC_BASE_URL must be an absolute http(s) URL without query or fragment components"
    );
    assert!(!error.message.contains("super-secret"));
}

#[test]
fn config_prefers_claude_model_override_over_fallback_model() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _model = EnvGuard::set(&lock, "CLAUDE_MODEL", "claude-opus-primary");
    let _fallback_model = EnvGuard::set(&lock, "ANTHROPIC_MODEL", "claude-sonnet-fallback");
    let _base_url = EnvGuard::remove(&lock, "ANTHROPIC_BASE_URL");
    let _timeout = EnvGuard::remove(&lock, "CLAUDE_TIMEOUT_MS");
    let _max_tokens = EnvGuard::remove(&lock, "CLAUDE_MAX_TOKENS");
    let _retry_max = EnvGuard::remove(&lock, "CLAUDE_RETRY_MAX");

    let config = ClaudeConfig::from_env().expect("config");
    assert_eq!(config.model, "claude-opus-primary");
}

#[test]
fn config_uses_fallback_model_when_primary_override_is_blank() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "test-key");
    let _model = EnvGuard::set(&lock, "CLAUDE_MODEL", "   ");
    let _fallback_model = EnvGuard::set(&lock, "ANTHROPIC_MODEL", "claude-sonnet-fallback");
    let _base_url = EnvGuard::remove(&lock, "ANTHROPIC_BASE_URL");
    let _timeout = EnvGuard::remove(&lock, "CLAUDE_TIMEOUT_MS");
    let _max_tokens = EnvGuard::remove(&lock, "CLAUDE_MAX_TOKENS");
    let _retry_max = EnvGuard::remove(&lock, "CLAUDE_RETRY_MAX");

    let config = ClaudeConfig::from_env().expect("config");
    assert_eq!(config.model, "claude-sonnet-fallback");
}

#[test]
fn auth_state_uses_explicit_subject_then_masked_key_and_trimmed_scopes() {
    let lock = GlobalStateLock::new();
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "my-secret-9876");
    let _subject = EnvGuard::set(&lock, "ANTHROPIC_AUTH_SUBJECT", "claude@example.com");
    let _scopes = EnvGuard::set(&lock, "ANTHROPIC_AUTH_SCOPES", "read, write ,,");
    let state = config::auth_state();
    assert_eq!(state.api_key_configured, true);
    assert_eq!(state.subject.as_deref(), Some("claude@example.com"));
    assert_eq!(state.scopes, vec!["read".to_string(), "write".to_string()]);

    let _subject = EnvGuard::remove(&lock, "ANTHROPIC_AUTH_SUBJECT");
    let state = config::auth_state();
    assert_eq!(state.subject.as_deref(), Some("key:***9876"));
}

#[test]
fn auth_subject_masking_is_panic_safe_for_non_ascii_keys() {
    let lock = GlobalStateLock::new();
    let _subject = EnvGuard::remove(&lock, "ANTHROPIC_AUTH_SUBJECT");
    let _api_key = EnvGuard::set(&lock, "ANTHROPIC_API_KEY", "\u{5BC6}\u{94A5}abcd");

    assert_eq!(config::auth_subject().as_deref(), Some("key:***abcd"));
}

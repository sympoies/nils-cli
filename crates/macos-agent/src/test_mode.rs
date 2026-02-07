use std::time::{SystemTime, UNIX_EPOCH};

pub fn enabled() -> bool {
    env_truthy("CODEX_MACOS_AGENT_TEST_MODE")
}

pub fn timestamp_token() -> String {
    if enabled() {
        if let Some(token) = std::env::var_os("CODEX_MACOS_AGENT_TEST_TIMESTAMP") {
            return token.to_string_lossy().into_owned();
        }
        return "test-timestamp".to_string();
    }

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

pub fn env_truthy(name: &str) -> bool {
    let raw =
        std::env::var_os(name).map(|value| value.to_string_lossy().trim().to_ascii_lowercase());
    matches!(
        raw.as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

#[cfg(test)]
mod tests {
    use nils_test_support::{EnvGuard, GlobalStateLock};

    use super::{enabled, timestamp_token};

    #[test]
    fn enabled_is_false_when_missing() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "CODEX_MACOS_AGENT_TEST_MODE");
        assert!(!enabled());
    }

    #[test]
    fn enabled_accepts_truthy_values() {
        let lock = GlobalStateLock::new();
        for value in ["1", "true", " yes ", "ON"] {
            let _guard = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", value);
            assert!(enabled(), "value should be truthy: {value}");
        }
    }

    #[test]
    fn timestamp_uses_explicit_env_in_test_mode() {
        let lock = GlobalStateLock::new();
        let _test = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_MODE", "1");
        let _ts = EnvGuard::set(&lock, "CODEX_MACOS_AGENT_TEST_TIMESTAMP", "20260101-000000");
        assert_eq!(timestamp_token(), "20260101-000000");
    }
}

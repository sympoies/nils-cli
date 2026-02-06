pub fn is_truthy(input: &str) -> bool {
    matches!(input.to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

pub fn is_truthy_or(input: Option<&str>, default: bool) -> bool {
    input.map(is_truthy).unwrap_or(default)
}

pub fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| is_truthy(value.as_str()))
        .unwrap_or(false)
}

pub fn env_truthy_or(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => is_truthy(value.as_str()),
        Err(_) => default,
    }
}

pub fn no_color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};

    #[test]
    fn is_truthy_matches_expected_values() {
        for value in ["1", "true", "TRUE", "yes", "On"] {
            assert!(is_truthy(value), "expected truthy value: {value}");
        }
    }

    #[test]
    fn is_truthy_rejects_falsey_or_unknown_values() {
        for value in ["", "0", "false", "no", "off", " yes ", "enabled"] {
            assert!(!is_truthy(value), "expected falsey value: {value}");
        }
    }

    #[test]
    fn is_truthy_or_uses_default_when_missing() {
        assert!(is_truthy_or(None, true));
        assert!(!is_truthy_or(None, false));
        assert!(is_truthy_or(Some("1"), false));
    }

    #[test]
    fn env_truthy_reads_process_environment() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_TRUTHY_TEST", "yes");
        assert!(env_truthy("NILS_COMMON_ENV_TRUTHY_TEST"));
    }

    #[test]
    fn env_truthy_or_falls_back_to_default() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "NILS_COMMON_ENV_TRUTHY_OR_TEST");
        assert!(env_truthy_or("NILS_COMMON_ENV_TRUTHY_OR_TEST", true));
        assert!(!env_truthy_or("NILS_COMMON_ENV_TRUTHY_OR_TEST", false));
    }

    #[test]
    fn no_color_enabled_checks_var_presence() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NO_COLOR", "");
        assert!(no_color_enabled());
    }
}

pub fn is_truthy(input: &str) -> bool {
    matches!(input.to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

pub fn is_truthy_or(input: Option<&str>, default: bool) -> bool {
    input.map(is_truthy).unwrap_or(default)
}

fn truthy_from_env(name: &str) -> Option<bool> {
    std::env::var_os(name).map(|value| {
        let value = value.to_string_lossy();
        is_truthy(value.trim())
    })
}

pub fn env_present(name: &str) -> bool {
    std::env::var_os(name).is_some()
}

pub fn env_truthy_if_present(name: &str) -> Option<bool> {
    std::env::var(name)
        .ok()
        .map(|value| is_truthy(value.trim()))
}

pub fn env_truthy(name: &str) -> bool {
    truthy_from_env(name).unwrap_or(false)
}

pub fn env_truthy_or(name: &str, default: bool) -> bool {
    truthy_from_env(name).unwrap_or(default)
}

pub fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

pub fn no_color_enabled() -> bool {
    env_present("NO_COLOR")
}

pub fn no_color_non_empty_enabled() -> bool {
    std::env::var("NO_COLOR")
        .ok()
        .is_some_and(|value| !value.trim().is_empty())
}

pub fn no_color_requested(explicit_no_color: bool) -> bool {
    explicit_no_color || no_color_enabled()
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
    fn env_present_checks_var_presence() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_PRESENT_TEST", "");
        assert!(env_present("NILS_COMMON_ENV_PRESENT_TEST"));
    }

    #[test]
    fn env_truthy_if_present_returns_none_when_missing() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "NILS_COMMON_ENV_TRUTHY_IF_PRESENT_MISSING_TEST");
        assert_eq!(
            env_truthy_if_present("NILS_COMMON_ENV_TRUTHY_IF_PRESENT_MISSING_TEST"),
            None
        );
    }

    #[test]
    fn env_truthy_if_present_parses_trimmed_value() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(
            &lock,
            "NILS_COMMON_ENV_TRUTHY_IF_PRESENT_VALUE_TEST",
            " yes ",
        );
        assert_eq!(
            env_truthy_if_present("NILS_COMMON_ENV_TRUTHY_IF_PRESENT_VALUE_TEST"),
            Some(true)
        );
    }

    #[test]
    fn env_truthy_trims_whitespace() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_TRUTHY_TRIM_TEST", " yes ");
        assert!(env_truthy("NILS_COMMON_ENV_TRUTHY_TRIM_TEST"));
    }

    #[test]
    fn env_truthy_or_falls_back_to_default() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "NILS_COMMON_ENV_TRUTHY_OR_TEST");
        assert!(env_truthy_or("NILS_COMMON_ENV_TRUTHY_OR_TEST", true));
        assert!(!env_truthy_or("NILS_COMMON_ENV_TRUTHY_OR_TEST", false));
    }

    #[test]
    fn env_truthy_or_prefers_present_trimmed_values() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_TRUTHY_OR_VALUE_TEST", " off ");
        assert!(!env_truthy_or("NILS_COMMON_ENV_TRUTHY_OR_VALUE_TEST", true));
    }

    #[test]
    fn env_or_default_prefers_present_value() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_OR_DEFAULT_PRESENT_TEST", "custom");
        assert_eq!(
            env_or_default("NILS_COMMON_ENV_OR_DEFAULT_PRESENT_TEST", "fallback"),
            "custom"
        );
    }

    #[test]
    fn env_or_default_uses_default_when_missing() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "NILS_COMMON_ENV_OR_DEFAULT_MISSING_TEST");
        assert_eq!(
            env_or_default("NILS_COMMON_ENV_OR_DEFAULT_MISSING_TEST", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn no_color_enabled_checks_var_presence() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NO_COLOR", "");
        assert!(no_color_enabled());
    }

    #[test]
    fn no_color_non_empty_enabled_distinguishes_empty_and_non_empty() {
        let lock = GlobalStateLock::new();

        {
            let _guard = EnvGuard::set(&lock, "NO_COLOR", "1");
            assert!(no_color_non_empty_enabled());
        }

        {
            let _guard = EnvGuard::set(&lock, "NO_COLOR", "");
            assert!(!no_color_non_empty_enabled());
        }
    }

    #[test]
    fn no_color_requested_respects_explicit_flag() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::remove(&lock, "NO_COLOR");
        assert!(no_color_requested(true));
        assert!(!no_color_requested(false));
    }

    #[test]
    fn no_color_requested_respects_env_presence() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NO_COLOR", "1");
        assert!(no_color_requested(false));
    }
}

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

pub fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn parse_duration_seconds(raw: &str) -> Option<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let raw = raw.to_ascii_lowercase();
    let (num_part, multiplier): (&str, u64) = match raw.chars().last()? {
        's' => (&raw[..raw.len().saturating_sub(1)], 1),
        'm' => (&raw[..raw.len().saturating_sub(1)], 60),
        'h' => (&raw[..raw.len().saturating_sub(1)], 60 * 60),
        'd' => (&raw[..raw.len().saturating_sub(1)], 60 * 60 * 24),
        'w' => (&raw[..raw.len().saturating_sub(1)], 60 * 60 * 24 * 7),
        ch if ch.is_ascii_digit() => (raw.as_str(), 1),
        _ => return None,
    };

    let num_part = num_part.trim();
    if num_part.is_empty() {
        return None;
    }

    let value = num_part.parse::<u64>().ok()?;
    if value == 0 {
        return None;
    }

    value.checked_mul(multiplier)
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

pub fn prompt_segment_color_enabled(explicit_toggle_env: &str) -> bool {
    use std::io::IsTerminal;

    if no_color_enabled() {
        return false;
    }

    if env_present(explicit_toggle_env) {
        return env_truthy(explicit_toggle_env);
    }

    if env_present("STARSHIP_SESSION_KEY") || env_present("STARSHIP_SHELL") {
        return true;
    }

    std::io::stdout().is_terminal()
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
    fn env_non_empty_returns_none_for_missing_or_blank_values() {
        let lock = GlobalStateLock::new();
        let _missing = EnvGuard::remove(&lock, "NILS_COMMON_ENV_NON_EMPTY_MISSING_TEST");
        assert_eq!(
            env_non_empty("NILS_COMMON_ENV_NON_EMPTY_MISSING_TEST"),
            None
        );

        let _blank = EnvGuard::set(&lock, "NILS_COMMON_ENV_NON_EMPTY_MISSING_TEST", "   ");
        assert_eq!(
            env_non_empty("NILS_COMMON_ENV_NON_EMPTY_MISSING_TEST"),
            None
        );
    }

    #[test]
    fn env_non_empty_returns_trimmed_value_when_present() {
        let lock = GlobalStateLock::new();
        let _guard = EnvGuard::set(&lock, "NILS_COMMON_ENV_NON_EMPTY_VALUE_TEST", "  value  ");
        assert_eq!(
            env_non_empty("NILS_COMMON_ENV_NON_EMPTY_VALUE_TEST"),
            Some("value".to_string())
        );
    }

    #[test]
    fn parse_duration_seconds_accepts_plain_and_suffixed_values() {
        assert_eq!(parse_duration_seconds("45"), Some(45));
        assert_eq!(parse_duration_seconds("45s"), Some(45));
        assert_eq!(parse_duration_seconds("2m"), Some(120));
        assert_eq!(parse_duration_seconds("3h"), Some(10_800));
        assert_eq!(parse_duration_seconds("4d"), Some(345_600));
        assert_eq!(parse_duration_seconds("2w"), Some(1_209_600));
        assert_eq!(parse_duration_seconds(" 7H "), Some(25_200));
    }

    #[test]
    fn parse_duration_seconds_rejects_invalid_inputs() {
        for value in ["", " ", "0", "0s", "s", "-1", "1x", "ms"] {
            assert_eq!(parse_duration_seconds(value), None, "value={value}");
        }
    }

    #[test]
    fn parse_duration_seconds_rejects_overflow() {
        assert_eq!(parse_duration_seconds("18446744073709551615w"), None);
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

    #[test]
    fn prompt_segment_color_enabled_no_color_has_highest_priority() {
        let lock = GlobalStateLock::new();
        let _no_color = EnvGuard::set(&lock, "NO_COLOR", "1");
        let _explicit = EnvGuard::set(&lock, "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED", "1");
        let _session = EnvGuard::set(&lock, "STARSHIP_SESSION_KEY", "session");
        assert!(!prompt_segment_color_enabled(
            "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED"
        ));
    }

    #[test]
    fn prompt_segment_color_enabled_honors_explicit_truthy_and_falsey_values() {
        let lock = GlobalStateLock::new();
        let _no_color = EnvGuard::remove(&lock, "NO_COLOR");
        let _session = EnvGuard::remove(&lock, "STARSHIP_SESSION_KEY");
        let _shell = EnvGuard::remove(&lock, "STARSHIP_SHELL");

        for value in ["1", " true ", "YES", "on"] {
            let _explicit = EnvGuard::set(&lock, "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED", value);
            assert!(
                prompt_segment_color_enabled("NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED"),
                "expected truthy value: {value}"
            );
        }

        for value in ["", " ", "0", "false", "no", "off", "y", "enabled"] {
            let _explicit = EnvGuard::set(&lock, "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED", value);
            assert!(
                !prompt_segment_color_enabled("NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED"),
                "expected falsey value: {value}"
            );
        }
    }

    #[test]
    fn prompt_segment_color_enabled_uses_prompt_markers_when_not_overridden() {
        let lock = GlobalStateLock::new();
        let _no_color = EnvGuard::remove(&lock, "NO_COLOR");
        let _explicit = EnvGuard::remove(&lock, "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED");
        let _session = EnvGuard::set(&lock, "STARSHIP_SESSION_KEY", "session");
        assert!(prompt_segment_color_enabled(
            "NILS_COMMON_PROMPT_SEGMENT_COLOR_ENABLED"
        ));
    }
}

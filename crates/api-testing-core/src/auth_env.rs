use crate::cli_util;

pub fn resolve_env_fallback(keys: &[&str]) -> Option<(String, String)> {
    for &key in keys {
        let Ok(value) = std::env::var(key) else {
            continue;
        };
        if let Some(token) = cli_util::trim_non_empty(&value) {
            return Some((token, key.to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};

    #[test]
    fn resolve_env_fallback_prefers_order() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::set(&lock, "ACCESS_TOKEN", "access");
        let _service = EnvGuard::set(&lock, "SERVICE_TOKEN", "service");

        assert_eq!(
            resolve_env_fallback(&["ACCESS_TOKEN", "SERVICE_TOKEN"]),
            Some(("access".to_string(), "ACCESS_TOKEN".to_string()))
        );
    }

    #[test]
    fn resolve_env_fallback_skips_empty_and_whitespace() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::set(&lock, "ACCESS_TOKEN", "  ");
        let _service = EnvGuard::set(&lock, "SERVICE_TOKEN", "service");

        assert_eq!(
            resolve_env_fallback(&["ACCESS_TOKEN", "SERVICE_TOKEN"]),
            Some(("service".to_string(), "SERVICE_TOKEN".to_string()))
        );
    }

    #[test]
    fn resolve_env_fallback_returns_none_when_missing() {
        let lock = GlobalStateLock::new();
        let _access = EnvGuard::remove(&lock, "ACCESS_TOKEN");
        let _service = EnvGuard::remove(&lock, "SERVICE_TOKEN");

        assert_eq!(
            resolve_env_fallback(&["ACCESS_TOKEN", "SERVICE_TOKEN"]),
            None
        );
    }
}

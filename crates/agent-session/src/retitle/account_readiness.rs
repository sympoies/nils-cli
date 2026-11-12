use crate::CliError;
use crate::retitle::RetitleReadiness;

/// Readiness for an automatic account selection the broker could not complete.
///
/// A broker that answered but offered no account (it rejected the request)
/// means no usable account exists. Any other failure (unavailable, timed out,
/// failed, or unusable response) says nothing about the roster, so it is a
/// broker fault and must not be reported as a missing account.
pub(super) fn selection_failure(error: &CliError) -> RetitleReadiness {
    if error.code() == "codex-account-broker-rejected" {
        RetitleReadiness::unavailable("account_missing", "select_account")
    } else {
        RetitleReadiness::unavailable("account_broker_unavailable", "configure_account_broker")
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use crate::retitle::{CONFIG_ENV, RetitleService};

    const BROKER_ENV: &str = "AGENT_SESSION_CODEX_ACCOUNT_BROKER";
    const SELECTED: &str = r#"{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","plan":"team"}"#;

    fn write_broker(dir: &Path, name: &str, script: &str) -> std::path::PathBuf {
        let broker = dir.join(name);
        std::fs::write(&broker, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&broker, std::fs::Permissions::from_mode(0o700)).unwrap();
        broker
    }

    fn automatic_config(codex_bin: &Path) -> String {
        json!({
            "provider": "codex_subscription",
            "account_selection": "default_with_capacity",
            "codex_bin": codex_bin,
            "model": "gpt-5.6-luna",
        })
        .to_string()
    }

    fn reasons(readiness: &crate::retitle::RetitleReadiness) -> (&str, &str, &str) {
        (
            readiness.status,
            readiness.reason_code,
            readiness.next_action,
        )
    }

    #[test]
    fn automatic_selection_failures_are_classified_by_cause() {
        let lock = nils_test_support::GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let rejecting = write_broker(tmp.path(), "rejecting", "exit 3");
        let malformed = write_broker(tmp.path(), "malformed", "printf '%s\\n' 'not json'");
        let _config = nils_test_support::EnvGuard::set(
            &lock,
            CONFIG_ENV,
            automatic_config(&rejecting).as_str(),
        );

        for (name, argv, expected) in [
            (
                "no selectable account",
                json!([rejecting]),
                ("unavailable", "account_missing", "select_account"),
            ),
            (
                "malformed broker response",
                json!([malformed]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "broker cannot be started",
                json!([tmp.path().join("absent-broker")]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
        ] {
            let _broker = nils_test_support::EnvGuard::set(&lock, BROKER_ENV, &argv.to_string());
            let readiness = RetitleService::from_environment().readiness();
            assert_eq!(reasons(&readiness), expected, "{name}");
        }
    }

    #[test]
    fn repeated_readiness_reads_recover_after_a_transient_broker_failure() {
        let lock = nils_test_support::GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let counter = tmp.path().join("calls");
        // The second broker call returns an unusable response, as a flapping
        // broker would; every other call selects the configured account.
        let broker = write_broker(
            tmp.path(),
            "flapping",
            &format!(
                "n=$(cat '{counter}' 2>/dev/null || echo 0)\nn=$((n + 1))\nprintf '%s' \"$n\" > '{counter}'\nif [ \"$n\" = 2 ]; then printf '%s\\n' 'not json'; else printf '%s\\n' '{SELECTED}'; fi",
                counter = counter.display(),
            ),
        );
        let _config =
            nils_test_support::EnvGuard::set(&lock, CONFIG_ENV, automatic_config(&broker).as_str());
        let _broker =
            nils_test_support::EnvGuard::set(&lock, BROKER_ENV, &json!([broker]).to_string());

        let service = RetitleService::from_environment();
        let observed = (0..3)
            .map(|_| {
                let readiness = service.readiness();
                (readiness.status, readiness.reason_code)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            observed,
            vec![
                ("ready", "ready"),
                ("unavailable", "account_broker_unavailable"),
                ("ready", "ready"),
            ]
        );
    }
}

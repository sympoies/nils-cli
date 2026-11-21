use crate::CliError;
use crate::retitle::RetitleReadiness;

pub(super) const BROKER_UNAVAILABLE_REASON: &str = "account_broker_unavailable";
const BROKER_UNAVAILABLE_ACTION: &str = "configure_account_broker";

/// Classify a broker failure for readiness and request-time account handling.
///
/// Only the reserved no-account exit 10 produces `-rejected` and reports
/// `account_missing`. Other exits, start failures, timeouts and unusable
/// responses are broker faults, which say nothing about the account roster.
pub(super) fn selection_failure(error: &CliError) -> RetitleReadiness {
    if error.code() == "codex-account-broker-rejected" {
        RetitleReadiness::unavailable("account_missing", "select_account")
    } else {
        RetitleReadiness::unavailable(BROKER_UNAVAILABLE_REASON, BROKER_UNAVAILABLE_ACTION)
    }
}

pub(super) fn map_account_broker_error(error: CliError, message: &'static str) -> CliError {
    // A broker timeout already has a bounded request-time recovery contract.
    if error.code() == "codex-account-broker-timeout" {
        return super::provider_timeout();
    }
    let readiness = selection_failure(&error);
    if readiness.reason_code == "account_missing" {
        super::retitle_error(
            "retitle-account-missing",
            message,
            true,
            readiness.next_action,
            "refresh_account",
        )
    } else {
        broker_unavailable()
    }
}

pub(super) fn broker_unavailable() -> CliError {
    CliError::unavailable(
        "retitle-provider-unavailable",
        "title account broker is unavailable",
        Some(serde_json::json!({
            "retryable": true,
            "next_action": BROKER_UNAVAILABLE_ACTION,
            "recovery": {"strategy": BROKER_UNAVAILABLE_ACTION, "safe_to_retry": true},
            "failure_class": BROKER_UNAVAILABLE_REASON,
        })),
    )
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
        let rejecting = write_broker(tmp.path(), "rejecting", "exit 10");
        let crashing = write_broker(tmp.path(), "crashing", "exit 1");
        let malformed = write_broker(tmp.path(), "malformed", "printf '%s\\n' 'not json'");
        let _config = nils_test_support::EnvGuard::set(
            &lock,
            CONFIG_ENV,
            automatic_config(&rejecting).as_str(),
        );

        for (name, argv, expected) in [
            (
                "typed no-account",
                json!([rejecting]),
                ("unavailable", "account_missing", "select_account"),
            ),
            (
                "broker crashes",
                json!([crashing]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "exit-2",
                json!([write_broker(tmp.path(), "exit-2", "exit 2")]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "exit-3",
                json!([write_broker(tmp.path(), "exit-3", "exit 3")]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "exit-7",
                json!([write_broker(tmp.path(), "exit-7", "exit 7")]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "signal",
                json!([write_broker(tmp.path(), "signal", "kill -TERM \"$$\"")]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "null-account",
                json!([write_broker(
                    tmp.path(),
                    "null-account",
                    "printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":null}' "
                )]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
            ),
            (
                "missing-account",
                json!([write_broker(
                    tmp.path(),
                    "missing-account",
                    "printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\"}' "
                )]),
                (
                    "unavailable",
                    "account_broker_unavailable",
                    "configure_account_broker",
                ),
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
            assert_eq!(reasons(&readiness), expected, "{name}: automatic");
            let fixed = json!({
                "provider": "codex_subscription", "account": "acct1",
                "codex_bin": tmp.path().join("rejecting"), "model": "gpt-5.6-luna",
            });
            let _config = nils_test_support::EnvGuard::set(&lock, CONFIG_ENV, &fixed.to_string());
            let readiness = RetitleService::from_environment().readiness();
            assert_eq!(reasons(&readiness), expected, "{name}: configured");
        }
    }

    #[test]
    fn request_time_broker_failures_are_classified_by_cause() {
        let lock = nils_test_support::GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let rejecting = write_broker(tmp.path(), "rejecting", "exit 10");
        let malformed = write_broker(tmp.path(), "malformed", "printf '%s\\n' 'not json'");

        for (broker, expected_code, expected_action) in [
            (rejecting, "retitle-account-missing", "select_account"),
            (
                write_broker(tmp.path(), "exit-1", "exit 1"),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(tmp.path(), "exit-2", "exit 2"),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(tmp.path(), "exit-3", "exit 3"),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(tmp.path(), "exit-7", "exit 7"),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(tmp.path(), "signal", "kill -TERM \"$$\""),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(
                    tmp.path(),
                    "null-account",
                    "printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\",\"account\":null}' ",
                ),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                write_broker(
                    tmp.path(),
                    "missing-account",
                    "printf '%s\\n' '{\"schema_version\":\"agent-session.codex-auth-broker.v1\"}' ",
                ),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                malformed,
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
            (
                tmp.path().join("absent-broker"),
                "retitle-provider-unavailable",
                "configure_account_broker",
            ),
        ] {
            let _broker =
                nils_test_support::EnvGuard::set(&lock, BROKER_ENV, &json!([broker]).to_string());
            for account_config in [
                json!({"account_selection": "default_with_capacity"}),
                json!({"account": "acct1"}),
            ] {
                let mut config = account_config;
                config["provider"] = json!("codex_subscription");
                config["codex_bin"] = json!("/bin/true");
                config["model"] = json!("gpt-5.6-luna");
                let config = crate::retitle::RetitleConfig::parse(&config.to_string()).unwrap();
                let error = crate::retitle::invoke_codex(
                    &config,
                    "Retitle this session",
                    std::time::Duration::from_secs(1),
                )
                .unwrap_err();

                assert_eq!(error.code(), expected_code);
                assert_eq!(error.details().unwrap()["next_action"], expected_action);
                assert_eq!(error.details().unwrap()["retryable"], true);
                if expected_action == "configure_account_broker" {
                    assert_eq!(
                        crate::retitle::provider_failure_class(&error),
                        "account_broker_unavailable"
                    );
                    assert_eq!(
                        error.details().unwrap()["recovery"]["strategy"],
                        "configure_account_broker"
                    );
                }
            }
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

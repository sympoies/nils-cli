use std::os::unix::fs::PermissionsExt;

use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use serde_json::json;

fn select_next(
    script: &str,
) -> Result<Option<crate::codex_account::CodexAccountSummary>, crate::CliError> {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    std::fs::write(&broker, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(&broker, std::fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    crate::codex_account::select_next_account("acct1", &[])
}

#[test]
fn typed_no_account_has_no_failover_target() {
    assert!(select_next("exit 10").unwrap().is_none());
}

#[test]
fn untyped_failures_are_not_empty_failover_selections() {
    let observed = ["exit 1", "exit 2", "exit 3", "exit 7", "kill -TERM \"$$\""].map(|script| {
        select_next(script)
            .err()
            .map(|error| error.code().to_string())
    });
    assert_eq!(
        observed,
        std::array::from_fn::<_, 5, _>(|_| Some("codex-account-broker-unavailable".to_string()))
    );
}

#[test]
fn null_or_missing_success_selection_is_invalid_not_no_account() {
    let observed = [
        r#"printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":null}'"#,
        r#"printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1"}'"#,
    ]
    .map(|script| {
        select_next(script)
            .err()
            .map(|error| error.code().to_string())
    });
    assert_eq!(
        observed,
        std::array::from_fn::<_, 2, _>(|_| Some(
            "codex-account-broker-invalid-response".to_string()
        ))
    );
}

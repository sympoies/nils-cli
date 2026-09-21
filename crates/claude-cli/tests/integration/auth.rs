use crate::support::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[test]
fn auth_status_wraps_public_fields_and_redacts_identity_and_tokens() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
if [ "$*" = "auth status --json" ]; then
  cat <<'JSON'
{"loggedIn":true,"authMethod":"claude.ai","apiProvider":"firstParty","email":"private@example.com","orgId":"private-org-id","orgName":"Private Org","subscriptionType":"team","accessToken":"secret-token"}
JSON
  exit 0
fi
exit 98
"#,
    );

    let output = run(
        &["auth", "status", "--format", "json"],
        &base_options(tmp.path()).with_fake_claude(&bin_dir),
    );

    assert_exit(&output, 0);
    assert!(!stdout(&output).contains("private@example.com"));
    assert!(!stdout(&output).contains("private-org-id"));
    assert!(!stdout(&output).contains("Private Org"));
    assert!(!stdout(&output).contains("secret-token"));
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "claude-cli.auth.v1");
    assert_eq!(payload["command"], "auth status");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["logged_in"], true);
    assert_eq!(payload["result"]["auth_method"], "claude.ai");
    assert_eq!(payload["result"]["api_provider"], "firstParty");
    assert_eq!(payload["result"]["subscription_type"], "team");
}

#[cfg(unix)]
#[test]
fn auth_status_validates_upstream_shape_and_exit_semantics() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
case "${CLAUDE_TEST_AUTH_CASE:-}" in
  logged-out) printf '%s\n' '{"loggedIn":false}'; exit 1 ;;
  invalid-json) printf '%s\n' 'not json'; exit 0 ;;
  null) printf '%s\n' 'null'; exit 0 ;;
  missing) printf '%s\n' '{"authMethod":"claude.ai"}'; exit 0 ;;
  wrong-type) printf '%s\n' '{"loggedIn":"true"}'; exit 0 ;;
  unexpected-exit) printf '%s\n' '{"loggedIn":true}'; exit 2 ;;
  unexpected-empty) exit 2 ;;
  unexpected-diagnostic) printf '%s\n' 'upstream diagnostic'; exit 2 ;;
  inconsistent) printf '%s\n' '{"loggedIn":false}'; exit 0 ;;
  oversized) while :; do printf '%s\n' 'unbounded auth output'; done ;;
  timeout) sleep 30 ;;
esac
exit 99
"#,
    );
    let base = base_options(tmp.path()).with_fake_claude(&bin_dir);

    let logged_out = run(
        &["auth", "status", "--format", "json"],
        &base.clone().with_env("CLAUDE_TEST_AUTH_CASE", "logged-out"),
    );
    assert_exit(&logged_out, 1);
    let payload: Value = serde_json::from_str(&stdout(&logged_out)).expect("logged-out json");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["logged_in"], false);

    for (case, expected_exit, expected_error) in [
        ("invalid-json", 65, "invalid-upstream-output"),
        ("null", 65, "invalid-upstream-shape"),
        ("missing", 65, "invalid-upstream-shape"),
        ("wrong-type", 65, "invalid-upstream-shape"),
        ("unexpected-exit", 1, "unexpected-upstream-status"),
        ("unexpected-empty", 1, "unexpected-upstream-status"),
        ("unexpected-diagnostic", 1, "unexpected-upstream-status"),
        ("inconsistent", 65, "inconsistent-upstream-status"),
        ("oversized", 65, "output-too-large"),
    ] {
        let output = run(
            &["auth", "status", "--format", "json"],
            &base.clone().with_env("CLAUDE_TEST_AUTH_CASE", case),
        );
        assert_exit(&output, expected_exit);
        let payload: Value = serde_json::from_str(&stdout(&output)).expect("error json");
        assert_eq!(payload["ok"], false, "case: {case}");
        assert_eq!(payload["error"]["code"], expected_error, "case: {case}");
    }

    let started = Instant::now();
    let timed_out = run(
        &["auth", "status", "--format", "json"],
        &base.with_env("CLAUDE_TEST_AUTH_CASE", "timeout"),
    );
    assert_exit(&timed_out, 1);
    let payload: Value = serde_json::from_str(&stdout(&timed_out)).expect("timeout json");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "upstream-timeout");
    assert!(started.elapsed() < Duration::from_secs(7));
}

#[cfg(unix)]
#[test]
fn auth_login_and_logout_delegate_exact_argv_and_propagate_status() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let argv_log = tmp.path().join("auth-argv.log");
    let bin_dir = write_fake_claude(
        tmp.path(),
        r#"#!/bin/sh
: > "$CLAUDE_TEST_ARGV_LOG"
for arg in "$@"; do printf '%s\n' "$arg" >> "$CLAUDE_TEST_ARGV_LOG"; done
case "$*" in
  "auth login --console --email operator@example.com") exit 7 ;;
  "auth logout") exit 8 ;;
esac
exit 99
"#,
    );
    let options = base_options(tmp.path())
        .with_fake_claude(&bin_dir)
        .with_env("CLAUDE_TEST_ARGV_LOG", &path_str(&argv_log));

    let login = run(
        &[
            "auth",
            "login",
            "--console",
            "--email",
            "operator@example.com",
        ],
        &options,
    );
    assert_exit(&login, 7);
    assert_eq!(
        std::fs::read_to_string(&argv_log).expect("login argv"),
        "auth\nlogin\n--console\n--email\noperator@example.com\n"
    );

    let logout = run(&["auth", "logout"], &options);
    assert_exit(&logout, 8);
    assert_eq!(
        std::fs::read_to_string(&argv_log).expect("logout argv"),
        "auth\nlogout\n"
    );
}

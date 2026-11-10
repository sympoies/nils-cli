//! Real CLI consumer round trips, independent of the daemon and provider.
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn invoke(root: &Path, args: &[&str]) -> (i32, Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .args(["--state-dir", root.to_str().unwrap(), "--host", "fixture"])
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code().unwrap(),
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            panic!(
                "CLI did not emit JSON: {}",
                String::from_utf8_lossy(&output.stderr)
            )
        }),
    )
}

fn seed(root: &Path) -> Value {
    let session = root.join("sessions/example");
    fs::create_dir_all(&session).unwrap();
    fs::write(
        session.join("session.json"),
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.session.v1", "id":"example", "agent":"claude",
            "mode":"interactive", "title":null, "cwd":"/fixture", "tmux_session":"example",
            "prompt_file":null, "log_file":null, "created_at":"2030-01-01T00:00:00Z",
            "updated_at":"2030-01-01T00:00:00Z"
        }))
        .unwrap(),
    )
    .unwrap();
    let alert = json!({
        "alert_id":"a".repeat(64), "session_id":"example", "incident_id":"a".repeat(64),
        "runtime_incarnation":"fixture-runtime", "runtime_generation":1, "provider":"claude",
        "phase":"incident", "result":"authentication", "recovery_action":"same_account_credential_recheck",
        "recorded_at":"2030-01-01T00:00:00Z", "required_targets":["person"],
        "delivery":{"person":{"state":"pending", "delivered_at":null, "receipt":null}},
        "state":"pending", "delivered_at":null
    });
    fs::write(
        session.join("auth-alerts.json"),
        serde_json::to_vec(&json!({
            "schema_version":"agent-session.auth-alerts.v1", "alerts":[alert]
        }))
        .unwrap(),
    )
    .unwrap();
    alert
}

#[test]
fn consumer_cli_lists_and_acknowledges_delivery_without_a_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    let alert = seed(tmp.path());
    let (code, body) = invoke(tmp.path(), &["auth-alert", "list", "--format", "json"]);
    assert_eq!(code, 0);
    assert_eq!(
        body,
        json!({"schema_version":"cli.agent-session.auth-alert.list.v1", "ok":true,
        "data":{"alerts":[alert],"truncated":false}})
    );
    let id = alert["alert_id"].as_str().unwrap();
    let proof = format!("sha256:{}", "b".repeat(64));
    let args = [
        "auth-alert",
        "ack",
        "example",
        id,
        "--target",
        "person",
        "--receipt",
        &proof,
        "--format",
        "json",
    ];
    let (code, first) = invoke(tmp.path(), &args);
    assert_eq!(code, 0);
    assert_eq!(
        first["schema_version"],
        "cli.agent-session.auth-alert.ack.v1"
    );
    assert_eq!(first["ok"], true);
    assert_eq!(first["data"]["state"], "delivered");
    assert!(first["data"]["delivered_at"].is_string());
    assert_eq!(first["data"]["delivery"]["person"]["receipt"], proof);
    assert_eq!(invoke(tmp.path(), &args), (0, first.clone()));
    let (_, listed) = invoke(tmp.path(), &["auth-alert", "list", "--format", "json"]);
    assert_eq!(listed["data"]["alerts"][0], first["data"]);
    assert!(
        !tmp.path()
            .join("sessions/example/auth-incidents.json")
            .exists()
    );
}

#[test]
fn consumer_cli_retains_leaf_envelopes_for_parse_context_and_operation_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let alert = seed(tmp.path());
    let id = alert["alert_id"].as_str().unwrap();
    let proof = format!("sha256:{}", "b".repeat(64));
    for (args, schema, expected) in [
        (
            vec!["auth-alert", "ack", "--format", "json"],
            "ack",
            "parse-error",
        ),
        (
            vec!["auth-alert", "list", "--limit", "0", "--format", "json"],
            "list",
            "parse-error",
        ),
        (
            vec![
                "auth-alert",
                "ack",
                "example",
                id,
                "--target",
                "person",
                "--receipt",
                "invalid",
                "--format",
                "json",
            ],
            "ack",
            "auth-alert-receipt-invalid",
        ),
        (
            vec![
                "auth-alert",
                "ack",
                "example",
                "missing",
                "--target",
                "person",
                "--receipt",
                &proof,
                "--format",
                "json",
            ],
            "ack",
            "auth-alert-not-found",
        ),
        (
            vec![
                "auth-alert",
                "ack",
                "example",
                id,
                "--target",
                "watchdog",
                "--receipt",
                &proof,
                "--format",
                "json",
            ],
            "ack",
            "auth-alert-target-unconfigured",
        ),
    ] {
        let (code, body) = invoke(tmp.path(), &args);
        assert_ne!(code, 0);
        assert_eq!(
            body["schema_version"],
            format!("cli.agent-session.auth-alert.{schema}.v1")
        );
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], expected);
    }
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-session"));
    nils_test_support::cmd::strip_ambient_managed_session_env(&mut command);
    let output = command
        .args([
            "--state-dir",
            tmp.path().to_str().unwrap(),
            "--host",
            "invalid host",
            "auth-alert",
            "list",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(!output.status.success());
    assert_eq!(
        body["schema_version"],
        "cli.agent-session.auth-alert.list.v1"
    );
    assert_eq!(body["ok"], false);
    fs::write(
        tmp.path().join("sessions/example/auth-alerts.json"),
        "invalid",
    )
    .unwrap();
    let (_, body) = invoke(tmp.path(), &["auth-alert", "list", "--format", "json"]);
    assert_eq!(body["error"]["code"], "auth-alert-store-unavailable");
}

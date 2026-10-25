//! Recipient authority transitions refuse precisely and retain unread payloads.
use std::fs;
use std::path::Path;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::{Value, json};

use super::coordination::{capability, rewrite_registry, seed_brokers, write_private_json};

const TOKEN: &str = "synthetic-recipient-capability-material";
const FOREIGN: &str = "synthetic-foreign-capability-material";
const ROTATED: &str = "synthetic-rotated-capability-material";
const BODY: &str = "SYNTHETIC-MAILBOX-BODY-CANARY";

fn cli(root: &Path, cap: Option<&Path>, command: &[&str]) -> CmdOutput {
    let state = root.join("state");
    let mut args = vec!["--state-dir", state.to_str().unwrap()];
    args.extend_from_slice(command);
    args.extend(["--session", "recipient", "--format", "json"]);
    if let Some(cap) = cap {
        args.extend(["--capability-file", cap.to_str().unwrap()]);
    }
    run_resolved(
        "agent-session",
        &args,
        &CmdOptions::new()
            .without_ambient_managed_session_env()
            .with_env_remove("AGENT_SESSION_CAPABILITY_FILE")
            .with_env("AGENT_SESSION_ID", "recipient")
            .with_env("AGENT_SESSION_RUNTIME_ID", "synthetic-original-incarnation")
            .with_cwd(root),
    )
}

fn fixture(root: &Path) -> std::path::PathBuf {
    let state = root.join("state");
    fs::create_dir(&state).unwrap();
    seed_brokers(
        &state,
        &[
            ("recipient", "synthetic-original-incarnation", TOKEN),
            ("replacement", "synthetic-replacement-incarnation", FOREIGN),
        ],
    );
    let session = state.join("sessions/recipient/session.json");
    let mut record: Value = serde_json::from_slice(&fs::read(&session).unwrap()).unwrap();
    record["lineage"] = json!({
        "schema_version":"agent-session.session-lineage.v1", "parent":null,
        "root":{"machine":"fixture", "session_id":"recipient", "session_created_at":record["created_at"]},
        "depth":0, "starter":{"kind":"operator", "via":"cli"},
        "forge_context":{"initiator":"operator", "role":null}
    });
    write_private_json(&session, &record);
    let now = jiff::Timestamp::now().as_second();
    rewrite_registry(&state, |registry| {
        registry["messages"] = json!([{
            "schema_version":"agent-session.message.v1",
            "message_id":"exact-unread-message",
            "sender_session_id":"sender", "sender_incarnation":"sender-incarnation",
            "recipient_session_id":"recipient",
            "recipient_incarnation":"synthetic-original-incarnation",
            "state":"unread", "revision":1, "reply_to":null, "reply_depth":0,
            "created_at":jiff::Timestamp::from_second(now).unwrap().to_string(),
            "created_at_epoch":now,
            "expires_at":jiff::Timestamp::from_second(now + 3600).unwrap().to_string(),
            "expires_at_epoch":now + 3600,
            "body_bytes":BODY.len(), "body":BODY
        }]);
    });
    let cap = std::path::PathBuf::from(capability(&state, "recipient"));
    assert_eq!(cli(root, Some(&cap), &["broker", "identity"]).code, 0);
    cap
}

fn stored(root: &Path) -> Value {
    serde_json::from_slice(&fs::read(root.join("state/coordination/registry.json")).unwrap())
        .unwrap()
}

fn assert_refusal(output: &CmdOutput, reason: &str, root: &Path) {
    assert_ne!(output.code, 0);
    let envelope = output.stdout_json();
    let error = &envelope["error"];
    assert_eq!(error["code"], "coordination-unauthorized");
    assert_eq!(error["details"]["reason_code"], reason);
    assert_eq!(error["details"]["retryable"], false);
    assert!(error["hint"].as_str().unwrap().contains("preserve"));
    assert!(error["hint"].as_str().unwrap().contains(reason));
    let text = format!("{}{}", output.stdout_text(), output.stderr_text());
    for canary in [
        TOKEN,
        FOREIGN,
        ROTATED,
        BODY,
        "synthetic-original-incarnation",
        "synthetic-replacement-incarnation",
        root.to_str().unwrap(),
    ] {
        assert!(
            !text.contains(canary),
            "refusal exposed private fixture data"
        );
    }
}

#[test]
fn mailbox_authority_runtime_transition_preserves_exact_unread_message() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cap = fixture(root);
    let inbox = cli(root, Some(&cap), &["message", "inbox", "--state", "unread"]);
    assert_eq!(inbox.code, 0);
    assert_eq!(
        inbox.stdout_json()["data"]["messages"][0]["message_id"],
        "exact-unread-message"
    );
    let path = root.join("state/sessions/recipient/session.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut transitioned = original.clone();
    transitioned["runtime"]["launch_id"] = json!("synthetic-replacement-incarnation");
    write_private_json(&path, &transitioned);
    let before = stored(root)["messages"].clone();
    for command in [
        vec!["message", "inbox", "--state", "unread"],
        vec!["message", "show", "--message", "exact-unread-message"],
        vec!["broker", "identity"],
    ] {
        assert_refusal(
            &cli(root, Some(&cap), &command),
            "runtime-incarnation-mismatch",
            root,
        );
    }
    assert_eq!(stored(root)["messages"], before);
    assert_eq!(stored(root)["messages"][0]["state"], "unread");

    // An unrelated replacement session still authenticates on the same host.
    let replacement_cap = capability(&root.join("state"), "replacement");
    let output = run_resolved(
        "agent-session",
        &[
            "--state-dir",
            root.join("state").to_str().unwrap(),
            "message",
            "inbox",
            "--session",
            "replacement",
            "--capability-file",
            &replacement_cap,
            "--format",
            "json",
        ],
        &CmdOptions::new()
            .without_ambient_managed_session_env()
            .with_cwd(root),
    );
    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_json()["data"]["messages"], json!([]));

    // Restore only synthetic authority to prove refusal did not consume mail.
    // This is a fixture assertion, not a production recovery procedure.
    write_private_json(&path, &original);
    let shown = cli(
        root,
        Some(&cap),
        &["message", "show", "--message", "exact-unread-message"],
    );
    assert_eq!(shown.code, 0);
    assert_eq!(shown.stdout_json()["data"]["body"]["text"], BODY);
}

#[test]
fn mailbox_authority_capability_transition_distinguishes_invalid_and_foreign_authority() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cap = fixture(root);
    let supplied = root.join("supplied-capability-canary");
    nils_common::fs::write_atomic(&supplied, TOKEN.as_bytes(), 0o600).unwrap();
    let inbox = cli(
        root,
        Some(&supplied),
        &["message", "inbox", "--state", "unread"],
    );
    assert_eq!(inbox.code, 0);
    // A broker-issued replacement invalidates the copied old token.
    nils_common::fs::write_atomic(&cap, ROTATED.as_bytes(), 0o600).unwrap();
    rewrite_registry(&root.join("state"), |registry| {
        registry["brokers"]["recipient"]["capability_digest"] =
            json!(super::coordination::digest(ROTATED));
    });
    let messages = stored(root)["messages"].clone();
    let show = ["message", "show", "--message", "exact-unread-message"];
    for command in [
        &show[..],
        &["broker", "identity"][..],
        &["message", "inbox"][..],
    ] {
        assert_refusal(
            &cli(root, Some(&supplied), command),
            "capability-mismatch",
            root,
        );
        assert_refusal(&cli(root, None, command), "capability-file-missing", root);
        assert_refusal(
            &cli(root, Some(&root.join("missing")), command),
            "capability-file-invalid",
            root,
        );
        let foreign = capability(&root.join("state"), "replacement");
        assert_refusal(
            &cli(root, Some(Path::new(&foreign)), command),
            "capability-mismatch",
            root,
        );
        nils_common::fs::write_atomic(&supplied, b"invalid", 0o600).unwrap();
        assert_refusal(
            &cli(root, Some(&supplied), command),
            "capability-file-invalid",
            root,
        );
        nils_common::fs::write_atomic(&supplied, TOKEN.as_bytes(), 0o600).unwrap();
    }
    assert_eq!(stored(root)["messages"], messages);
    let shown = cli(root, Some(&cap), &show);
    assert_eq!(shown.code, 0);
    assert_eq!(shown.stdout_json()["data"]["body"]["text"], BODY);
}

#[test]
fn mailbox_authority_broker_transition_refuses_without_consuming_mail() {
    for reason in [
        "broker-not-ready",
        "broker-capability-unavailable",
        "broker-identity-unavailable",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let cap = fixture(root);
        let supplied = root.join("copied-current-capability");
        nils_common::fs::write_atomic(&supplied, TOKEN.as_bytes(), 0o600).unwrap();
        let inbox = cli(
            root,
            Some(&supplied),
            &["message", "inbox", "--state", "unread"],
        );
        assert_eq!(inbox.code, 0);
        assert_eq!(
            inbox.stdout_json()["data"]["messages"][0]["message_id"],
            "exact-unread-message"
        );
        match reason {
            "broker-not-ready" => rewrite_registry(&root.join("state"), |registry| {
                registry["brokers"]["recipient"]["state"] = json!("recovering");
            }),
            "broker-capability-unavailable" => {
                fs::remove_file(&cap).unwrap();
            }
            _ => rewrite_registry(&root.join("state"), |registry| {
                registry["brokers"]
                    .as_object_mut()
                    .unwrap()
                    .remove("recipient");
            }),
        }
        let before = stored(root)["messages"].clone();
        for command in [
            vec!["message", "inbox", "--state", "unread"],
            vec!["message", "show", "--message", "exact-unread-message"],
            vec!["broker", "identity"],
        ] {
            assert_refusal(&cli(root, Some(&supplied), &command), reason, root);
        }
        assert_eq!(stored(root)["messages"], before);
    }
}

use super::coordination::{capability, data, run, seed_brokers, write_private_json};
use nils_test_support::cmd::CmdOutput;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, path::Path};

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().unwrap();
    seed_brokers(
        tmp.path(),
        &[
            ("alpha", "inc-alpha", "alpha-private-fixture-capability"),
            (
                "beta",
                "inc-beta",
                "beta-private-fixture-capability-material",
            ),
        ],
    );
    fs::write(tmp.path().join("body"), "reply deadline fixture").unwrap();
    tmp
}

fn send(root: &Path, key: &str, extra: &[&str]) -> CmdOutput {
    let cap = capability(root, "alpha");
    let body = root.join("body");
    let mut args = vec![
        "--state-dir",
        root.to_str().unwrap(),
        "message",
        "send",
        "--from",
        "alpha",
        "--to",
        "beta",
        "--capability-file",
        &cap,
        "--body-file",
        body.to_str().unwrap(),
        "--idempotency-key",
        key,
        "--format",
        "json",
    ];
    args.extend(extra);
    run(root, &args)
}

fn audit(root: &Path) -> Value {
    let output = run(
        root,
        &[
            "--state-dir",
            root.to_str().unwrap(),
            "message",
            "audit",
            "--include-healthy",
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    let envelope = output.stdout_json();
    assert_eq!(
        envelope["schema_version"],
        "cli.agent-session.message-audit.v1"
    );
    assert_eq!(
        envelope["data"]["schema_version"],
        "agent-session.mail-audit.v1"
    );
    envelope["data"].clone()
}

fn row(page: &Value, id: &str) -> Value {
    page["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["message_id"] == id)
        .unwrap()
        .clone()
}

#[test]
fn reply_deadline_send_without_flag_has_no_obligation_or_new_metadata() {
    let tmp = fixture();
    let sent = send(tmp.path(), "no-deadline", &[]);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let message = data(&sent);
    assert_eq!(message["state"], "unread");
    assert!(message.get("reply_by").is_none());
    let registry: Value =
        serde_json::from_slice(&fs::read(tmp.path().join("coordination/registry.json")).unwrap())
            .unwrap();
    assert!(registry["messages"][0].get("reply_by_epoch").is_none());
    let record = row(&audit(tmp.path()), message["message_id"].as_str().unwrap());
    assert!(record.get("reply_by").is_none());
    assert!(
        !record["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
}

#[test]
fn reply_deadline_becomes_overdue_after_deadline_and_reply_to_clears_it() {
    let tmp = fixture();
    let sent = send(tmp.path(), "deadline", &["--reply-by", "1h"]);
    assert_eq!(sent.code, 0, "{}", sent.stdout_text());
    let message = data(&sent);
    let id = message["message_id"].as_str().unwrap();
    let path = tmp.path().join("coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        registry["messages"][0]["reply_by_epoch"].as_i64().unwrap()
            - registry["messages"][0]["created_at_epoch"]
                .as_i64()
                .unwrap(),
        3600
    );
    let before = fs::read(&path).unwrap();
    let record = row(&audit(tmp.path()), id);
    assert_eq!(record["reply_by"], message["reply_by"]);
    assert_eq!(record["replied_at"], Value::Null);
    assert!(
        !record["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
    assert_eq!(fs::read(&path).unwrap(), before, "audit is observational");
    // Pin an elapsed deadline without wall-clock sleeps; read/ack is not a reply.
    registry["messages"][0]["reply_by_epoch"] = json!(1);
    registry["messages"][0]["state"] = json!("acknowledged");
    write_private_json(&path, &registry);
    let record = row(&audit(tmp.path()), id);
    assert!(
        record["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
    assert_eq!(record["reply_by"], "1970-01-01T00:00:01Z");
    let cap = capability(tmp.path(), "beta");
    let body = tmp.path().join("body");
    let reply = run(
        tmp.path(),
        &[
            "--state-dir",
            tmp.path().to_str().unwrap(),
            "message",
            "send",
            "--from",
            "beta",
            "--to",
            "alpha",
            "--reply-to",
            id,
            "--capability-file",
            &cap,
            "--body-file",
            body.to_str().unwrap(),
            "--idempotency-key",
            "answer-0001",
            "--format",
            "json",
        ],
    );
    assert_eq!(reply.code, 0, "{}", reply.stdout_text());
    let record = row(&audit(tmp.path()), id);
    assert!(
        !record["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
    assert!(record["replied_at"].is_string());
    // Clearing is durable even if the answer is later compacted from history.
    let mut registry: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    registry["messages"]
        .as_array_mut()
        .unwrap()
        .retain(|m| m["message_id"] == id);
    write_private_json(&path, &registry);
    assert!(
        !row(&audit(tmp.path()), id)["anomalies"]
            .as_array()
            .unwrap()
            .contains(&json!("overdue-reply"))
    );
}

#[test]
fn reply_deadline_duration_is_bounded_actionable_and_idempotent() {
    let tmp = fixture();
    for duration in ["0", "8d", "-1s", "1.5h", "soon", "18446744073709551615d"] {
        let output = send(tmp.path(), "invalid", &[&format!("--reply-by={duration}")]);
        assert_eq!(output.code, 64, "{}", output.stdout_text());
        let error = &output.stdout_json()["error"];
        assert_eq!(error["code"], "mailbox-reply-by-invalid");
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("1 second to 7 days")
        );
        assert!(error["message"].as_str().unwrap().contains("1h"));
    }
    let output = send(tmp.path(), "maximum-deadline", &["--reply-by", "7d"]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(
        data(&send(tmp.path(), "maximum-deadline", &["--reply-by", "7d"])),
        data(&output)
    );
    let conflict = send(tmp.path(), "maximum-deadline", &["--reply-by", "1s"]);
    assert_eq!(
        conflict.stdout_json()["error"]["code"],
        "idempotency-key-reused"
    );
    let registry: Value =
        serde_json::from_slice(&fs::read(tmp.path().join("coordination/registry.json")).unwrap())
            .unwrap();
    assert_eq!(registry["messages"].as_array().unwrap().len(), 1);
}

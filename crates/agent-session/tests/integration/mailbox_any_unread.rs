use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use nils_test_support::cmd::{CmdOptions, CmdOutput, run_resolved};
use pretty_assertions::{assert_eq, assert_ne};
use serde_json::Value;

use super::coordination::{
    capability, data, load_coordination_registry, seed_brokers, write_private_json,
};

const BODY_CANARY: &str = "ANY-UNREAD-BODY-MUST-STAY-PRIVATE";

fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let state = tmp.path().join("state");
    fs::create_dir(&state).expect("state directory");
    seed_brokers(
        &state,
        &[
            (
                "sender",
                "sender-incarnation",
                "fixture-sender-capability-material-0000000001",
            ),
            (
                "recipient",
                "recipient-incarnation",
                "fixture-recipient-capability-material-0000000001",
            ),
        ],
    );
    tmp
}

fn run(tmp: &Path, args: &[&str]) -> CmdOutput {
    run_resolved(
        "agent-session",
        args,
        &CmdOptions::new()
            .without_ambient_managed_session_env()
            .with_cwd(tmp),
    )
}

fn send(tmp: &Path, id: &str) -> String {
    let now_millis = jiff::Timestamp::now().as_millisecond();
    send_at(tmp, id, now_millis)
}

fn send_at(tmp: &Path, id: &str, now_millis: i64) -> String {
    let state = tmp.join("state");
    let now = now_millis / 1000;
    let path = state.join("coordination/registry.json");
    let mut registry: Value =
        serde_json::from_slice(&fs::read(&path).expect("registry")).expect("registry JSON");
    registry["messages"]
        .as_array_mut()
        .expect("message list")
        .push(serde_json::json!({
            "schema_version": "agent-session.message.v1",
            "message_id": id,
            "sender_session_id": "sender",
            "sender_incarnation": "sender-incarnation",
            "recipient_session_id": "recipient",
            "recipient_incarnation": "recipient-incarnation",
            "state": "unread",
            "revision": 1,
            "reply_to": null,
            "reply_depth": 0,
            "created_at": jiff::Timestamp::from_millisecond(now_millis).unwrap().to_string(),
            "created_at_epoch": now,
            "created_at_epoch_millis": now_millis,
            "expires_at": jiff::Timestamp::from_second(now + 3600).unwrap().to_string(),
            "expires_at_epoch": now + 3600,
            "body_bytes": BODY_CANARY.len(),
            "body": BODY_CANARY
        }));
    write_private_json(&path, &registry);
    id.to_string()
}

fn wait(tmp: &Path, cursor: Option<&str>) -> CmdOutput {
    let state = tmp.join("state");
    let cap = capability(&state, "recipient");
    let mut args = vec![
        "--state-dir",
        state.to_str().unwrap(),
        "message",
        "wait",
        "--session",
        "recipient",
        "--any-unread",
        "--timeout",
        "1s",
        "--capability-file",
        &cap,
        "--format",
        "json",
    ];
    if let Some(cursor) = cursor {
        args.extend(["--newer-than", cursor]);
    }
    run(tmp, &args)
}

fn assert_arrival(output: &CmdOutput, id: &str) -> Value {
    assert_eq!(
        output.code,
        0,
        "stdout={} stderr={}",
        output.stdout_text(),
        output.stderr_text()
    );
    assert!(!output.stdout_text().contains(BODY_CANARY));
    assert!(!output.stderr_text().contains(BODY_CANARY));
    let result = data(output);
    let rows = result["messages"].as_array().expect("metadata messages");
    assert!(rows.iter().any(|row| row["message_id"] == id));
    for row in rows {
        assert_eq!(row["state"], "unread");
        assert!(row.get("body").is_none());
        assert!(row["revision"].is_u64());
        assert!(row["created_at"].is_string());
        assert!(row["sender"].is_object());
    }
    result
}

#[test]
fn any_unread_wait_returns_existing_unread_metadata_without_reading() {
    let tmp = fixture();
    let id = send(tmp.path(), "existing-unread-0001");
    let result = assert_arrival(&wait(tmp.path(), None), &id);
    assert!(result["cursor"].is_string());
    let registry = load_coordination_registry(&tmp.path().join("state"));
    assert_eq!(registry["messages"][0]["state"], "unread");
    assert_eq!(registry["messages"][0]["revision"], 1);
}

#[test]
fn any_unread_wait_timeout_is_distinct_from_arrival_and_usage_error() {
    let tmp = fixture();
    let output = wait(tmp.path(), None);
    assert_ne!(output.code, 0);
    assert_ne!(
        output.code, 64,
        "supported wait selector must not be a usage error"
    );
    assert_eq!(output.stdout_json()["error"]["code"], "wait-timeout");
    assert_eq!(output.code, 69, "timeout has a distinct exit status");
    let details = &output.stdout_json()["error"]["details"];
    assert!(details["retryable"].is_boolean());
    assert!(details["next_action"].is_string());
    assert!(details["recovery"].is_object());
    assert_eq!(details["retryable"], true);
    assert_eq!(details["next_action"], "retry_wait");
}

#[test]
fn any_unread_wait_cursor_excludes_existing_mail_and_includes_new_arrival() {
    let tmp = fixture();
    let old = send(tmp.path(), "0d2d87fc-7ec2-4b92-a248-4ed17ea2d407");
    let first = assert_arrival(&wait(tmp.path(), None), &old);
    let cursor = first["cursor"].as_str().expect("arrival cursor");
    let unchanged = wait(tmp.path(), Some(cursor));
    assert_ne!(unchanged.code, 0);
    assert_eq!(unchanged.stdout_json()["error"]["code"], "wait-timeout");
    let new = send(tmp.path(), "4fb946db-461d-47ed-bd10-2df9b611f4b2");
    let arrived = assert_arrival(&wait(tmp.path(), Some(cursor)), &new);
    assert!(
        arrived["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["message_id"] != old)
    );
    let registry = load_coordination_registry(&tmp.path().join("state"));
    assert!(
        registry["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["state"] == "unread")
    );
}

#[test]
fn any_unread_wait_cursor_stays_valid_after_acknowledged_mail_is_replaced() {
    let tmp = fixture();
    let ids: Vec<_> = (0..256)
        .map(|index| send(tmp.path(), &format!("cursor-churn-{index:04}")))
        .collect();
    let first = assert_arrival(&wait(tmp.path(), None), ids.last().unwrap());
    let cursor = first["cursor"].as_str().expect("full-batch cursor");

    let registry_path = tmp.path().join("state/coordination/registry.json");
    let mut registry: Value = serde_json::from_slice(&fs::read(&registry_path).expect("registry"))
        .expect("registry JSON");
    for message in registry["messages"].as_array_mut().expect("message list") {
        message["state"] = Value::String("acknowledged".to_string());
        message["revision"] = Value::from(message["revision"].as_u64().expect("revision") + 1);
    }
    write_private_json(&registry_path, &registry);

    let replacement = send(tmp.path(), "cursor-churn-replacement-0001");
    let arrived = assert_arrival(&wait(tmp.path(), Some(cursor)), &replacement);
    assert_eq!(arrived["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn any_unread_wait_cursor_includes_later_same_millisecond_message_with_lower_id() {
    let tmp = fixture();
    let timestamp = jiff::Timestamp::now().as_millisecond();
    let prior = send_at(
        tmp.path(),
        "4fb946db-461d-47ed-bd10-2df9b611f4b2",
        timestamp,
    );
    let first = assert_arrival(&wait(tmp.path(), None), &prior);
    let cursor = first["cursor"].as_str().expect("arrival cursor");

    let later = send_at(
        tmp.path(),
        "0d2d87fc-7ec2-4b92-a248-4ed17ea2d407",
        timestamp,
    );
    let arrived = assert_arrival(&wait(tmp.path(), Some(cursor)), &later);
    assert!(
        arrived["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["message_id"] != prior)
    );
}

struct WaitChild(Child);
impl Drop for WaitChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn any_unread_wait_accepts_arrival_in_one_waiter_process() {
    let tmp = fixture();
    let state = tmp.path().join("state");
    let cap = capability(&state, "recipient");
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command
        .current_dir(tmp.path())
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "message",
            "wait",
            "--session",
            "recipient",
            "--any-unread",
            "--timeout",
            "5s",
            "--capability-file",
            &cap,
            "--format",
            "json",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("AGENT_SESSION_") || key == "AGENT_HOOK_BIN" {
            command.env_remove(key);
        }
    }
    let mut child = WaitChild(command.spawn().expect("spawn waiter"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        child.0.try_wait().expect("waiter status").is_none(),
        "empty inbox waiter must stay alive"
    );
    #[cfg(target_os = "linux")]
    assert!(
        fs::read_to_string(format!(
            "/proc/{}/task/{}/children",
            child.0.id(),
            child.0.id()
        ))
        .expect("waiter children")
        .trim()
        .is_empty(),
        "waiter must not spawn periodic inbox processes"
    );
    let id = send(tmp.path(), "pending-arrival-0001");
    let mut stdout = String::new();
    use std::io::Read;
    child
        .0
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut stdout)
        .expect("wait output");
    let status = child.0.wait().expect("waiter exit");
    assert!(status.success(), "{stdout}");
    let result: Value = serde_json::from_str(&stdout).expect("JSON output");
    assert!(
        result["data"]["messages"]
            .as_array()
            .expect("metadata")
            .iter()
            .any(|row| row["message_id"] == id)
    );
    assert!(!stdout.contains(BODY_CANARY));
}

#[test]
fn any_unread_wait_requires_the_exact_recipient_capability() {
    let tmp = fixture();
    send(tmp.path(), "capability-fence-0001");
    let state = tmp.path().join("state");
    let wrong = capability(&state, "sender");
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "message",
            "wait",
            "--session",
            "recipient",
            "--any-unread",
            "--timeout",
            "1s",
            "--capability-file",
            &wrong,
            "--format",
            "json",
        ],
    );
    assert_ne!(output.code, 0);
    assert_eq!(
        output.stdout_json()["error"]["code"],
        "coordination-unauthorized"
    );
}

#[test]
fn any_unread_wait_rejects_a_cursor_from_another_recipient() {
    let tmp = fixture();
    let message_id = send(tmp.path(), "cursor-recipient-binding-0001");
    let first = assert_arrival(&wait(tmp.path(), None), &message_id);
    let cursor = first["cursor"].as_str().expect("arrival cursor");
    let state = tmp.path().join("state");
    let cap = capability(&state, "sender");
    let output = run(
        tmp.path(),
        &[
            "--state-dir",
            state.to_str().unwrap(),
            "message",
            "wait",
            "--session",
            "sender",
            "--any-unread",
            "--newer-than",
            cursor,
            "--timeout",
            "1s",
            "--capability-file",
            &cap,
            "--format",
            "json",
        ],
    );
    assert_eq!(output.code, 65);
    assert_eq!(output.stdout_json()["error"]["code"], "cursor-invalid");
}

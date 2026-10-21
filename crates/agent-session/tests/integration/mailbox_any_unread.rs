use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use base64::Engine as _;
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

fn spawn_waiter(tmp: &Path) -> WaitChild {
    spawn_waiter_for(tmp, "5s")
}

fn spawn_waiter_for(tmp: &Path, timeout: &str) -> WaitChild {
    let state = tmp.join("state");
    let cap = capability(&state, "recipient");
    let mut command = Command::new(nils_test_support::bin::resolve("agent-session"));
    command
        .current_dir(tmp)
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "message",
            "wait",
            "--session",
            "recipient",
            "--any-unread",
            "--timeout",
            timeout,
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
    WaitChild(command.spawn().expect("spawn waiter"))
}

#[test]
fn any_unread_wait_accepts_arrival_in_one_waiter_process() {
    let tmp = fixture();
    let mut child = spawn_waiter(tmp.path());
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

// Count the waiter's own CPU time, including its notification backend threads.
// A wall-time timeout alone cannot distinguish sleeping from the self-wake loop.
#[cfg(target_os = "linux")]
fn cpu_ticks(child: &Child) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{}/stat", child.id())).expect("waiter stat");
    let fields: Vec<_> = stat
        .rsplit_once(") ")
        .expect("stat command")
        .1
        .split_whitespace()
        .collect();
    fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap()
}

#[test]
#[cfg(target_os = "linux")]
fn any_unread_wait_idle_cpu_is_bounded() {
    let tmp = fixture();
    let mut child = spawn_waiter_for(tmp.path(), "60s");
    // Process start-up burns CPU for a host-dependent time. Measure only once
    // the waiter has used none for three consecutive samples; a self-wake
    // loop never settles, so the budget below still catches it.
    let settle_deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut quiet_samples = 0;
    let mut last = cpu_ticks(&child.0);
    while quiet_samples < 3 && std::time::Instant::now() < settle_deadline {
        std::thread::sleep(Duration::from_millis(200));
        assert!(child.0.try_wait().unwrap().is_none());
        let current = cpu_ticks(&child.0);
        quiet_samples = if current == last {
            quiet_samples + 1
        } else {
            0
        };
        last = current;
    }
    let before = cpu_ticks(&child.0);
    let window = Duration::from_millis(750);
    std::thread::sleep(window);
    assert!(child.0.try_wait().unwrap().is_none());
    let consumed = cpu_ticks(&child.0) - before;
    // SAFETY: sysconf with _SC_CLK_TCK takes no pointer arguments.
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    assert!(ticks_per_second > 0);
    let budget = (window.as_secs_f64() * ticks_per_second as f64 / 4.0).ceil() as u64;
    assert!(
        consumed <= budget,
        "idle waiter consumed {consumed} CPU ticks; budget {budget}"
    );
}

#[test]
fn any_unread_wait_all_waiters_observe_atomic_registry_arrival() {
    let tmp = fixture();
    let mut children: Vec<_> = (0..5).map(|_| spawn_waiter(tmp.path())).collect();
    std::thread::sleep(Duration::from_millis(300));
    for child in &mut children {
        assert!(child.0.try_wait().unwrap().is_none(), "idle waiter exited");
    }
    // Publish through an atomic replacement, as the registry writer does.
    let id = "multi-waiter-arrival";
    let staged = fixture();
    send(staged.path(), id);
    let root = tmp.path().join("state/coordination");
    let next = root.join("registry.next");
    fs::copy(
        staged.path().join("state/coordination/registry.json"),
        &next,
    )
    .unwrap();
    fs::rename(next, root.join("registry.json")).unwrap();
    for mut child in children {
        use std::io::Read;
        let mut stdout = String::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut stdout)
            .unwrap();
        assert!(child.0.wait().unwrap().success(), "{stdout}");
        let output: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(output["data"]["messages"][0]["message_id"], id);
        assert!(!stdout.contains(BODY_CANARY));
    }
}

#[test]
fn any_unread_wait_excludes_previous_incarnation_unread_mail() {
    let tmp = fixture();
    send(tmp.path(), "previous-incarnation-message");
    let registry_path = tmp.path().join("state/coordination/registry.json");
    let mut registry = load_coordination_registry(&tmp.path().join("state"));
    registry["messages"][0]["recipient_incarnation"] = Value::from("previous-incarnation");
    write_private_json(&registry_path, &registry);
    let output = wait(tmp.path(), None);
    assert_eq!(output.code, 69);
    assert_eq!(output.stdout_json()["error"]["code"], "wait-timeout");
    let current = send(tmp.path(), "current-incarnation-message");
    let result = assert_arrival(&wait(tmp.path(), None), &current);
    assert_eq!(result["messages"].as_array().unwrap().len(), 1);
}

#[test]
fn any_unread_wait_rejects_independently_mismatched_cursor_fields() {
    let tmp = fixture();
    let id = send(tmp.path(), "cursor-independent-binding");
    let first = assert_arrival(&wait(tmp.path(), None), &id);
    let codec = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let original: Value =
        serde_json::from_slice(&codec.decode(first["cursor"].as_str().unwrap()).unwrap()).unwrap();
    for field in ["recipient_session_id", "recipient_incarnation"] {
        let mut cursor = original.clone();
        cursor[field] = Value::from("other-recipient-binding");
        let encoded = codec.encode(serde_json::to_vec(&cursor).unwrap());
        let output = wait(tmp.path(), Some(&encoded));
        assert_eq!(output.code, 65, "{field}: {}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "cursor-invalid",
            "{field}"
        );
    }
}

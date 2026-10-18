use super::*;
use pretty_assertions::assert_eq;
use std::os::fd::AsRawFd;

const OPERATION: &str = "c7d75b5a-9ad5-4d99-9004-afd6e806d301";

fn diagnostics(value: &Value) {
    assert!(uuid::Uuid::parse_str(value["operation_id"].as_str().expect("operation id")).is_ok());
    let wait = value["diagnostics"]["coordination_lock_wait_ms"]
        .as_u64()
        .expect("lock wait");
    let elapsed = value["diagnostics"]["command_elapsed_ms"]
        .as_u64()
        .expect("command elapsed");
    assert!(elapsed >= wait, "{value}");
}

fn route(serve: &Serve, body: Value) -> (u16, Value) {
    let response = reqwest::blocking::Client::new()
        .post(format!(
            "http://{}/sessions/{SESSION}/console-start/v1",
            serve.addr
        ))
        .bearer_auth(CAPABILITY)
        .json(&body)
        .send()
        .expect("route response");
    (
        response.status().as_u16(),
        response.json().expect("route json"),
    )
}

#[test]
fn console_operation_success_and_failures_have_separate_timing() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child","agent":"claude"}}}),
    );
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    diagnostics(&output.stdout_json()["data"]);
    aggregator.reply(String::new());
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    diagnostics(&error_of(&output)["details"]);
    drop(serve);
    fs::remove_file(fixture.state_dir.join("coordination/daemon-endpoint.json")).ok();
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    let details = error_of(&output)["details"].clone();
    diagnostics(&details);
    assert_eq!(details["recovery"]["safe_to_retry"], true);
}

#[test]
fn console_operation_lock_timeout_measures_only_lock_wait() {
    let fixture = Fixture::new();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.state_dir.join("coordination/registry.lock"))
        .unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    let output = fixture.start_via_console(&["--agent", "claude", "--format", "json"], true);
    assert_eq!(error_of(&output)["code"], "coordination-lock-timeout");
    let details = error_of(&output)["details"].clone();
    diagnostics(&details);
    assert!(
        details["diagnostics"]["coordination_lock_wait_ms"]
            .as_u64()
            .unwrap()
            >= 1900
    );
    assert!(
        details["diagnostics"]["command_elapsed_ms"]
            .as_u64()
            .unwrap()
            < 6000
    );
}

#[test]
fn console_operation_reconcile_reports_created_id_after_response_loss_and_restart() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child","agent":"claude"}}}),
    );
    let request = json!({"operation_id":OPERATION,"session":{"agent":"claude"}});
    let (status, result) = route(&serve, request.clone());
    assert_eq!(status, 201, "{result}");
    drop(serve); // Caller lost its response; operation evidence survives daemon restart.
    let serve = fixture.serve(&[], Some(&aggregator));
    for body in [json!({"operation_id":OPERATION,"reconcile":true}), request] {
        let (status, result) = route(&serve, body);
        assert_eq!(status, 201, "{result}");
        assert_eq!(result["session"]["id"], "created-child");
        assert_eq!(result["reconciliation"]["state"], "created");
        diagnostics(&result);
    }
    assert_eq!(aggregator.seen().len(), 1, "must never create twice");
}

#[test]
fn console_operation_reconcile_unknown_reports_candidates_without_creating() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply(String::new()); // Created upstream, but response was lost.
    let (status, result) = route(
        &serve,
        json!({"operation_id":OPERATION,"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 502, "{result}");
    let mut child = record(MACHINE, "existing-child", Some("child-launch"), true);
    child["lineage"] = child_lineage();
    let mut board = view();
    board["records"] = json!([child]);
    aggregator.reply_json(200, &board);
    let (status, result) = route(&serve, json!({"operation_id":OPERATION,"reconcile":true}));
    assert_eq!(status, 201, "{result}");
    assert_eq!(result["reconciliation"]["state"], "ambiguous");
    assert_eq!(result["reconciliation"]["safe_to_retry"], false);
    assert_eq!(
        result["reconciliation"]["candidates"][0]["session_id"],
        "existing-child"
    );
    assert!(
        result["reconciliation"]["candidates"][0]
            .get("title")
            .is_none()
    );
    let seen = aggregator.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].path(), "/api/coordination/board/v1");
}

#[test]
fn console_operation_replay_conflicts_and_unknown_reconcile_fail_closed() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    let (status, _) = route(&serve, json!({"operation_id":OPERATION,"reconcile":true}));
    assert_eq!(status, 400);
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child"}}}),
    );
    let (status, _) = route(
        &serve,
        json!({"operation_id":OPERATION,"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 201);
    let (status, result) = route(
        &serve,
        json!({"operation_id":OPERATION,"session":{"agent":"codex"}}),
    );
    assert_eq!(status, 400, "{result}");
    assert_eq!(result["error"]["code"], "console-start-operation-conflict");
    assert_eq!(aggregator.seen().len(), 1);
}

#[test]
fn console_operation_cli_replay_and_reconcile_do_not_repeat_create() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child","agent":"claude"}}}),
    );
    for _ in 0..2 {
        let output = fixture.start_via_console(
            &[
                "--agent",
                "claude",
                "--operation-id",
                OPERATION,
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        assert_eq!(
            output.stdout_json()["data"]["session"]["id"],
            "created-child"
        );
        assert_eq!(output.stdout_json()["data"]["operation_id"], OPERATION);
        diagnostics(&output.stdout_json()["data"]);
    }
    let output = fixture.start_via_console(&["--operation-id", OPERATION, "--reconcile"], true);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert!(
        output
            .stdout_text()
            .contains("existing session: created-child")
    );
    assert!(!output.stdout_text().contains("started"));
    assert_eq!(aggregator.seen().len(), 1);
    // The retained receipt contains metadata, not upstream response content.
    let receipt = fs::read_dir(fixture.state_dir.join("coordination"))
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("console-start-")
                && entry.path().extension().is_some_and(|ext| ext == "json")
        })
        .unwrap();
    let value: Value = serde_json::from_slice(&fs::read(receipt.path()).unwrap()).unwrap();
    assert_eq!(
        value["session"],
        json!({"id":"created-child","agent":"claude"})
    );
    assert!(value.get("prompt").is_none());
}

#[test]
fn console_operation_pre_registration_error_retains_created_id() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(502, &json!({"ok":false,"error":{"code":"title-mode-unconfirmed","message":"created but unconfirmed",
        "details":{"created_session_id":"created-child","prompt":"fixture-sensitive-text"}}}));
    let output = fixture.start_via_console(
        &[
            "--agent",
            "claude",
            "--operation-id",
            OPERATION,
            "--format",
            "json",
        ],
        true,
    );
    assert_eq!(output.code, 1, "{}", output.stdout_text());
    let details = error_of(&output)["details"].clone();
    diagnostics(&details);
    assert_eq!(details["created_session_id"], "created-child");
    assert_eq!(details["safe_to_retry"], false);
    assert!(!output.stdout_text().contains("fixture-sensitive-text"));
    let (status, result) = route(&serve, json!({"operation_id":OPERATION,"reconcile":true}));
    assert_eq!(status, 201, "{result}");
    assert_eq!(result["session"]["id"], "created-child");
    assert_eq!(aggregator.seen().len(), 1);
}

#[test]
fn console_operation_daemon_contention_has_measured_wait_and_no_duplicate() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child"}}}),
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(fixture.state_dir.join("coordination/registry.lock"))
        .unwrap();
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    thread::scope(|scope| {
        let request = scope.spawn(|| {
            route(
                &serve,
                json!({"operation_id":OPERATION,"session":{"agent":"claude"}}),
            )
        });
        thread::sleep(Duration::from_millis(150));
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
        let (status, value) = request.join().unwrap();
        assert_eq!(status, 201, "{value}");
        diagnostics(&value);
        assert!(
            value["diagnostics"]["coordination_lock_wait_ms"]
                .as_u64()
                .unwrap()
                >= 75,
            "{value}"
        );
    });
    assert_eq!(aggregator.seen().len(), 1);
}

#[test]
fn console_operation_unavailable_board_and_pinned_replay_remain_unsafe() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply(String::new());
    let args = [
        "--agent",
        "claude",
        "--operation-id",
        OPERATION,
        "--format",
        "json",
    ];
    let output = fixture.start_via_console(&args, true);
    assert_eq!(
        error_of(&output)["details"]["recovery"]["safe_to_retry"],
        false
    );
    aggregator.reply_json(500, &json!({}));
    let output = fixture.start_via_console(&args, true);
    let value = output.stdout_json();
    assert_eq!(
        value["data"]["reconciliation"]["candidate_scan"],
        "unavailable"
    );
    assert_eq!(value["data"]["reconciliation"]["safe_to_retry"], false);
    assert!(value["data"].get("session").is_none());
    assert_eq!(aggregator.seen().len(), 2);

    let operation = "c7d75b5a-9ad5-4d99-9004-afd6e806d302";
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"unconfirmed-child"}}}),
    );
    for _ in 0..2 {
        let output = fixture.start_via_console(
            &[
                "--agent",
                "claude",
                "--title-mode",
                "pinned",
                "--operation-id",
                operation,
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(error_of(&output)["code"], "title-mode-unconfirmed");
        assert_eq!(
            error_of(&output)["details"]["created_session_id"],
            "unconfirmed-child"
        );
    }
    assert_eq!(aggregator.seen().len(), 3);
}

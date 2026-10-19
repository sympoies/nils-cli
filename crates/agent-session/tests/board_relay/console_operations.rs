use super::*;
use pretty_assertions::assert_eq;
use std::os::fd::AsRawFd;

fn operation_for(counter: u64) -> String {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    uuid::Uuid::from_u128((ms << 80) | (7 << 76) | (2 << 62) | counter as u128).to_string()
}

fn operation() -> &'static str {
    static ID: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| operation_for(1));
    ID.as_str()
}

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
    route_client(&reqwest::blocking::Client::new(), serve, body)
}

fn route_client(client: &reqwest::blocking::Client, serve: &Serve, body: Value) -> (u16, Value) {
    let response = client
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
    let request = json!({"operation_id":operation(),"session":{"agent":"claude"}});
    let (status, result) = route(&serve, request.clone());
    assert_eq!(status, 201, "{result}");
    drop(serve); // Caller lost its response; operation evidence survives daemon restart.
    let serve = fixture.serve(&[], Some(&aggregator));
    for body in [
        json!({"operation_id":operation(),"reconcile":true}),
        request,
    ] {
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
        json!({"operation_id":operation(),"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 502, "{result}");
    let mut child = record(MACHINE, "existing-child", Some("child-launch"), true);
    child["lineage"] = child_lineage();
    // The public board projects identity triples, without audit-only incarnations.
    child["lineage"]["parent"]
        .as_object_mut()
        .unwrap()
        .remove("session_incarnation");
    let mut board = view();
    let mut namesake = child.clone();
    namesake["session_id"] = json!("unrelated-child");
    namesake["lineage"]["parent"]["session_created_at"] = json!("2029-01-01T00:00:00Z");
    board["records"] = json!([namesake, child]);
    aggregator.reply_json(200, &board);
    let (status, result) = route(&serve, json!({"operation_id":operation(),"reconcile":true}));
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
    let (status, _) = route(&serve, json!({"operation_id":operation(),"reconcile":true}));
    assert_eq!(status, 400);
    aggregator.reply_json(
        201,
        &json!({"ok":true,"data":{"session":{"id":"created-child"}}}),
    );
    let (status, _) = route(
        &serve,
        json!({"operation_id":operation(),"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 201);
    let (status, result) = route(
        &serve,
        json!({"operation_id":operation(),"session":{"agent":"codex"}}),
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
                operation(),
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
        assert_eq!(output.stdout_json()["data"]["operation_id"], operation());
        diagnostics(&output.stdout_json()["data"]);
    }
    let output = fixture.start_via_console(&["--operation-id", operation(), "--reconcile"], true);
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
            operation(),
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
    let (status, result) = route(&serve, json!({"operation_id":operation(),"reconcile":true}));
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
    let (status, value) = route(
        &serve,
        json!({"operation_id":operation(),"session":{"agent":"claude"}}),
    );
    assert!(status >= 400, "{value}");
    assert_eq!(value["error"]["code"], "coordination-lock-timeout");
    diagnostics(&value["error"]["details"]);
    assert!(
        value["error"]["details"]["diagnostics"]["coordination_lock_wait_ms"]
            .as_u64()
            .unwrap()
            >= 1900
    );
    assert!(aggregator.seen().is_empty());
    assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
    let (status, value) = route(
        &serve,
        json!({"operation_id":operation(),"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 201, "{value}");
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
        operation(),
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

    let other_operation = operation_for(2);
    let operation = other_operation.as_str();
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

#[test]
fn console_operation_capacity_reclaims_terminal_receipts_without_replaying_evicted_starts() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    let first = operation_for(100);
    let client = reqwest::blocking::Client::new();
    for index in 0..1026 {
        fixture.heartbeat();
        let id = if index == 0 {
            first.clone()
        } else {
            operation_for(100 + index)
        };
        aggregator.reply_json(
            201,
            &json!({"ok":true,"data":{"session":{"id":format!("child-{index}")}}}),
        );
        let (status, result) = route_client(
            &client,
            &serve,
            json!({"operation_id":id,"session":{"agent":"claude"}}),
        );
        assert_eq!(status, 201, "start {index}: {result}");
    }
    let (status, result) = route_client(
        &client,
        &serve,
        json!({"operation_id":first,"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 400, "{result}");
    assert_eq!(result["error"]["code"], "console-start-operation-expired");
    assert_eq!(
        aggregator.seen().len(),
        1026,
        "evicted replay must never create"
    );
}

#[test]
fn console_operation_capacity_classifies_definite_refusals_as_terminal() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    aggregator.reply_json(
        400,
        &json!({"ok":false,"error":{"code":"role-invalid","message":"invalid role"}}),
    );
    let (status, _) = route(
        &serve,
        json!({"operation_id":operation(),"session":{"agent":"claude"}}),
    );
    assert_eq!(status, 400);
    let (status, result) = route(&serve, json!({"operation_id":operation(),"reconcile":true}));
    assert_eq!(status, 201, "{result}");
    assert_eq!(result["reconciliation"]["state"], "not_created");
    assert_eq!(result["reconciliation"]["safe_to_retry"], true);
    assert_eq!(aggregator.seen().len(), 1);
}

#[test]
fn console_operation_capacity_reclaims_old_and_recent_ambiguity_with_replay_fence() {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    for started_ms in [now_ms - 8 * 24 * 60 * 60 * 1000, now_ms - 10_000] {
        let fixture = Fixture::new();
        let aggregator = Aggregator::start();
        let serve = fixture.serve(&[], Some(&aggregator));
        for index in 0..1024 {
            private_file(&fixture.state_dir.join(format!("coordination/console-start-fixture-{index}.json")),
                json!({"schema_version":"agent-session.console-start-operation.v1","state":"ambiguous","operation_started_at_ms":started_ms}).to_string().as_bytes());
        }
        fixture.heartbeat();
        aggregator.reply_json(
            201,
            &json!({"ok":true,"data":{"session":{"id":"fresh-child"}}}),
        );
        let (status, result) = route(
            &serve,
            json!({"operation_id":operation_for(5000),"session":{"agent":"claude"}}),
        );
        assert_eq!(status, 201, "{result}");
        let expired =
            uuid::Uuid::from_u128(((started_ms as u128) << 80) | (7 << 76) | (2 << 62) | 1)
                .to_string();
        let (status, result) = route(
            &serve,
            json!({"operation_id":expired,"session":{"agent":"claude"}}),
        );
        assert_eq!(status, 400, "{result}");
        assert_eq!(result["error"]["code"], "console-start-operation-expired");
        assert_eq!(aggregator.seen().len(), 1);
    }
}

fn reconcile_expired_operation(evicted: bool) {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&[], Some(&aggregator));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let started = if evicted {
        now - 2000
    } else {
        now - 8 * 24 * 60 * 60 * 1000
    };
    if evicted {
        private_file(
            &fixture
                .state_dir
                .join("coordination/console-start-expiry-watermark"),
            (now - 1000).to_string().as_bytes(),
        );
    }
    let operation =
        uuid::Uuid::from_u128(((started as u128) << 80) | (7 << 76) | (2 << 62) | 1).to_string();
    let (status, result) = route(&serve, json!({"operation_id":operation,"reconcile":true}));
    assert_eq!(status, 400, "{result}");
    assert_eq!(result["error"]["code"], "console-start-operation-expired");
    assert!(
        result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("inspect Console")
    );
    assert_eq!(result["error"]["details"]["operation_id"], operation);
    let (status, result) = route(
        &serve,
        json!({"operation_id":operation_for(9000),"reconcile":true}),
    );
    assert_eq!(status, 400, "{result}");
    assert_eq!(result["error"]["code"], "console-start-operation-not-found");
    assert!(
        aggregator.seen().is_empty(),
        "missing receipt checks must not call upstream"
    );
}

#[test]
fn console_operation_reconcile_distinguishes_evicted_from_unused_ids() {
    reconcile_expired_operation(true);
}

#[test]
fn console_operation_reconcile_distinguishes_aged_from_unused_ids() {
    reconcile_expired_operation(false);
}

#[test]
fn console_start_failure_survives_cli_forwarding_and_receipt_reconciliation() {
    for retryable in [true, false] {
        let fixture = Fixture::new();
        let aggregator = Aggregator::start();
        let serve = fixture.serve(&[], Some(&aggregator));
        let operation = operation_for(if retryable { 601 } else { 602 });
        aggregator.reply_json(500, &json!({"ok":false,"error":{
            "code":"start-failed", "message":"provider exited during startup",
            "details":{
                "session_id":"failed-child", "reason":"runtime-exited-during-startup",
                "provider_exit_code":23, "stderr_tail":format!("{}provider configuration unavailable", if retryable { String::new() } else { "中".repeat(2000) }),
                "retryable":retryable, "safe_to_retry":retryable,
                "next_action":if retryable {"retry_create"} else {"inspect_session"},
                "recovery":{"safe_to_retry":retryable},
                "cleanup":if retryable {json!({"state":"completed"})} else {json!({"state":"blocked","reason":"process_boundary_live"})}
            }
        }}));
        let output = fixture.start_via_console(
            &[
                "--agent",
                "codex",
                "--operation-id",
                &operation,
                "--format",
                "json",
            ],
            true,
        );
        let error = error_of(&output);
        assert_eq!(error["code"], "start-failed");
        assert_eq!(error["details"]["provider_exit_code"], 23);
        assert_eq!(error["details"]["safe_to_retry"], retryable);
        assert_eq!(error["details"]["recovery"]["safe_to_retry"], retryable);
        let tail = error["details"]["stderr_tail"].as_str().unwrap();
        assert!(tail.ends_with("provider configuration unavailable"));
        assert!(tail.len() <= 4096);
        if !retryable {
            assert_eq!(error["details"]["cleanup"]["state"], "blocked");
            assert_eq!(
                error["details"]["cleanup"]["reason"],
                "process_boundary_live"
            );
        }
        drop(serve);
        let _serve = fixture.serve(&[], Some(&aggregator));
        let output = fixture.start_via_console(
            &[
                "--agent",
                "codex",
                "--operation-id",
                &operation,
                "--reconcile",
                "--format",
                "json",
            ],
            true,
        );
        assert_eq!(output.code, 0, "{}", output.stdout_text());
        let result = output.stdout_json();
        assert_eq!(result["data"]["reconciliation"]["state"], "start_failed");
        assert_eq!(result["data"]["reconciliation"]["safe_to_retry"], retryable);
        assert_eq!(result["data"]["session"]["id"], "failed-child");
        assert_eq!(result["data"]["failure"]["provider_exit_code"], 23);
        assert!(
            result["data"]["failure"].get("stderr_tail").is_none(),
            "receipts retain metadata only"
        );
        assert_eq!(
            aggregator.seen().len(),
            1,
            "reconciliation must not create or scan after a definitive failure"
        );
    }
}

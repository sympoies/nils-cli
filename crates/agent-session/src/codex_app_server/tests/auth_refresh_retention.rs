use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn external_auth_refresh_retention_allows_65th_refresh_after_closed_connections() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"retention-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, mut record) = recovery_fixture(&tmp);
    bind_thread(&record, "fixture-thread").unwrap();
    let socket_dir = tempfile::Builder::new()
        .prefix("cx-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket_file = socket_dir.path().join("server.sock");
    record
        .runtime
        .as_mut()
        .unwrap()
        .extra
        .insert(SOCKET_KEY.to_string(), json!(display_path(&socket_file)));
    crate::write_session_record(&context, &record).unwrap();
    let path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    let listener = tokio::net::UnixListener::bind(socket_path(&record).unwrap()).unwrap();
    for number in 1..=65 {
        // Advance receipt ages without sleeping; each refresh is outside the
        // retry window, as it would be in a long-lived runtime.
        if path.exists() {
            let _lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
            let mut stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            for operation in stored["operations"].as_array_mut().unwrap() {
                operation["started_at_epoch_ms"] =
                    json!(Timestamp::now().as_millisecond() - 61_000);
            }
            fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        }
        let (handle, commands, ready) = starting_control_channel();
        let current = crate::codex_account::prepare_control_reconnect(
            &context,
            &record.id,
            &record.runtime.as_ref().unwrap().launch_id,
        )
        .unwrap();
        let control = tokio::spawn(run_control(context.clone(), current, commands, ready));
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let login = receive_json(&mut socket).await;
        assert_eq!(login["method"], "account/login/start");
        respond(&mut socket, &login, json!({"type":"chatgptAuthTokens"})).await;
        let loaded = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &loaded,
            json!({"data":["fixture-thread"],"nextCursor":null}),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        respond(&mut socket, &resume, json!({})).await;
        let usage = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &usage,
            json!({"rateLimits":{"primary":{"usedPercent":1}}}),
        )
        .await;
        send_json(&mut socket, json!({"id":"refresh","method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}})).await.unwrap();
        let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
            .await
            .unwrap();
        assert!(
            matches!(&message, Some(Ok(Message::Text(_)))),
            "refresh {number} must receive credentials: {message:?}"
        );
        let response: Value = match message.unwrap().unwrap() {
            Message::Text(text) => serde_json::from_str(&text).unwrap(),
            _ => unreachable!(),
        };
        assert_eq!(response["id"], "refresh");
        assert_eq!(response["result"]["accessToken"], "retention-fixture-token");
        // EOF ends the real control connection, including all error returns.
        drop(socket);
        drop(handle);
        let _ = tokio::time::timeout(Duration::from_secs(3), control)
            .await
            .unwrap()
            .unwrap();
    }
    let current = crate::load_session_record(&context, &record.id).unwrap();
    assert_eq!(
        serde_json::to_value(&current.runtime).unwrap(),
        serde_json::to_value(&record.runtime).unwrap()
    );
    assert_eq!(
        current.provider_resume.as_ref().unwrap().session_id,
        "fixture-thread"
    );
    assert_eq!(
        crate::codex_account::view_for_record(&current)
            .selected_account
            .as_deref(),
        Some("acct1")
    );
    let stored: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(stored["operations"].as_array().unwrap().len() < 64);
}

#[tokio::test]
async fn external_auth_refresh_retention_preserves_uncertain_live_and_unscoped_receipts() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    let calls = tmp.path().join("calls");
    fs::write(&broker, r#"#!/bin/sh
printf 'called\n' >> "$1"
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"retention-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker, calls.clone()]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let request = json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}});
    let mut sink = RecordingMessageSink::default();
    respond_to_external_auth_refresh(
        &mut sink,
        &request,
        Some((&context, &record, "acct1", "original")),
    )
    .await
    .unwrap();
    let path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    let initial: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (state, closed, unscoped, expected) in [
        ("resolving", true, false, "previous-outcome-unknown"),
        ("response_unknown", true, false, "previous-outcome-unknown"),
        ("credentials_supplied", false, false, "capacity-exhausted"),
        ("refresh_failed", false, false, "capacity-exhausted"),
        ("future_state", true, false, "capacity-exhausted"),
        ("credentials_supplied", false, true, "capacity-exhausted"),
    ] {
        let mut stored = initial.clone();
        let operations: Vec<Value> = (0..64)
            .map(|number| {
                let mut receipt = initial["operations"][0].clone();
                receipt["operation_id"] = json!(format!("old-{number}"));
                receipt["state"] = json!(state);
                receipt["started_at_epoch_ms"] = json!(Timestamp::now().as_millisecond() - 61_000);
                receipt["connection_closed"] = json!(closed);
                if unscoped {
                    receipt.as_object_mut().unwrap().remove("connection_id");
                    receipt.as_object_mut().unwrap().remove("connection_closed");
                }
                receipt
            })
            .collect();
        stored["operations"] = json!(operations);
        let before = serde_json::to_vec(&stored).unwrap();
        fs::write(&path, &before).unwrap();
        let error = respond_to_external_auth_refresh(
            &mut sink,
            &request,
            Some((&context, &record, "acct1", "replacement")),
        )
        .await
        .unwrap_err();
        assert!(
            error.contains(expected),
            "{state}, closed={closed}, unscoped={unscoped}: {error}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    assert_eq!(sink.messages.len(), 1);
    assert_eq!(fs::read_to_string(calls).unwrap().lines().count(), 1);
}

#[tokio::test]
async fn external_auth_refresh_retention_prunes_closed_failures_without_resetting_retry_limits() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let broker = tmp.path().join("broker");
    fs::write(&broker, r#"#!/bin/sh
printf '%s\n' '{"schema_version":"agent-session.codex-auth-broker.v1","account":"acct1","access_token":"retention-fixture-token","chatgpt_account_id":"fixture-workspace"}'
"#).unwrap();
    fs::set_permissions(&broker, fs::Permissions::from_mode(0o700)).unwrap();
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        &serde_json::to_string(&vec![broker]).unwrap(),
    );
    let (context, record) = recovery_fixture(&tmp);
    let request = json!({"id":1,"method":"account/chatgptAuthTokens/refresh","params":{"reason":"unauthorized"}});
    let mut sink = RecordingMessageSink::default();
    respond_to_external_auth_refresh(
        &mut sink,
        &request,
        Some((&context, &record, "acct1", "original")),
    )
    .await
    .unwrap();
    let path = crate::session_dir(&context, &record.id).join("codex-auth-recovery.json");
    let initial: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    // Recent closed failures retain both the three-start budget and backoff.
    for (count, age, expected) in [
        (3, 2_000, "retry-budget-exhausted"),
        (1, 0, "retry-backoff"),
    ] {
        let mut stored = initial.clone();
        let operations: Vec<Value> = (0..count)
            .map(|number| {
                let mut receipt = initial["operations"][0].clone();
                receipt["operation_id"] = json!(format!("closed-{number}"));
                receipt["connection_closed"] = json!(true);
                receipt["state"] = json!("refresh_failed");
                receipt["started_at_epoch_ms"] = json!(Timestamp::now().as_millisecond() - age);
                receipt
            })
            .collect();
        stored["operations"] = json!(operations);
        fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let error = respond_to_external_auth_refresh(
            &mut sink,
            &request,
            Some((&context, &record, "acct1", "replacement")),
        )
        .await
        .unwrap_err();
        assert!(error.contains(expected), "{error}");
    }
    let mut stored = initial.clone();
    let operations: Vec<Value> = (0..64)
        .map(|number| {
            let mut receipt = initial["operations"][0].clone();
            receipt["operation_id"] = json!(format!("closed-{number}"));
            receipt["connection_closed"] = json!(true);
            receipt["state"] = json!("refresh_failed");
            receipt["started_at_epoch_ms"] = json!(Timestamp::now().as_millisecond() - 61_000);
            receipt
        })
        .collect();
    stored["operations"] = json!(operations);
    fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    respond_to_external_auth_refresh(
        &mut sink,
        &request,
        Some((&context, &record, "acct1", "replacement")),
    )
    .await
    .unwrap();
    let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(stored["operations"].as_array().unwrap().len(), 1);
    assert_eq!(stored["operations"][0]["state"], "credentials_supplied");
    assert_eq!(sink.messages.len(), 2);
}

#[tokio::test]
async fn external_auth_refresh_retention_never_blocks_the_runtime_on_connection_drop() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = record_with_runtime("retire-contention", &tmp.path().join("server.sock"));
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    let connection = crate::codex_app_server::auth_refresh::Connection::new(&context, &record);
    let (locked, ready) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
        locked.send(()).unwrap();
        // The watchdog keeps a blocking implementation from hanging the test.
        released.recv_timeout(Duration::from_secs(2)).is_ok()
    });
    ready.recv().unwrap();
    drop(connection);
    // A synchronous drop would wait on the lock, preventing this single
    // runtime worker from advancing and making the watchdog release it.
    let sent = release.send(()).is_ok();
    assert!(
        holder.join().unwrap(),
        "connection drop blocked the runtime worker"
    );
    assert!(sent);
}

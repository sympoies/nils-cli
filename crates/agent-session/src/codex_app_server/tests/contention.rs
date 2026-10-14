use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn thread_start_contention_is_retried_before_forwarding_exactly_once() {
    for exhaust in [false, true] {
        let tmp = tempfile::TempDir::new().unwrap();
        let upstream = tmp.path().join("app.sock");
        let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record_with_runtime("start-contention", &upstream);
        fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let args = crate::cli::CodexAppServerProxyArgs {
            id: record.id.clone(),
            upstream: upstream.clone(),
            listen: upstream.with_extension("proxy"),
        };
        let proxy_context = context.clone();
        let proxy = tokio::spawn(async move { run_proxy_session(proxy_context, args).await });
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            if !exhaust {
                let request = receive_json(&mut socket).await;
                assert_eq!(request["id"], 17);
                assert_eq!(request["method"], "thread/start");
                respond(
                    &mut socket,
                    &request,
                    json!({ "thread": { "id": "fixture-thread" } }),
                )
                .await;
            }
            // No duplicate or rejected thread creation may reach the provider.
            let request = receive_json(&mut socket).await;
            assert_eq!(request["method"], "thread/read");
            respond(&mut socket, &request, json!({ "ok": true })).await;
            socket.close(None).await.unwrap();
        });
        let stream = connect_socket(&upstream.with_extension("proxy"))
            .await
            .unwrap();
        let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
            .await
            .unwrap();
        let lock = crate::acquire_session_record_lock(&context, &record.id).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(if exhaust { 6500 } else { 1300 }));
            drop(lock);
        });
        let started = Instant::now();
        tui.send(Message::Text(
            json!({ "id":17, "method":"thread/start", "params":{} })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        let response = tokio::time::timeout(Duration::from_secs(8), receive_json(&mut tui))
            .await
            .unwrap();
        assert_eq!(response["id"], 17);
        assert!(started.elapsed() < Duration::from_secs(8));
        if exhaust {
            assert_eq!(response["error"]["code"], -32001);
            assert_eq!(
                response["error"]["data"]["reason"],
                "manual_cancellation_busy"
            );
            assert_eq!(response["error"]["data"]["retryable"], true);
            assert_eq!(
                response["error"]["data"]["recovery"]["outcome"],
                "not_forwarded"
            );
        } else {
            assert_eq!(response["result"]["thread"]["id"], "fixture-thread");
        }
        release.join().unwrap();
        tui.send(Message::Text(
            json!({ "id":18, "method":"thread/read", "params":{} })
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        assert_eq!(receive_json(&mut tui).await["id"], 18);
        server.await.unwrap();
        proxy.await.unwrap().unwrap();
    }
}

#[test]
fn permanent_tui_refusals_are_not_classified_as_retryable_contention() {
    for rejection in [
        TuiMutationRejection::AccountMutationForbidden,
        TuiMutationRejection::AccountAuthorityUnavailable,
        TuiMutationRejection::RuntimeChanged,
    ] {
        let Message::Text(text) = tui_busy_response(&json!(1), rejection) else {
            panic!("text response");
        };
        let response: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(response["error"]["data"]["retryable"], false);
        assert_eq!(
            response["error"]["data"]["recovery"]["outcome"],
            "not_forwarded"
        );
    }
}

#[test]
fn tui_busy_response_preserves_stable_contract_for_every_rejection() {
    for (rejection, reason) in [
        (
            TuiMutationRejection::TurnAlreadyPending,
            "turn_already_pending",
        ),
        (
            TuiMutationRejection::AccountMutationForbidden,
            "account_mutation_forbidden",
        ),
        (TuiMutationRejection::AccountNotReady, "account_not_ready"),
        (
            TuiMutationRejection::TurnRequestInvalid,
            "turn_request_invalid",
        ),
        (
            TuiMutationRejection::TurnGateOpenFailed,
            "turn_gate_open_failed",
        ),
        (TuiMutationRejection::TurnGateBusy, "turn_gate_busy"),
        (
            TuiMutationRejection::ManualMarkerThreadMismatch,
            "manual_marker_thread_mismatch",
        ),
        (
            TuiMutationRejection::ManualMarkerReplaced,
            "manual_marker_replaced",
        ),
        (
            TuiMutationRejection::ManualMarkerInvalid,
            "manual_marker_invalid",
        ),
        (TuiMutationRejection::ManualAckFailed, "manual_ack_failed"),
        (
            TuiMutationRejection::RuntimeIdentityMissing,
            "runtime_identity_missing",
        ),
        (
            TuiMutationRejection::ManualCancellationBusy,
            "manual_cancellation_busy",
        ),
        (TuiMutationRejection::RuntimeChanged, "runtime_changed"),
        (
            TuiMutationRejection::AccountAuthorityUnavailable,
            "account_authority_unavailable",
        ),
    ] {
        let Message::Text(text) = tui_busy_response(&json!(7), rejection) else {
            panic!("busy response must be text");
        };
        let response: Value = serde_json::from_str(text.as_str()).unwrap();
        assert_eq!(response["id"], 7);
        assert_eq!(response["error"]["code"], -32001);
        assert_eq!(
            response["error"]["message"],
            "agent-session state is busy; retry the request"
        );
        assert_eq!(response["error"]["data"]["reason"], reason);
    }
}

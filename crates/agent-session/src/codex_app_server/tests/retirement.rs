use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[tokio::test]
async fn compact_forwards_normally_during_an_in_flight_turn() {
    exercise_compaction(false).await;
}

#[tokio::test]
async fn compact_waits_for_retirement_and_fences_its_pending_response() {
    exercise_compaction(true).await;
}

async fn exercise_compaction(hold_retirement: bool) {
    let tmp = tempfile::tempdir().unwrap();
    let (_socket_dir, upstream) = nils_test_support::short_socket_path("compact.sock");
    let listener = tokio::net::UnixListener::bind(&upstream).unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record = record_with_runtime("compact-fixture", &upstream);
    let resume_id = "fixture-thread";
    record.provider_resume = Some(ProviderResume {
        provider: "codex".into(),
        session_id: resume_id.into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: "test".into(),
        resume_args: canonical_provider_resume_args(AgentKind::Codex, &record.cwd, resume_id)
            .unwrap(),
        extra: BTreeMap::new(),
    });
    let dir = crate::session_dir(&context, &record.id);
    fs::create_dir_all(&dir).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    crate::activity::activate_runtime(&context, &record).unwrap();

    let lock_path = dir.join(".codex-app-server-compaction.lock");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    if hold_retirement {
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
    }
    let proxy_path = upstream.with_extension("proxy");
    let args = crate::cli::CodexAppServerProxyArgs {
        id: record.id.clone(),
        upstream,
        listen: proxy_path.clone(),
    };
    let proxy = tokio::spawn(run_proxy_session(context, args));
    let stream = connect_socket(&proxy_path).await.unwrap();
    let (mut tui, _) = tokio_tungstenite::client_async("ws://localhost", stream)
        .await
        .unwrap();
    let (server_stream, _) = listener.accept().await.unwrap();
    let mut server = tokio_tungstenite::accept_async(server_stream)
        .await
        .unwrap();
    let turn = json!({
        "id": 4,
        "method": "turn/start",
        "params": {"threadId": resume_id, "input": [{"type": "text", "text": "synthetic"}]}
    });
    if !hold_retirement {
        send_json(&mut tui, turn.clone()).await.unwrap();
        assert_eq!(receive_json(&mut server).await["method"], "turn/start");
    }
    let compact = json!({
        "id": 5,
        "method": "thread/compact/start",
        "params": {"threadId": resume_id}
    });
    send_json(&mut tui, compact.clone()).await.unwrap();

    if hold_retirement {
        assert!(
            tokio::time::timeout(Duration::from_millis(100), server.next())
                .await
                .is_err(),
            "the proxy forwarded compaction across the retirement fence"
        );
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_UN) }, 0);
    }
    assert_eq!(receive_json(&mut server).await, compact);
    if hold_retirement {
        assert_ne!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "the proxy released the compaction fence before the response"
        );
    }
    respond(&mut server, &compact, json!({})).await;
    assert_eq!(receive_json(&mut tui).await["result"], json!({}));
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    if !hold_retirement {
        respond(&mut server, &turn, json!({"turn":{"id":"fixture-turn"}})).await;
        assert_eq!(
            receive_json(&mut tui).await["result"]["turn"]["id"],
            "fixture-turn"
        );
    }
    tui.close(None).await.unwrap();
    let _ = proxy.await.unwrap();
}

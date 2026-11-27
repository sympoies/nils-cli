//! Own the fixture root until the test runtime has drained detached writes.

use super::*;
use pretty_assertions::assert_eq;

pub(crate) fn run(test: impl AsyncFnOnce(&Path)) {
    let root = nils_test_support::tempdir::ScopedTempDir::new();
    run_at(root.path(), test);
}

/// Keep the existing short fixture paths for tests that bind sockets in the
/// root. Explicit close checks cleanup even outside the probe's TMPDIR.
pub(crate) fn run_short(test: impl AsyncFnOnce(&Path)) {
    let (root, _) = nils_test_support::short_socket_path("socket");
    run_at(root.path(), test);
    root.close().expect("remove short control fixture");
}

fn run_at(root: &Path, test: impl AsyncFnOnce(&Path)) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(test(root));
    // Connection drop can schedule blocking retirement after control exits.
    // Runtime drop joins that work while the fixture root is still live.
    drop(runtime);
}

pub(super) async fn assert_reconnect_resumes_bound_thread(root: &Path) {
    let (_socket_dir, socket) = nils_test_support::short_socket_path("reconnect.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let context = CliContext {
        state_dir: root.join("state"),
        host: None,
    };
    let record = record_with_runtime("reconnect", &socket);
    fs::create_dir_all(crate::session_dir(&context, &record.id)).unwrap();
    crate::write_session_record(&context, &record).unwrap();
    bind_thread(&record, "raw-thread-a").unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        let initialize = receive_json(&mut socket).await;
        respond(&mut socket, &initialize, json!({})).await;
        assert_eq!(receive_json(&mut socket).await["method"], "initialized");
        let loaded = receive_json(&mut socket).await;
        respond(
            &mut socket,
            &loaded,
            json!({
                "data": ["raw-thread-decoy", "raw-thread-a"],
                "nextCursor": null
            }),
        )
        .await;
        let resume = receive_json(&mut socket).await;
        assert_eq!(resume["method"], "thread/resume");
        assert_eq!(resume["params"]["threadId"], "raw-thread-a");
        assert_eq!(resume["params"]["excludeTurns"], true);
        respond(&mut socket, &resume, json!({})).await;
        for _ in 0..2 {
            let usage = receive_json(&mut socket).await;
            assert_eq!(usage["method"], "account/rateLimits/read");
            respond(
                &mut socket,
                &usage,
                json!({
                    "rateLimits": {
                        "primary": { "usedPercent": 100, "resetsAt": 1_900_000_000 }
                    }
                }),
            )
            .await;
        }
    });
    let (handle, commands, ready) = starting_control_channel();
    let control = tokio::spawn(run_control(
        context.clone(),
        record.clone(),
        commands,
        ready,
    ));

    let usage = handle.usage().await.unwrap();
    assert!(usage.authoritative);
    assert!(usage.has_exhausted_windows);
    server.await.unwrap();
    let persisted = crate::load_session_record(&context, &record.id).unwrap();
    let resume = persisted
        .provider_resume
        .as_ref()
        .expect("control reconnect must persist the loaded thread identity");
    assert_eq!(resume.session_id, "raw-thread-a");
    assert_eq!(resume.capture_method, PROVIDER_RESUME_CAPTURE_METHOD);
    assert!(
        crate::session_dir(&context, &record.id)
            .join(crate::SESSION_RESUME_FILE)
            .is_file()
    );
    drop(handle);
    control.abort();
    let _ = control.await;
}

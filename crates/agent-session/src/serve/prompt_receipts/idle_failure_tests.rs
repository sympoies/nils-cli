use super::*;
use codex_app_server::guarded::IdleReason;
use pretty_assertions::{assert_eq, assert_ne};

#[tokio::test]
async fn native_idle_reason_survives_durable_refusal_and_lookup() {
    let temp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    fs::create_dir_all(session_dir(&context, "fixture-session")).unwrap();
    let receipt = Receipt::unknown(
        "fixture-session".into(),
        "runtime-one".into(),
        "attempt-one".into(),
    );
    let store::ClaimResult::New(claim) = store::claim(
        &context,
        receipt.clone(),
        &store::fingerprint("synthetic notice", true),
    )
    .unwrap() else {
        panic!("new claim")
    };
    let error = IdleReason::ThreadReadRpcFailed.annotate(CliError::runtime(
        "send-idle-unobservable",
        "native idle unavailable",
        Some(json!({"stage":"admission", "pasted":false, "submitted":false})),
    ));
    let result = classify(terminal::failure(error), receipt.clone()).await;
    assert_eq!(result.receipt.outcome, Outcome::Refused);
    store::finish(claim, &result).unwrap();
    let loaded = store::lookup(&context, &receipt).unwrap().unwrap();
    let response = loaded.response("fixture-machine");
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value["error"]["details"]["receipt"]["native_idle_reason"],
        "thread_read_rpc_failed"
    );
    assert_eq!(
        value["error"]["hint"],
        IdleReason::ThreadReadRpcFailed.hint()
    );
    assert_eq!(
        loaded.receipt.code.as_deref(),
        Some("send-idle-unobservable")
    );
}

#[tokio::test]
async fn native_idle_refusal_record_drops_unrecognized_reason_content() {
    let receipt = Receipt::unknown(
        "fixture-session".into(),
        "runtime-one".into(),
        "attempt-one".into(),
    );
    let response = terminal::failure(CliError::runtime(
        "send-idle-unobservable",
        "idle unavailable",
        Some(
            json!({"stage":"admission", "pasted":false, "submitted":false,
            "native_idle_reason":"private-provider-status"}),
        ),
    ));
    let result = classify(response, receipt).await;
    let bytes = serde_json::to_string(&result).unwrap();
    assert!(!bytes.contains("private-provider-status"));
    assert!(result.receipt.native_idle_reason.is_none());
}

#[test]
fn independent_keyed_attempts_share_one_runtime_boundary() {
    use idle_boundary::Store;
    let temp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    let id = "boundary-fixture";
    let incarnation = "fixture-launch";
    fs::create_dir_all(session_dir(&context, id)).unwrap();
    let observation = idle_observation("binding-one", 0, "initial-turn");
    let boundary = {
        let lock = crate::acquire_session_record_lock(&context, id).unwrap();
        Store::new(&context, id, incarnation, &lock)
            .observe(observation.clone())
            .unwrap()
            .unwrap()
    };
    let start = std::sync::Barrier::new(2);
    let mut admitted = std::thread::scope(|scope| {
        ["attempt-one", "attempt-two"]
            .map(|request_id| {
                let context = &context;
                let start = &start;
                let observation = &observation;
                let token = &boundary.token;
                scope.spawn(move || {
                    start.wait();
                    let receipt =
                        Receipt::unknown(id.into(), incarnation.into(), request_id.into());
                    let fingerprint = store::fingerprint("fixture prompt", true);
                    let store::ClaimResult::New(_claim) =
                        store::claim(context, receipt, &fingerprint).unwrap()
                    else {
                        panic!("each request has its own key");
                    };
                    let lock = crate::acquire_session_record_lock(context, id).unwrap();
                    let store = Store::new(context, id, incarnation, &lock);
                    store
                        .consume(token, observation)
                        .map_err(|error| error.code().to_owned())
                })
            })
            .map(|thread| thread.join().unwrap())
    });
    admitted.sort();
    assert_eq!(admitted, [Ok(()), Err("stale-idle-ready-boundary".into())]);
    assert_eq!(
        idle_boundary::projected(&context, id, incarnation, &observation.binding).unwrap(),
        None
    );
}

fn idle_observation(binding: &str, position: u64, turn: &str) -> idle_boundary::IdleObservation {
    idle_boundary::IdleObservation {
        binding: store::digest(&binding),
        position,
        turn: store::digest(&turn),
    }
}

fn boundary_context(temp: &tempfile::TempDir) -> CliContext {
    let context = CliContext {
        state_dir: temp.path().to_path_buf(),
        host: None,
    };
    fs::create_dir_all(session_dir(&context, "boundary-fixture")).unwrap();
    context
}

#[test]
fn idle_ready_boundary_requires_turn_progress_after_consumption() {
    use idle_boundary::{Store, projected};

    let temp = tempfile::TempDir::new().unwrap();
    let context = boundary_context(&temp);
    let id = "boundary-fixture";
    let incarnation = "runtime-one";
    let initial = idle_observation("control-one", 1, "turn-one");
    let first = {
        let lock = crate::acquire_session_record_lock(&context, id).unwrap();
        let store = Store::new(&context, id, incarnation, &lock);
        let first = store.observe(initial.clone()).unwrap().unwrap();
        assert_eq!(store.observe(initial.clone()).unwrap(), Some(first.clone()));
        assert_eq!(
            projected(&context, id, incarnation, &initial.binding).unwrap(),
            Some(first.clone())
        );
        store.consume(&first.token, &initial).unwrap();
        first
    };

    // A new context and lock see the same durable, consumed boundary.
    let restarted = boundary_context(&temp);
    let lock = crate::acquire_session_record_lock(&restarted, id).unwrap();
    let store = Store::new(&restarted, id, incarnation, &lock);
    assert_eq!(
        projected(&restarted, id, incarnation, &initial.binding).unwrap(),
        None
    );
    assert_eq!(
        store.check(&first.token).unwrap_err().code(),
        "stale-idle-ready-boundary"
    );
    store.invalidate().unwrap();
    assert_eq!(store.observe(initial.clone()).unwrap(), None);
    let rebound = idle_observation("control-two", 1, "turn-one");
    assert_eq!(store.observe(rebound).unwrap(), None);
    let regressed = idle_observation("control-one", 0, "previous-turn");
    assert_eq!(store.observe(regressed).unwrap(), None);

    let completed = idle_observation("control-two", 2, "turn-two");
    let second = store.observe(completed.clone()).unwrap().unwrap();
    assert_eq!(first.generation, "1");
    assert_eq!(second.generation, "2");
    assert_ne!(first.token, second.token);
    assert_eq!(
        store.check(&first.token).unwrap_err().code(),
        "stale-idle-ready-boundary"
    );
    store.consume(&second.token, &completed).unwrap();
}

#[test]
fn idle_ready_boundary_rejects_a_turn_that_advanced_before_submit() {
    use idle_boundary::{Store, projected};

    let temp = tempfile::TempDir::new().unwrap();
    let context = boundary_context(&temp);
    let id = "boundary-fixture";
    let incarnation = "runtime-one";
    let initial = idle_observation("control-one", 1, "turn-one");
    let completed = idle_observation("control-one", 2, "turn-two");
    let lock = crate::acquire_session_record_lock(&context, id).unwrap();
    let store = Store::new(&context, id, incarnation, &lock);
    let first = store.observe(initial.clone()).unwrap().unwrap();

    let stale = store.consume(&first.token, &completed).unwrap_err();
    assert_eq!(stale.code(), "stale-idle-ready-boundary");
    assert_eq!(
        stale.details(),
        Some(&json!({"stage":"admission","pasted":false,"submitted":false,"dispatched":false}))
    );
    assert_eq!(
        projected(&context, id, incarnation, &initial.binding).unwrap(),
        None
    );
    let second = store.observe(completed.clone()).unwrap().unwrap();
    assert_eq!(second.generation, "2");
    assert_ne!(first.token, second.token);
    store.consume(&second.token, &completed).unwrap();
}

#[test]
fn idle_ready_boundary_invalidates_on_binding_or_incarnation_change() {
    use idle_boundary::{Store, projected};

    let temp = tempfile::TempDir::new().unwrap();
    let context = boundary_context(&temp);
    let id = "boundary-fixture";
    let initial = idle_observation("control-one", 1, "turn-one");
    let rebound = idle_observation("control-two", 1, "turn-one");
    let lock = crate::acquire_session_record_lock(&context, id).unwrap();
    let store = Store::new(&context, id, "runtime-one", &lock);
    let first = store.observe(initial.clone()).unwrap().unwrap();

    assert_eq!(
        projected(&context, id, "runtime-one", &rebound.binding).unwrap(),
        None
    );
    assert_eq!(
        store.consume(&first.token, &rebound).unwrap_err().code(),
        "stale-idle-ready-boundary"
    );
    let second = store.observe(rebound.clone()).unwrap().unwrap();
    assert_eq!(second.generation, "2");
    assert_ne!(first.token, second.token);
    store.invalidate().unwrap();
    assert_eq!(
        store.check(&second.token).unwrap_err().code(),
        "stale-idle-ready-boundary"
    );
    let renewed = store.observe(rebound.clone()).unwrap().unwrap();
    assert_eq!(renewed.generation, "3");

    let replaced = Store::new(&context, id, "runtime-two", &lock);
    assert_eq!(
        projected(&context, id, "runtime-two", &rebound.binding).unwrap(),
        None
    );
    assert_eq!(
        replaced
            .consume(&renewed.token, &rebound)
            .unwrap_err()
            .code(),
        "stale-idle-ready-boundary"
    );
    let next = replaced.observe(rebound.clone()).unwrap().unwrap();
    assert_eq!(next.generation, "1");
    assert_ne!(renewed.token, next.token);
    assert_eq!(
        projected(&context, id, "runtime-one", &rebound.binding).unwrap(),
        None
    );
    replaced.consume(&next.token, &rebound).unwrap();
}

#[test]
fn idle_ready_boundary_refuses_to_replace_an_unreadable_record() {
    use idle_boundary::{Store, projected};

    let temp = tempfile::TempDir::new().unwrap();
    let context = boundary_context(&temp);
    let id = "boundary-fixture";
    let incarnation = "runtime-one";
    let observation = idle_observation("control-one", 0, "initial-turn");
    let path = session_dir(&context, id).join("idle-ready-boundary.json");
    fs::write(&path, b"null").unwrap();
    let lock = crate::acquire_session_record_lock(&context, id).unwrap();
    let store = Store::new(&context, id, incarnation, &lock);

    assert_eq!(
        store.observe(observation.clone()).unwrap_err().code(),
        "idle-ready-boundary-unavailable"
    );
    assert_eq!(
        store
            .consume("attempted-token", &observation)
            .unwrap_err()
            .code(),
        "idle-ready-boundary-unavailable"
    );
    assert!(projected(&context, id, incarnation, &observation.binding).is_err());
    assert_eq!(fs::read(path).unwrap(), b"null");
}

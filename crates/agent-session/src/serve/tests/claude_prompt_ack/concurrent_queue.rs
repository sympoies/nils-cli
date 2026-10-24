// Only the first of two concurrent identical requests reaches the queue.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn concurrent_identical_queue_requests_do_not_share_a_baseline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let id = "claude-concurrent-queue";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let pane = QUEUED_CLAUDE_PANE;
    fs::write(tmp.path().join("queued"), pane).unwrap();
    fs::write(tmp.path().join("empty"), DRAINED_CLAUDE_PANE).unwrap();
    // Unlocked observations rendezvous before either request can deliver. A
    // locked observation belongs to its serialized paste and must proceed so
    // the lock can later pass to the other request. This makes the stale-read
    // interleaving deterministic without sleeping to schedule either request.
    let capture = tmp.path().join("capture.py");
    fs::write(&capture, r#"import fcntl, os, pathlib, sys, time
root = pathlib.Path(sys.argv[1])
lock_path = pathlib.Path(sys.argv[2])
styled = '-e' in sys.argv[3:]
if not styled and not (root / 'entered').exists():
    with lock_path.open('a') as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            locked = True
        else:
            locked = False
            fcntl.flock(lock, fcntl.LOCK_UN)
    if not locked:
        (root / ('baseline-' + str(os.getpid()))).touch()
        deadline = time.monotonic() + 10
        while len(list(root.glob('baseline-*'))) < 2:
            if time.monotonic() >= deadline:
                raise RuntimeError('baseline rendezvous timed out')
            time.sleep(0.01)
        # Both unlocked baselines see the original empty queue.
        sys.stdout.write((root / 'empty').read_text())
        sys.exit(0)
pane = (root / ('queued' if (root / 'entered').exists() else 'empty')).read_text()
if styled:
    pane = pane.replace('Press up to edit queued messages', '\x1b[2mPress up to edit queued messages\x1b[22m')
sys.stdout.write(pane)
"#).unwrap();
    let tmux = minimal_tmux(tmp.path());
    let lock_path = tmp
        .path()
        .join(crate::SESSION_LOCKS_DIR)
        .join(format!("{id}.lock"));
    let script = fs::read_to_string(&tmux).unwrap().replace(
        "  capture-pane) printf 'pane\\n'; exit 0 ;;",
        &format!("  send-keys) echo enter >> '{attempts}'; touch '{entered}'; exit 0 ;;\n  capture-pane) exec python3 '{capture}' '{root}' '{lock}' \"$@\" ;;",
            attempts=tmp.path().join("attempts").display(), entered=tmp.path().join("entered").display(),
            capture=capture.display(), root=tmp.path().display(), lock=lock_path.display()),
    );
    fs::write(&tmux, script).unwrap();
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let record = load_session_record(&st.context, id).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    // Create the real lock file used by both submissions and the fixture probe.
    drop(crate::acquire_session_record_lock(&st.context, id).unwrap());
    let request = |key| {
        post_json(
            &format!("/sessions/{id}/prompt/v2"),
            Some(TOKEN),
            json!({"text":"Second queued message.", "expected_session_incarnation":launch, "request_id":key}),
        )
    };
    let (a, b) = tokio::join!(
        call(router(st.clone()), request("concurrent-a")),
        call(router(st.clone()), request("concurrent-b")),
    );
    let mut statuses = [a.0.as_u16(), b.0.as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 500], "a={:?}, b={:?}", a.1, b.1);
    let (submitted, unknown) = if a.0 == StatusCode::OK {
        (a.1, b.1)
    } else {
        (b.1, a.1)
    };
    assert_eq!(submitted["data"]["queued"], true);
    assert_eq!(unknown["error"]["details"]["receipt"]["outcome"], "unknown");
    assert_eq!(
        fs::read_to_string(tmp.path().join("attempts"))
            .unwrap()
            .lines()
            .count(),
        2
    );
}

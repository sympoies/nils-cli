use super::*;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

fn hold_registry(fixture: &Fixture) -> fs::File {
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(fixture.state_dir.join("coordination/registry.lock"))
        .expect("open fixture registry lock");
    // SAFETY: the lock descriptor is valid and held for the returned File's lifetime.
    assert_eq!(
        unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "hold fixture registry lock"
    );
    lock
}

#[test]
fn managed_board_relay_uses_the_committed_registry_under_contention() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let _serve = fixture.serve(&["--board"], Some(&aggregator));
    let _lock = hold_registry(&fixture);
    aggregator.reply_json(200, &view());

    let output = fixture.board(&["--state", "live", "--format", "json"]);
    assert_eq!(output.code, 0, "{}", output.stdout_text());
    assert_eq!(output.stdout_json()["data"]["mode"], "relay");
    assert_eq!(aggregator.seen().len(), 1);
}

#[test]
fn daemon_board_relay_uses_the_committed_registry_under_contention() {
    let fixture = Fixture::new();
    let aggregator = Aggregator::start();
    let serve = fixture.serve(&["--board"], Some(&aggregator));
    let _lock = hold_registry(&fixture);
    aggregator.reply_json(200, &view());

    let (status, body) = serve.get(&format!("/sessions/{SESSION}/board/v1"), Some(CAPABILITY));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body, view());
    assert_eq!(aggregator.seen().len(), 1);
}

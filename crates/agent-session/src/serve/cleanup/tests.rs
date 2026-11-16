use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;

fn expired_orphan(home: &Path, state: &Path, id: &str) -> PathBuf {
    let dir = home.join("tmp").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(".agent-session-tmp"),
        json!({
            "schema_version": "agent-session.session-tmp.v1",
            "session_id": id, "state_dir": state, "stopped_at_epoch": 1,
        })
        .to_string(),
    )
    .unwrap();
    fs::File::open(&dir)
        .unwrap()
        .set_times(
            fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
        )
        .unwrap();
    dir
}

async fn wait_removed(path: &Path) {
    let start = Instant::now();
    while path.exists() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "maintenance did not run"
        );
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn maintenance_repeats_without_launches_and_preserves_foreign_roots() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::TempDir::new().unwrap();
    let home = temp.path().join("home");
    let state = temp.path().join("state");
    let _home = EnvGuard::set(&lock, "AGENT_HOME", home.to_str().unwrap());
    let _grace = EnvGuard::set(&lock, "AGENT_SESSION_TMP_GRACE_SECS", "3600");
    let first = expired_orphan(&home, &state, "first");
    let foreign = expired_orphan(&home, &temp.path().join("vanished-foreign"), "foreign");
    let context = CliContext {
        state_dir: state.clone(),
        host: None,
    };
    let task = tokio::spawn(tmp_maintenance(
        context,
        PathBuf::from("/nonexistent/fixture-tmux"),
    ));
    wait_removed(&first).await;
    let second = expired_orphan(&home, &state, "second");
    // Advance Tokio's clock only; filesystem retention uses explicit old mtimes.
    tokio::time::advance(TMP_SWEEP_INTERVAL).await;
    wait_removed(&second).await;
    assert_eq!(fs::read_dir(home.join("tmp")).unwrap().count(), 1);
    assert!(foreign.is_dir());
    task.abort();
    let _ = task.await;
}

#[tokio::test]
async fn delete_tombstone_cleanup_starts_after_bind_and_is_bounded() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    let root = tmp.path().join(crate::SESSION_DELETE_TOMBSTONES_DIR);
    std::fs::create_dir(&root).unwrap();
    for index in 0..65 {
        let tombstone = root.join(format!("session-{index}"));
        std::fs::create_dir(&tombstone).unwrap();
        std::fs::write(tombstone.join("session.json"), b"retired").unwrap();
    }

    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    assert!(
        bind_listener_and_start_delete_tombstone_cleanup(
            &context,
            address,
            Path::new("/nonexistent/fixture-tmux")
        )
        .await
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 65);

    drop(occupied);
    let listener = bind_listener_and_start_delete_tombstone_cleanup(
        &context,
        address,
        Path::new("/nonexistent/fixture-tmux"),
    )
    .await
    .unwrap();
    let connected = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    drop(connected);

    let started_at = Instant::now();
    loop {
        let remaining = std::fs::read_dir(&root).unwrap().count();
        if remaining == 1 {
            break;
        }
        assert!(
            started_at.elapsed() < Duration::from_secs(2),
            "post-bind cleanup did not finish its bounded sweep: remaining={remaining}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn delete_tombstone_cleanup_failure_does_not_block_listener_availability() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    std::fs::write(
        tmp.path().join(crate::SESSION_DELETE_TOMBSTONES_DIR),
        b"invalid cleanup root",
    )
    .unwrap();

    let listener = bind_listener_and_start_delete_tombstone_cleanup(
        &context,
        "127.0.0.1:0".parse().unwrap(),
        Path::new("/nonexistent/fixture-tmux"),
    )
    .await
    .unwrap();
    let connected = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    drop(connected);
}

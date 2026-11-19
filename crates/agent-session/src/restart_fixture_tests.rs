use super::*;
use pretty_assertions::assert_eq;
use std::time::Instant;

fn pane_is_running(pid: &str) -> bool {
    let output = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .any(|state| !state.starts_with('Z'))
}

#[test]
fn fake_tmux_drop_stops_its_pane_even_while_the_root_still_exists() {
    let root = tempfile::tempdir().unwrap();
    let tmux = fake_tmux(root.path());
    let output = std::process::Command::new(&*tmux)
        .arg("new-session")
        .output()
        .unwrap();
    assert!(output.status.success());
    let pid = fs::read_to_string(root.path().join("pane")).unwrap();
    let pid = pid.trim();
    assert!(pane_is_running(pid));
    drop(tmux);
    let deadline = Instant::now() + Duration::from_secs(2);
    while pane_is_running(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let stopped = !pane_is_running(pid);
    // Also clean up when exercising the unfixed fixture.
    let _ = std::process::Command::new("kill")
        .args(["--", &format!("-{pid}")])
        .output();
    assert!(stopped, "fixture drop left its pane running");
    assert!(root.path().exists());
}

#[test]
fn fake_tmux_pane_stops_when_its_owner_exits_without_teardown() {
    const CHILD_ROOT: &str = "AGENT_SESSION_RESTART_FIXTURE_CHILD_ROOT";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let tmux = fake_tmux(Path::new(&root));
        let output = std::process::Command::new(&*tmux)
            .arg("new-session")
            .output()
            .unwrap();
        assert!(output.status.success());
        // Model an owner that exits without running fixture destructors.
        std::mem::forget(tmux); // tempdir-leak-audit: allow — Parent owns tempdir; skip only pane guard.
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "restart::tests::fixture_tests::fake_tmux_pane_stops_when_its_owner_exits_without_teardown"])
        .env(CHILD_ROOT, root.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let pid = fs::read_to_string(root.path().join("pane")).unwrap();
    let pid = pid.trim();
    let deadline = Instant::now() + Duration::from_secs(2);
    while pane_is_running(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let stopped = !pane_is_running(pid);
    let _ = std::process::Command::new("kill")
        .args(["--", &format!("-{pid}")])
        .output();
    assert!(stopped, "pane outlived its owner process");
}

#[test]
fn restart_fixture_drop_reclaims_its_short_root() {
    let fixture = tempfile::tempdir().unwrap();
    let context = context(fixture.path());
    seed_exited_session(&context, fixture.path());
    let tmux = fake_tmux(fixture.path());
    running_session(&context, &tmux);
    let marker = fixture
        .path()
        .join("state/agent-home/tmp")
        .join(ID)
        .join(".agent-session-short");
    let token = fs::read_to_string(marker).unwrap();
    let root = Path::new("/tmp").join(format!("ark-{token}"));
    assert!(root.is_dir());
    drop(tmux);
    assert_eq!(
        fs::symlink_metadata(root).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn fixture_cleanup_leaves_a_short_root_pointing_outside_its_home() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct RootGuard(PathBuf);
    impl Drop for RootGuard {
        fn drop(&mut self) {
            let _ = fs::remove_file(self.0.join("t"));
            let _ = fs::remove_dir(&self.0);
        }
    }
    let fixture = tempfile::tempdir().unwrap();
    let home = fixture.path().join("home");
    let backing = home.join("tmp/session");
    let outside = fixture.path().join("outside");
    fs::create_dir_all(&backing).unwrap();
    fs::create_dir(&outside).unwrap();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let marker = backing.join(".agent-session-short");
    fs::write(&marker, &token).unwrap();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
    let root = Path::new("/tmp").join(format!("ark-{token}"));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let _root_guard = RootGuard(root.clone());
    symlink(&outside, root.join("t")).unwrap();
    fixture::short_roots::remove_owned_short_roots(fixture.path(), &home).unwrap();
    assert!(root.join("t").is_symlink());
    assert!(outside.is_dir());
    fs::rename(&home, fixture.path().join("home-original")).unwrap();
    symlink(&outside, &home).unwrap();
    assert!(fixture::short_roots::remove_owned_short_roots(fixture.path(), &home).is_err());
    assert!(root.join("t").is_symlink());
}

use super::*;

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

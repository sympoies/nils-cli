use super::*;

#[cfg(target_os = "linux")]
#[test]
fn linux_process_session_pin_rejects_an_uncaptured_member() {
    let tmp = tempfile::TempDir::new().unwrap();
    let descendant_pid_path = tmp.path().join("uncaptured-descendant.pid");
    let pane = ReapedTestProcessGroup::spawn_tree(&descendant_pid_path);
    let descendant_pid: libc::pid_t = fs::read_to_string(descendant_pid_path)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let leader = crate::read_linux_process_identity(pane.pid() as libc::pid_t)
        .unwrap()
        .expect("live session leader");
    let identity = TmuxRuntimeIdentity {
        macos_boot_id: None,
        launch_id: Some("launch-uncaptured-member".to_string()),
        session_id: "$91".to_string(),
        pane_id: "%91".to_string(),
        pane_pid: pane.pid() as libc::pid_t,
        pane_start_time: None,
        process_group_id: Some(pane.process_group_id),
        process_session_id: Some(leader.session_id),
        process_session_members: vec![TmuxProcessIdentity {
            pid: leader.pid,
            start_time: leader.start_time,
        }],
        control_group_members: Vec::new(),
        control_group: None,
        cgroup_mount: None,
        pid_namespace: None,
    };

    assert_eq!(
        crate::verify_captured_linux_process_session(
            leader.session_id,
            &identity.process_session_members,
        )
        .unwrap_err(),
        crate::SessionTerminationFailure::VerificationFailed
    );
    assert_eq!(unsafe { libc::kill(leader.pid, 0) }, 0);
    assert_eq!(unsafe { libc::kill(descendant_pid, 0) }, 0);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "subprocess fixture for the bounded provider helper"]
fn macos_bounded_provider_helper_fixture() {
    let Some(pid_file) = std::env::var_os("AGENT_SESSION_TEST_PROVIDER_HELPER_PID") else {
        return;
    };
    // Inherit the provider's process group without retaining its terminal.
    // Killing the provider alone leaves this bounded helper reparented.
    let mut helper = Command::new("/usr/bin/caffeinate")
        .args(["-i", "-t", "300"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn bounded helper");
    let pid_file = std::path::PathBuf::from(pid_file);
    let ready_file = pid_file.with_extension("ready");
    fs::write(&ready_file, helper.id().to_string()).expect("record helper PID");
    fs::rename(ready_file, pid_file).expect("publish helper PID");
    helper.wait().expect("reap helper if provider survives");
}

#[cfg(target_os = "macos")]
#[test]
fn macos_account_switch_teardown_stops_the_bounded_provider_helper() {
    let tmp = tempfile::TempDir::new().unwrap();
    let context = test_context(tmp.path());
    let id = create_test_record_id(&context, AgentKind::Claude, None, Some("helper-switch"));
    let mut record = load_session_record(&context, &id).unwrap();
    let helper_pid_file = tmp.path().join("helper.pid");
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::account_switch::macos_bounded_provider_helper_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("AGENT_SESSION_TEST_PROVIDER_HELPER_PID", &helper_pid_file)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn provider fixture");
    // Drop kills this test-owned group and reaps its leader even on RED.
    let pane = ReapedTestProcessGroup::from_child(child);
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !helper_pid_file.is_file() && Instant::now() < ready_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let helper_pid: libc::pid_t = fs::read_to_string(&helper_pid_file)
        .expect("provider publishes helper PID")
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::getpgid(helper_pid) }, pane.process_group_id);
    let tmux = tmp.path().join("tmux");
    let killed = shell_words::quote(&tmp.path().join("killed").to_string_lossy());
    fs::write(
        &tmux,
        format!(
            "#!/bin/sh\n{identity}if [ \"$1\" = has-session ]; then\n  if [ -f {killed} ]; then printf '%s\\n' \"can't find session: fixture\" >&2; exit 1; fi\n  exit 0\nfi\nif [ \"$1\" = if-shell ]; then\n  kill -KILL {pane_pid}\n  : > {killed}\n  exit 0\nfi\nexit 42\n",
            identity = deletion_identity_script(&context, &record, pane.pid()),
            pane_pid = pane.pid(),
        ),
    )
    .unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();

    let started = Instant::now();
    crate::terminate_tmux_session_with_timeouts(
        &context,
        &mut record,
        &tmux,
        None,
        Duration::from_secs(1),
        Duration::from_secs(1),
        true,
    )
    .expect("account-switch teardown must stop the provider's surviving bounded helper");
    assert!(started.elapsed() < Duration::from_secs(60));
    assert_eq!(
        crate::process_group_status(pane.process_group_id),
        crate::ProcessGroupStatus::Stopped,
        "the helper must be gone before account-switch resume",
    );
}

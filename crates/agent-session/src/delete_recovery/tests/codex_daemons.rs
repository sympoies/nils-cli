use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use std::process::{Child, Command, Stdio};

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn provider_owned_codex_daemon_pair_survives_successful_session_delete() {
    let lock = GlobalStateLock::new();
    let (tmp, context, record) = session_fixture();
    let owner_home = tmp.path().join("owner-provider-home");
    fs::create_dir(&owner_home).unwrap();
    let _owner_home = EnvGuard::set(&lock, "CODEX_HOME", owner_home.to_str().unwrap());
    let home = tmp.path().join("provider-home");
    let bin = home.join("packages/app-server-daemon/releases/fixture/bin/codex");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let source = tmp.path().join("daemon.c");
    fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { for (;;) pause(); }\n",
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&bin)
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink(
        "releases/fixture",
        home.join("packages/app-server-daemon/current"),
    )
    .unwrap();
    let pid_dir = home.join("app-server-daemon");
    fs::create_dir_all(&pid_dir).unwrap();
    let runtime = &record.runtime.as_ref().unwrap().launch_id;
    let mut daemons = Vec::new();
    for (args, file) in [
        (
            vec!["app-server", "--listen", "unix://", "--managed-daemon"],
            "daemon.pid",
        ),
        (
            vec!["app-server", "daemon", "pid-update-loop"],
            "daemon-updater.pid",
        ),
    ] {
        let mut command = Command::new(&bin);
        command
            .args(args)
            .env("CODEX_HOME", &home)
            .env("AGENT_SESSION_ID", &record.id)
            .env("AGENT_SESSION_RUNTIME_ID", runtime)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let child = Daemon(command.spawn().unwrap());
        let identity = read_linux_process_identity(child.0.id() as i32)
            .unwrap()
            .unwrap();
        nils_common::fs::write_atomic(&pid_dir.join(file), &serde_json::to_vec(&json!({
            "pid": child.0.id(),
            "processIdentity": {"bootId": linux_boot_id().unwrap(), "startTicks": identity.start_time}
        })).unwrap(), 0o600).unwrap();
        daemons.push(child);
    }
    let cgroup_root = tmp.path().join("cgroup");
    let group_path = "/tmux-spawn-fixture.scope";
    let group_dir = cgroup_root.join(&group_path[1..]);
    fs::create_dir_all(&group_dir).unwrap();
    fs::write(group_dir.join("cgroup.events"), "populated 1\n").unwrap();
    fs::write(
        group_dir.join("cgroup.procs"),
        daemons
            .iter()
            .map(|child| format!("{}\n", child.0.id()))
            .collect::<String>(),
    )
    .unwrap();
    let metadata = fs::metadata(&group_dir).unwrap();
    let identity: TmuxRuntimeIdentity = serde_json::from_value(json!({
        "session_id": "$1", "pane_id": "%1", "pane_pid": 999999,
        "control_group": {"path": group_path, "device": metadata.dev(), "inode": metadata.ino(), "boot_id": linux_boot_id().unwrap()},
        "control_group_members": daemons.iter().map(|child| {
            let observed = read_linux_process_identity(child.0.id() as i32).unwrap().unwrap();
            json!({"pid": observed.pid, "start_time": observed.start_time})
        }).collect::<Vec<_>>()
    })).unwrap();
    // Candidate-controlled home, argv and PID files cannot grant an exemption.
    assert_eq!(
        process_observation::cgroup(
            &identity,
            identity.control_group.as_ref().unwrap(),
            &cgroup_root
        )
        .0,
        ProcessGroupStatus::Running,
        "an untrusted provider-home lookalike stays in the session boundary"
    );
    let fence = SessionRegistryFence::from_record(&record);
    assert!(
        finish_session_delete(
            &context,
            record.clone(),
            session_dir(&context, &record.id),
            fence,
            board::CloseReason::Deleted
        )
        .is_err()
    );
    assert!(session_dir(&context, &record.id).exists());
    let _trusted_home = EnvGuard::set(&lock, "CODEX_HOME", home.to_str().unwrap());
    assert_eq!(
        process_observation::cgroup(
            &identity,
            identity.control_group.as_ref().unwrap(),
            &cgroup_root
        )
        .0,
        ProcessGroupStatus::Stopped,
        "a provider-only cgroup is outside the session boundary"
    );
    let group = identity.control_group.as_ref().unwrap();
    fs::write(group_dir.join("cgroup.kill"), "0\n").unwrap();
    assert!(
        recover_persisted_linux_control_group_at_root(
            group,
            &identity.control_group_members,
            TmuxTerminationState::KillConfirmed {
                thaw_on_recovery: false
            },
            &cgroup_root
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(group_dir.join("cgroup.kill")).unwrap(),
        "0\n"
    );
    let moved_group = tmp.path().join("moved-cgroup");
    fs::rename(&group_dir, &moved_group).unwrap();
    assert!(
        recover_persisted_linux_control_group_at_root(
            group,
            &identity.control_group_members,
            TmuxTerminationState::KillConfirmed {
                thaw_on_recovery: false
            },
            &cgroup_root
        )
        .is_err()
    );
    for child in &mut daemons {
        assert!(child.0.try_wait().unwrap().is_none());
    }
    fs::rename(moved_group, &group_dir).unwrap();
    let pid_file = pid_dir.join("daemon.pid");
    let original_pid = fs::read(&pid_file).unwrap();
    let mut stale: Value = serde_json::from_slice(&original_pid).unwrap();
    stale["processIdentity"]["startTicks"] =
        json!(stale["processIdentity"]["startTicks"].as_u64().unwrap() + 1);
    nils_common::fs::write_atomic(&pid_file, &serde_json::to_vec(&stale).unwrap(), 0o600).unwrap();
    assert_eq!(
        process_observation::cgroup(&identity, group, &cgroup_root).0,
        ProcessGroupStatus::Running
    );
    nils_common::fs::write_atomic(&pid_file, &original_pid, 0o600).unwrap();
    let directory = session_dir(&context, &record.id);
    let result = finish_session_delete(
        &context,
        record.clone(),
        directory.clone(),
        SessionRegistryFence::from_record(&record),
        board::CloseReason::Deleted,
    )
    .unwrap();
    assert!(!directory.exists());
    let value = serde_json::to_value(&result).unwrap();
    let retained = value["process_inventory"]["retained_shared_daemons"]
        .as_array()
        .unwrap();
    assert_eq!(retained.len(), 2);
    for child in &mut daemons {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "shared provider fixture was killed"
        );
        assert!(
            retained
                .iter()
                .any(|row| row["pid"] == child.0.id() && row["reason"] == "shared-daemon-retained")
        );
    }
}

#[test]
fn orphan_maintenance_preparation_refuses_a_mixed_shared_provider_scope() {
    let lock = GlobalStateLock::new();
    let (tmp, context, record) = session_fixture();
    let home = tmp.path().join("provider-home");
    let bin = home.join("packages/app-server-daemon/releases/fixture/bin/codex");
    fs::create_dir_all(bin.parent().unwrap()).unwrap();
    let source = tmp.path().join("daemon.c");
    fs::write(
        &source,
        "#include <unistd.h>\nint main(void) { setsid(); for (;;) pause(); }\n",
    )
    .unwrap();
    assert!(
        Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&bin)
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink(
        "releases/fixture",
        home.join("packages/app-server-daemon/current"),
    )
    .unwrap();
    let pid_dir = home.join("app-server-daemon");
    fs::create_dir(&pid_dir).unwrap();
    let _home = EnvGuard::set(&lock, "CODEX_HOME", home.to_str().unwrap());
    let daemon_pid_file = tmp.path().join("provider.pid");
    let worker_pid_file = tmp.path().join("worker.pid");
    let unit = format!("tmux-spawn-{}.scope", uuid::Uuid::new_v4());
    let mut command = Command::new("systemd-run");
    command.args(["--user", "--scope", "--quiet", "--collect", "--unit", &unit]);
    if let Ok(slice) = std::env::var("NILS_CLI_GATE_SLICE") {
        command.arg("--slice").arg(slice);
    }
    command.args(["--", "/bin/sh", "-c",
        "\"$1\" app-server --listen unix:// --managed-daemon & printf '%s\\n' \"$!\" > \"$2\"; printf '%s\\n' \"$$\" > \"$3\"; exec sleep 60",
        "fixture"])
        .arg(&bin).arg(&daemon_pid_file).arg(&worker_pid_file)
        .env("AGENT_SESSION_ID", &record.id)
        .env("AGENT_SESSION_RUNTIME_ID", &record.runtime.as_ref().unwrap().launch_id)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    struct Scope {
        unit: String,
        child: Child,
    }
    impl Drop for Scope {
        fn drop(&mut self) {
            // The UUID scope contains only this test's disposable processes.
            let _ = Command::new("systemctl")
                .args(["--user", "stop", &self.unit])
                .status();
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
    let mut scope = Scope {
        unit,
        child: command.spawn().unwrap(),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    let read_pid = |path: &Path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse::<libc::pid_t>().ok())
    };
    let (daemon_pid, worker_pid) = loop {
        if let (Some(daemon), Some(worker)) =
            (read_pid(&daemon_pid_file), read_pid(&worker_pid_file))
            && read_linux_process_identity(daemon)
                .unwrap()
                .is_some_and(|identity| identity.session_id == daemon)
        {
            break (daemon, worker);
        }
        assert!(
            scope.child.try_wait().unwrap().is_none(),
            "fixture scope exited during startup"
        );
        assert!(
            Instant::now() < deadline,
            "native mixed-scope fixture did not start"
        );
        thread::sleep(Duration::from_millis(10));
    };
    let daemon = read_linux_process_identity(daemon_pid).unwrap().unwrap();
    let worker = read_linux_process_identity(worker_pid).unwrap().unwrap();
    let group = linux_process_control_group(worker_pid)
        .unwrap()
        .expect("owned dedicated scope");
    assert_eq!(
        linux_process_control_group(daemon_pid).unwrap().as_ref(),
        Some(&group)
    );
    nils_common::fs::write_atomic(&pid_dir.join("daemon.pid"), &serde_json::to_vec(&json!({
        "pid": daemon_pid, "processIdentity": {"bootId": linux_boot_id().unwrap(), "startTicks": daemon.start_time}
    })).unwrap(), 0o600).unwrap();
    assert!(crate::delete_recovery::codex_daemons::proven_pid(
        daemon_pid,
        Some(daemon.start_time)
    ));
    let identity: TmuxRuntimeIdentity = serde_json::from_value(json!({
        "launch_id": record.runtime.as_ref().unwrap().launch_id,
        "session_id": "$99", "pane_id": "%99", "pane_pid": worker_pid,
        "pane_start_time": worker.start_time, "process_group_id": worker.process_group_id,
        "process_session_id": worker.session_id,
        "process_session_members": [{"pid": worker_pid, "start_time": worker.start_time}],
        "control_group_members": [{"pid": worker_pid, "start_time": worker.start_time}, {"pid": daemon_pid, "start_time": daemon.start_time}],
        "control_group": group
    })).unwrap();
    let tmux = tmp.path().join("absent-tmux");
    let _status = tmux_probe_fixture::install_status(&tmux, tmux_probe_fixture::Status::Stopped);
    assert_eq!(
        verified_tmux_status_with_timeout(&tmux, "=fixture", Duration::from_secs(1)),
        "stopped"
    );
    assert_eq!(
        process_observation::runtime(&identity).0,
        ProcessGroupStatus::Running
    );
    let path = linux_control_group_full_path(Path::new(&group.path)).unwrap();
    let freeze_before = fs::read(path.join("cgroup.freeze")).unwrap();
    // Orphan maintenance's AlreadyStopped transaction uses this common owner
    // before persisting FreezePending, freezing, or writing cgroup.kill.
    let prepared = crate::runtime_teardown::split_scope::prepare_process_runtime(&identity, false);
    assert!(
        matches!(
            &prepared,
            Err(SessionTerminationFailure::RuntimeIdentityUnavailable)
        ),
        "maintenance must refuse shared infrastructure before preparing a cgroup kill; preparation_succeeded={}, failure={:?}",
        prepared.is_ok(),
        prepared.as_ref().err()
    );
    assert_eq!(fs::read(path.join("cgroup.freeze")).unwrap(), freeze_before);
    assert!(persisted_tmux_termination_state(&record).unwrap().is_none());
    for observed in [&daemon, &worker] {
        let current = read_linux_process_identity(observed.pid).unwrap().unwrap();
        assert!(
            !current.zombie && current.start_time == observed.start_time,
            "fixture was terminated"
        );
    }
    assert!(session_dir(&context, &record.id).exists());
}

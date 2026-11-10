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

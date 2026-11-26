use super::*;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use std::sync::atomic::{AtomicBool, Ordering};

const FIXTURE_TEST: &str = "delete_recovery::tests::macos_close::orphan_fixture_process";
const FIXTURE_PATH: &str = "NILS_TEST_MACOS_CLOSE_PATH";
const FIXTURE_ORPHAN: &str = "NILS_TEST_MACOS_CLOSE_ORPHAN";
const FIXTURE_TIMEOUT: Duration = Duration::from_secs(20);
static SAW_TERM: AtomicBool = AtomicBool::new(false);

extern "C" fn note_term(_: libc::c_int) {
    SAW_TERM.store(true, Ordering::Relaxed);
}

#[test]
#[allow(clippy::zombie_processes)] // Exiting deliberately reparents the child to launchd.
fn orphan_fixture_process() {
    let Some(path) = std::env::var_os(FIXTURE_PATH).map(PathBuf::from) else {
        return;
    };
    if std::env::var_os(FIXTURE_ORPHAN).is_none() {
        let mut orphan = ProcessCommand::new(std::env::current_exe().unwrap());
        orphan
            .args(std::env::args_os().skip(1))
            .env(FIXTURE_ORPHAN, "1");
        // Start a new POSIX session, so this child also escapes the pane group.
        unsafe {
            orphan.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        // The fixture intentionally exits without reaping its detached child.
        let _orphan = orphan.spawn().unwrap();
        wait_until(|| path.with_extension("leave").is_file());
        return;
    }

    unsafe { libc::signal(libc::SIGTERM, note_term as *const () as libc::sighandler_t) };
    fs::write(path.with_extension("ready"), std::process::id().to_string()).unwrap();
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    while path.parent().unwrap().is_dir() && Instant::now() < deadline {
        if SAW_TERM.swap(false, Ordering::Relaxed) {
            fs::write(path.with_extension("term"), b"").unwrap();
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + FIXTURE_TIMEOUT;
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "fixture did not reach the expected state"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn fixture_process(pid: libc::pid_t) -> Option<(u32, bool)> {
    fixture_process_info(pid).map(|info| (info.pbi_ppid, info.pbi_status == libc::SZOMB))
}

fn fixture_process_info(pid: libc::pid_t) -> Option<libc::proc_bsdinfo> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let length = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    (length == size).then(|| unsafe { info.assume_init() })
}

fn start_orphan(
    executable: &Path,
    path: &Path,
    record: &SessionRecord,
    marked: bool,
    server: bool,
) -> (std::process::Child, libc::pid_t) {
    let launch = &record.runtime.as_ref().unwrap().launch_id;
    let mut launcher = ProcessCommand::new(executable);
    launcher
        .args(["--exact", FIXTURE_TEST, "--skip"])
        .arg(format!("AGENT_SESSION_ID={}", record.id))
        .arg("--skip")
        .arg(format!("AGENT_SESSION_RUNTIME_ID={launch}"))
        .env(FIXTURE_PATH, path)
        .env_remove(FIXTURE_ORPHAN)
        .env_remove("AGENT_SESSION_ID")
        .env_remove("AGENT_SESSION_RUNTIME_ID")
        .env_remove("SCCACHE_START_SERVER")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if marked {
        launcher
            .env("AGENT_SESSION_ID", &record.id)
            .env("AGENT_SESSION_RUNTIME_ID", launch);
    }
    if server {
        launcher.env("SCCACHE_START_SERVER", "1");
    }
    unsafe {
        launcher.pre_exec(|| {
            if libc::setsid() < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
    let child = launcher.spawn().unwrap();
    let ready = path.with_extension("ready");
    wait_until(|| fs::read_to_string(&ready).is_ok_and(|pid| !pid.is_empty()));
    let pid = fs::read_to_string(ready).unwrap().parse().unwrap();
    assert_eq!(fixture_process(pid), Some((child.id(), false)));
    (child, pid)
}

fn exit_launcher(mut launcher: std::process::Child, path: &Path, orphan_pid: libc::pid_t) {
    fs::write(path.with_extension("leave"), b"").unwrap();
    assert!(launcher.wait().unwrap().success());
    assert_eq!(fixture_process(orphan_pid), Some((1, false)));
}

#[test]
fn ordinary_macos_close_stops_only_marked_session_orphans() {
    let (tmp, context, mut record) = session_fixture();
    let directory = session_dir(&context, &record.id);
    let shared_executable = tmp.path().join("sccache");
    fs::copy(std::env::current_exe().unwrap(), &shared_executable).unwrap();

    let orphan_path = tmp.path().join("orphan");
    let (launcher, orphan_pid) =
        start_orphan(&shared_executable, &orphan_path, &record, true, false);
    let pane = launcher.id() as libc::pid_t;
    let tmux_probe_fixture::Probe::Running(identity) = tmux_probe_fixture::running_identity(
        "$42",
        "%42",
        pane,
        Some(pane),
        Some(&record.runtime.as_ref().unwrap().launch_id),
    ) else {
        unreachable!()
    };
    persist_tmux_runtime_identity(&mut record, &identity).unwrap();
    write_session_record(&context, &record).unwrap();
    exit_launcher(launcher, &orphan_path, orphan_pid);

    let lookalike_path = tmp.path().join("lookalike");
    let (launcher, lookalike_pid) =
        start_orphan(&shared_executable, &lookalike_path, &record, false, false);
    exit_launcher(launcher, &lookalike_path, lookalike_pid);
    let daemon_path = tmp.path().join("daemon");
    let (launcher, daemon_pid) =
        start_orphan(&shared_executable, &daemon_path, &record, true, true);
    exit_launcher(launcher, &daemon_path, daemon_pid);

    let attempted_signals = std::cell::RefCell::new(Vec::new());
    let refusal = macos_orphans::terminate_with(&context, &record, |_, signal| {
        attempted_signals.borrow_mut().push(signal);
    })
    .unwrap_err();
    assert_eq!(
        attempted_signals.into_inner(),
        [libc::SIGTERM, libc::SIGKILL]
    );
    assert_eq!(refusal.code(), "session-termination-failed");
    let details = refusal.0.details.unwrap();
    assert_eq!(details["reason"], "process-still-running");
    assert_eq!(details["action"], "delete-with-leave-processes");
    let blocking = details["blocking_processes"].as_array().unwrap();
    assert_eq!(blocking.len(), 1);
    assert_eq!(blocking[0]["pid"], orphan_pid);
    assert_eq!(
        refusal.0.hint.unwrap(),
        format!(
            "To close while leaving the listed processes running, run `agent-session delete --leave-processes -- {}`.",
            record.id
        )
    );
    assert!(directory.exists());

    let tmux = tmp.path().join("tmux");
    let _capture = tmux_probe_fixture::install(&tmux, tmux_probe_fixture::Probe::Stopped);
    let _status = tmux_probe_fixture::install_status(&tmux, tmux_probe_fixture::Status::Stopped);
    let result = delete_session_with_timeouts(
        &context,
        &record.id,
        tmux,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(result.deleted);
    assert!(!directory.exists());
    assert!(orphan_path.with_extension("term").is_file());
    assert!(fixture_process(orphan_pid).is_none_or(|(_, zombie)| zombie));
    assert_eq!(fixture_process(lookalike_pid), Some((1, false)));
    assert!(!lookalike_path.with_extension("term").exists());
    assert_eq!(fixture_process(daemon_pid), Some((1, false)));
    assert!(!daemon_path.with_extension("term").exists());
}

#[test]
fn ordinary_macos_close_retains_proven_codex_provider_daemon() {
    let lock = GlobalStateLock::new();
    let (tmp, context, mut record) = session_fixture();
    let home = tmp.path().join("codex-home");
    let package = home.join("packages/app-server-daemon");
    let executable = package.join("releases/fixture/bin/codex");
    fs::create_dir_all(executable.parent().unwrap()).unwrap();
    fs::copy(std::env::current_exe().unwrap(), &executable).unwrap();
    std::os::unix::fs::symlink("releases/fixture", package.join("current")).unwrap();
    let pid_dir = home.join("app-server-daemon");
    fs::create_dir(&pid_dir).unwrap();
    let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", home.to_str().unwrap());

    let daemon_path = tmp.path().join("provider-daemon");
    let (launcher, daemon_pid) = start_orphan(&executable, &daemon_path, &record, true, false);
    let pane = launcher.id() as libc::pid_t;
    let tmux_probe_fixture::Probe::Running(identity) = tmux_probe_fixture::running_identity(
        "$42",
        "%42",
        pane,
        Some(pane),
        Some(&record.runtime.as_ref().unwrap().launch_id),
    ) else {
        unreachable!()
    };
    persist_tmux_runtime_identity(&mut record, &identity).unwrap();
    write_session_record(&context, &record).unwrap();
    exit_launcher(launcher, &daemon_path, daemon_pid);

    let info = fixture_process_info(daemon_pid).unwrap();
    // proc_uniqidentifierinfo is 56 bytes with the unique ID after the UUID.
    let mut unique_info = [0_u64; 7];
    let bytes = std::mem::size_of_val(&unique_info) as libc::c_int;
    assert_eq!(
        unsafe { libc::proc_pidinfo(daemon_pid, 17, 1, unique_info.as_mut_ptr().cast(), bytes) },
        bytes
    );
    let pid_record = json!({
        "pid": daemon_pid,
        "processIdentity": {
            "bootId": capture_macos_boot_id().unwrap(),
            "uniqueId": unique_info[2],
            "startSeconds": info.pbi_start_tvsec,
            "startMicroseconds": info.pbi_start_tvusec,
        },
    });
    nils_common::fs::write_atomic(
        &pid_dir.join("daemon.pid"),
        &serde_json::to_vec(&pid_record).unwrap(),
        0o600,
    )
    .unwrap();

    let orphan_path = tmp.path().join("session-codex");
    let (launcher, orphan_pid) = start_orphan(&executable, &orphan_path, &record, true, false);
    exit_launcher(launcher, &orphan_path, orphan_pid);
    let mut stale_record = pid_record;
    stale_record["pid"] = json!(orphan_pid);
    nils_common::fs::write_atomic(
        &pid_dir.join("daemon-updater.pid"),
        &serde_json::to_vec(&stale_record).unwrap(),
        0o600,
    )
    .unwrap();

    let tmux = tmp.path().join("tmux");
    let _capture = tmux_probe_fixture::install(&tmux, tmux_probe_fixture::Probe::Stopped);
    let _status = tmux_probe_fixture::install_status(&tmux, tmux_probe_fixture::Status::Stopped);
    let result = delete_session_with_timeouts(
        &context,
        &record.id,
        tmux,
        Duration::from_secs(1),
        Duration::from_secs(1),
    )
    .unwrap();
    assert!(result.deleted);
    assert_eq!(fixture_process(daemon_pid), Some((1, false)));
    assert!(!daemon_path.with_extension("term").exists());
    let retained = &result.process_inventory.retained_shared_daemons;
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].pid, daemon_pid);
    assert_eq!(retained[0].command, "codex-app-server");
    assert!(orphan_path.with_extension("term").is_file());
    assert!(fixture_process(orphan_pid).is_none_or(|(_, zombie)| zombie));
}

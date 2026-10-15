use super::*;

#[cfg(target_os = "linux")]
pub(super) struct SupervisorFixture {
    pub(super) child: Child,
    pub(super) completion_channel: Option<std::os::unix::net::UnixStream>,
    pub(super) reaped: bool,
}

#[cfg(target_os = "linux")]
impl SupervisorFixture {
    #[cfg(target_os = "linux")]
    pub(super) fn id(&self) -> u32 {
        self.child.id()
    }

    pub(super) fn abandon_owner(&mut self) {
        drop(self.completion_channel.take());
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        let status = self.child.try_wait()?;
        if status.is_some() {
            self.reaped = true;
        }
        Ok(status)
    }

    pub(super) fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        let status = self.child.wait()?;
        self.reaped = true;
        Ok(status)
    }
}

#[cfg(target_os = "linux")]
impl Drop for SupervisorFixture {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        unsafe {
            libc::kill(-(self.child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn pin_published_process_with<C: OwnedProcessControl>(
    control: &mut C,
    identity: ProcessIdentity,
) -> Result<Option<C::Pinned>> {
    control.pin(identity)
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn signal_published_process_with<C: OwnedProcessControl>(
    control: &mut C,
    process: &C::Pinned,
) -> Result<()> {
    control.signal(process, libc::SIGKILL).map(|_| ())
}

#[cfg(target_os = "linux")]
pub(super) struct PublishedPidCleanup(Option<PinnedProcess>);

#[cfg(target_os = "linux")]
impl PublishedPidCleanup {
    pub(super) fn new(identity: ProcessIdentity) -> Self {
        let mut control = SystemProcessControl;
        let process = pin_published_process_with(&mut control, identity)
            .ok()
            .flatten();
        Self(process)
    }

    #[cfg(target_os = "linux")]
    pub(super) fn process(&self) -> Option<&PinnedProcess> {
        self.0.as_ref()
    }

    pub(super) fn terminate_and_wait(&mut self) -> Result<()> {
        let Some(process) = &self.0 else {
            return Ok(());
        };
        let mut control = SystemProcessControl;
        signal_published_process_with(&mut control, process)?;
        wait_pinned_process_until(process, Instant::now() + Duration::from_secs(1))?;
        self.0 = None;
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for PublishedPidCleanup {
    fn drop(&mut self) {
        let _ = self.terminate_and_wait();
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn fixture_monotonic_deadline(timeout: Duration) -> u64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) },
        0,
        "read fixture monotonic clock"
    );
    let now = u64::try_from(now.tv_sec)
        .expect("nonnegative monotonic seconds")
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(u64::try_from(now.tv_nsec).ok()?))
        .expect("bounded monotonic clock");
    now.checked_add(u64::try_from(timeout.as_nanos()).expect("bounded fixture timeout"))
        .expect("bounded fixture deadline")
}

#[cfg(target_os = "linux")]
pub(super) fn spawn_authenticated_supervisor_fixture(
    scenario: &str,
    pid_path: &Path,
) -> SupervisorFixture {
    spawn_authenticated_supervisor_fixture_with_timeout(scenario, pid_path, Duration::from_secs(5))
}

#[cfg(target_os = "linux")]
pub(super) fn spawn_authenticated_supervisor_fixture_with_timeout(
    scenario: &str,
    pid_path: &Path,
    timeout: Duration,
) -> SupervisorFixture {
    spawn_authenticated_supervisor_fixture_with_timeout_and_delay(
        scenario,
        pid_path,
        timeout,
        Duration::ZERO,
    )
}

#[cfg(target_os = "linux")]
pub(super) fn spawn_authenticated_supervisor_fixture_with_timeout_and_delay(
    scenario: &str,
    pid_path: &Path,
    timeout: Duration,
    capability_delay: Duration,
) -> SupervisorFixture {
    use std::os::unix::net::UnixStream;

    let deadline_nanos = fixture_monotonic_deadline(timeout);
    let (mut sender, receiver) = UnixStream::pair().expect("fixture capability socket");
    let child = Command::new(env::current_exe().expect("current test executable"))
        .arg("authenticated_process_supervisor_test_fixture")
        .arg("--nocapture")
        .env(PROCESS_SUPERVISOR_ENV, "1")
        .env(
            PROCESS_SUPERVISOR_CAPABILITY_FD_ENV,
            libc::STDIN_FILENO.to_string(),
        )
        .env("NILS_PROCESS_SUPERVISOR_TEST_SCENARIO", scenario)
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", pid_path)
        .stdin(Stdio::from(OwnedFd::from(receiver)))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn authenticated supervisor fixture");
    let mut child = child;
    std::thread::sleep(capability_delay);
    if let Err(error) = sender.write_all(PROCESS_SUPERVISOR_CAPABILITY) {
        let _ = child.kill();
        let _ = child.wait();
        panic!("deliver fixture capability: {error}");
    }
    if let Err(error) = sender.write_all(&deadline_nanos.to_be_bytes()) {
        unsafe {
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("deliver fixture deadline: {error}");
    }
    SupervisorFixture {
        child,
        completion_channel: Some(sender),
        reaped: false,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn spawn_authenticated_supervisor_output_fixture(
    scenario: &str,
    pid_path: &Path,
) -> SpawnedOutputChild {
    use std::os::unix::net::UnixStream;

    let (mut sender, receiver) = UnixStream::pair().expect("fixture capability socket");
    let mut child = Command::new(env::current_exe().expect("current test executable"))
        .arg("authenticated_process_supervisor_test_fixture")
        .arg("--nocapture")
        .env(PROCESS_SUPERVISOR_ENV, "1")
        .env(
            PROCESS_SUPERVISOR_CAPABILITY_FD_ENV,
            libc::STDIN_FILENO.to_string(),
        )
        .env("NILS_PROCESS_SUPERVISOR_TEST_SCENARIO", scenario)
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", pid_path)
        .stdin(Stdio::from(OwnedFd::from(receiver)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .expect("spawn authenticated supervisor output fixture");
    let fallback_owner = if matches!(scenario, "internal-failure" | "target-status-one") {
        None
    } else {
        Some(
            ProcessOwner::for_root(child.id() as libc::pid_t)
                .expect("create scoped output fallback owner"),
        )
    };
    if let Err(error) = sender.write_all(PROCESS_SUPERVISOR_CAPABILITY) {
        unsafe {
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("deliver output fixture capability: {error}");
    }
    if let Err(error) =
        sender.write_all(&fixture_monotonic_deadline(Duration::from_secs(5)).to_be_bytes())
    {
        unsafe {
            let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("deliver output fixture deadline: {error}");
    }
    SpawnedOutputChild {
        child,
        supervised: true,
        cleanup_proof: Some(sender),
        fallback_owner,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run_process_supervisor_capability_fixture(
    scenario: &str,
    capability: Option<&[u8]>,
) -> std::process::ExitStatus {
    use std::os::unix::net::UnixStream;

    let sockets = capability.map(|_| UnixStream::pair().expect("capability test socket"));
    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("process_supervisor_capability_validation_test_fixture")
        .arg("--nocapture")
        .env("NILS_PROCESS_SUPERVISOR_CAPABILITY_TEST", scenario)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    if scenario == "malformed-fd" {
        command.env(PROCESS_SUPERVISOR_CAPABILITY_FD_ENV, "not-a-descriptor");
    }
    if sockets.is_some() {
        command.env(
            PROCESS_SUPERVISOR_CAPABILITY_FD_ENV,
            libc::STDIN_FILENO.to_string(),
        );
    }
    let mut sockets = sockets.map(|(sender, receiver)| {
        command.stdin(Stdio::from(OwnedFd::from(receiver)));
        sender
    });
    let mut child = command
        .spawn()
        .expect("spawn capability validation fixture");
    if let (Some(bytes), Some(sender)) = (capability, sockets.as_mut()) {
        sender
            .write_all(bytes)
            .expect("deliver capability validation bytes");
        sender
            .write_all(&fixture_monotonic_deadline(Duration::from_secs(1)).to_be_bytes())
            .expect("deliver capability validation deadline");
    }
    child
        .wait()
        .expect("wait for capability validation fixture")
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_supervisor_capability_validation_test_fixture() {
    let Some(scenario) = env::var_os("NILS_PROCESS_SUPERVISOR_CAPABILITY_TEST") else {
        return;
    };
    let result = if scenario == OsStr::new("valid-untrusted-parent") {
        validate_process_supervisor_capability_with(|| false)
    } else {
        validate_process_supervisor_capability()
    };
    if scenario == OsStr::new("valid") {
        result.expect("trusted parent with the exact capability must authenticate");
    } else {
        result.expect_err("invalid supervisor capability boundary must fail closed");
    }
}

#[test]
#[cfg(target_os = "linux")]
fn process_supervisor_wrong_peer_sender_test_fixture() {
    use std::os::unix::net::UnixStream;

    let Some(socket_path) = env::var_os("NILS_PROCESS_SUPERVISOR_WRONG_PEER_SOCKET") else {
        return;
    };
    let mut sender = UnixStream::connect(socket_path).expect("connect wrong-peer sender");
    sender
        .write_all(PROCESS_SUPERVISOR_CAPABILITY)
        .expect("deliver wrong-peer capability");
    sender
        .write_all(&fixture_monotonic_deadline(Duration::from_secs(1)).to_be_bytes())
        .expect("deliver wrong-peer deadline");
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_supervisor_capability_boundaries_are_independent_and_fail_closed() {
    let mismatched = vec![b'x'; PROCESS_SUPERVISOR_CAPABILITY.len()];
    for (scenario, capability) in [
        ("missing-fd", None),
        ("malformed-fd", None),
        ("mismatched-capability", Some(mismatched.as_slice())),
        (
            "valid-untrusted-parent",
            Some(PROCESS_SUPERVISOR_CAPABILITY),
        ),
        ("valid", Some(PROCESS_SUPERVISOR_CAPABILITY)),
    ] {
        let status = run_process_supervisor_capability_fixture(scenario, capability);
        assert!(
            status.success(),
            "capability boundary fixture failed: {scenario}"
        );
    }
}

#[test]
#[cfg(target_os = "linux")]
fn process_supervisor_rejects_capability_from_non_parent_peer() {
    use std::os::unix::net::UnixListener;

    let root = tempfile::TempDir::new().expect("wrong-peer capability root");
    let socket_path = root.path().join("capability.sock");
    let listener = UnixListener::bind(&socket_path).expect("bind wrong-peer capability socket");
    let mut sender = Command::new(env::current_exe().expect("current test executable"))
        .arg("process_supervisor_wrong_peer_sender_test_fixture")
        .arg("--nocapture")
        .env("NILS_PROCESS_SUPERVISOR_WRONG_PEER_SOCKET", &socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn wrong-peer capability sender");
    let (receiver, _) = listener.accept().expect("accept wrong-peer capability");
    let descriptor = receiver.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    assert!(flags >= 0, "read wrong-peer descriptor flags");
    assert!(
        unsafe { libc::fcntl(descriptor, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } >= 0,
        "make wrong-peer descriptor inheritable"
    );
    let mut validator = Command::new(env::current_exe().expect("current test executable"))
        .arg("process_supervisor_capability_validation_test_fixture")
        .arg("--nocapture")
        .env("NILS_PROCESS_SUPERVISOR_CAPABILITY_TEST", "wrong-peer")
        .env(PROCESS_SUPERVISOR_CAPABILITY_FD_ENV, descriptor.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .expect("spawn wrong-peer capability validator");
    drop(receiver);

    assert!(
        validator
            .wait()
            .expect("wait for wrong-peer validator")
            .success(),
        "a non-parent peer must fail authentication"
    );
    assert!(
        sender.wait().expect("wait for wrong-peer sender").success(),
        "wrong-peer sender fixture failed"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn process_supervisor_peer_credentials_reject_wrong_parent_or_user() {
    let expected_parent = 42;
    let expected_uid = 1_000;
    let credentials = libc::ucred {
        pid: expected_parent,
        uid: expected_uid,
        gid: 1_000,
    };

    assert!(linux_peer_credentials_match(
        credentials,
        expected_parent,
        expected_uid
    ));
    assert!(!linux_peer_credentials_match(
        libc::ucred {
            pid: expected_parent + 1,
            ..credentials
        },
        expected_parent,
        expected_uid,
    ));
    assert!(!linux_peer_credentials_match(
        libc::ucred {
            uid: expected_uid + 1,
            ..credentials
        },
        expected_parent,
        expected_uid,
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
struct SupervisorCompletionFixtureOwner;

#[cfg(any(target_os = "linux", target_os = "macos"))]
impl OwnedProcessTracker for SupervisorCompletionFixtureOwner {
    fn identities(&mut self, deadline: Instant) -> Result<Vec<ProcessIdentity>> {
        ensure_deadline(deadline)?;
        Ok(Vec::new())
    }

    fn known_identities(&self) -> Vec<ProcessIdentity> {
        Vec::new()
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn authenticated_process_supervisor_test_fixture() {
    let Some(scenario) = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_SCENARIO") else {
        return;
    };
    let authenticated =
        validate_process_supervisor_capability().expect("authenticate fixture parent");
    let program = if scenario == OsStr::new("internal-failure") {
        PathBuf::from("/nils-supervisor-fixture/missing-target")
    } else {
        env::current_exe().expect("current test executable")
    };
    let arguments = vec![
        OsString::from("process_supervisor_target_test_fixture"),
        OsString::from("--nocapture"),
    ];
    let expected = match scenario.to_str() {
        Some("successful-child") => exit::SUCCESS,
        Some(
            "blocked-child"
            | "detached-double-fork"
            | "escaped-output-timeout"
            | "target-status-one",
        ) => exit::RUNTIME,
        Some("internal-failure") => {
            let mut owner = SupervisorCompletionFixtureOwner;
            supervise_process_with_cleanup_proof_and_owner(
                program.into_os_string(),
                &arguments,
                Some(authenticated.channel),
                authenticated.deadline,
                &mut owner,
            )
            .expect_err("fixture target launch must fail internally");
            std::process::exit(exit::RUNTIME);
        }
        Some("target-signal") => {
            supervise_process_with_cleanup_proof(
                program.into_os_string(),
                &arguments,
                Some(authenticated.channel),
                authenticated.deadline,
            )
            .expect_err("fixture target launch must fail internally");
            std::process::exit(exit::RUNTIME);
        }
        _ => panic!("unsupported supervisor fixture scenario"),
    };
    let status = if scenario == OsStr::new("target-status-one") {
        let mut owner = SupervisorCompletionFixtureOwner;
        supervise_process_with_cleanup_proof_and_owner(
            program.into_os_string(),
            &arguments,
            Some(authenticated.channel),
            authenticated.deadline,
            &mut owner,
        )
    } else {
        supervise_process_with_cleanup_proof(
            program.into_os_string(),
            &arguments,
            Some(authenticated.channel),
            authenticated.deadline,
        )
    }
    .expect("run supervisor fixture");
    if scenario == OsStr::new("target-status-one") {
        std::process::exit(status);
    }
    assert_eq!(status, expected);
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::zombie_processes)] // The supervisor, not this short-lived fixture parent, owns reaping.
fn process_supervisor_target_test_fixture() {
    let Some(scenario) = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_SCENARIO") else {
        return;
    };
    let pid_path = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH").expect("fixture pid path");
    match scenario.to_str() {
        Some("target-signal") => {
            unsafe {
                libc::kill(libc::getpid(), libc::SIGKILL);
            }
            unreachable!("SIGKILL target survived");
        }
        Some("target-status-one") => std::process::exit(exit::RUNTIME),
        Some("blocked-child") => {
            fs::write(pid_path, unsafe { libc::getpid() }.to_string())
                .expect("publish blocked fixture PID");
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        Some("successful-child") => {
            let mut child = Command::new(env::current_exe().expect("current test executable"))
                .arg("process_supervisor_leaf_test_fixture")
                .arg("--nocapture")
                .env("NILS_PROCESS_SUPERVISOR_LEAF", "1")
                .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", &pid_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn surviving fixture child");
            let deadline = Instant::now() + Duration::from_secs(1);
            while !Path::new(&pid_path).exists() {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("surviving fixture child did not publish its PID");
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        Some(scenario @ ("detached-double-fork" | "escaped-output-timeout")) => {
            let mut intermediate =
                Command::new(env::current_exe().expect("current test executable"));
            intermediate
                .arg("process_supervisor_intermediate_test_fixture")
                .arg("--nocapture")
                .env("NILS_PROCESS_SUPERVISOR_INTERMEDIATE", "1")
                .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", &pid_path)
                .stdin(Stdio::null())
                .process_group(0);
            if scenario == "escaped-output-timeout" {
                intermediate.env("NILS_PROCESS_SUPERVISOR_LEAF_INHERIT_OUTPUT", "1");
            } else {
                intermediate.stdout(Stdio::null()).stderr(Stdio::null());
            }
            intermediate
                .spawn()
                .expect("spawn detached fixture intermediate");
            while !Path::new(&pid_path).exists() {
                std::thread::sleep(Duration::from_millis(2));
            }
            loop {
                std::thread::park_timeout(Duration::from_secs(1));
            }
        }
        _ => panic!("unsupported target fixture scenario"),
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::zombie_processes)] // Exiting here deliberately reparents the leaf to the supervisor.
fn process_supervisor_intermediate_test_fixture() {
    if env::var_os("NILS_PROCESS_SUPERVISOR_INTERMEDIATE").as_deref() != Some(OsStr::new("1")) {
        return;
    }
    let pid_path = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH").expect("fixture pid path");
    let mut command = Command::new(env::current_exe().expect("current test executable"));
    command
        .arg("process_supervisor_leaf_test_fixture")
        .arg("--nocapture")
        .env("NILS_PROCESS_SUPERVISOR_LEAF", "1")
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", pid_path)
        .stdin(Stdio::null())
        .process_group(0);
    if env::var_os("NILS_PROCESS_SUPERVISOR_LEAF_INHERIT_OUTPUT").as_deref()
        != Some(OsStr::new("1"))
    {
        command.stdout(Stdio::null()).stderr(Stdio::null());
    }
    command.spawn().expect("spawn detached fixture leaf");
    if let Some(release_path) = env::var_os("NILS_PROCESS_SUPERVISOR_INTERMEDIATE_RELEASE_PATH") {
        while !Path::new(&release_path).exists() {
            std::thread::sleep(Duration::from_millis(2));
        }
    } else if let Some(delay) = env::var_os("NILS_PROCESS_SUPERVISOR_INTERMEDIATE_EXIT_DELAY_MS")
        .and_then(|value| value.to_str().and_then(|value| value.parse::<u64>().ok()))
    {
        std::thread::sleep(Duration::from_millis(delay));
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn process_supervisor_leaf_test_fixture() {
    if env::var_os("NILS_PROCESS_SUPERVISOR_LEAF").as_deref() != Some(OsStr::new("1")) {
        return;
    }
    let pid_path = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH").expect("fixture pid path");
    let staged = Path::new(&pid_path).with_extension("staged");
    fs::write(&staged, unsafe { libc::getpid() }.to_string()).expect("stage leaf PID");
    fs::rename(staged, pid_path).expect("publish fixture leaf PID");
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(clippy::zombie_processes)] // The collector must own the escaped leaf after this leader exits.
fn output_cleanup_escaped_descendant_test_fixture() {
    if env::var_os("NILS_OUTPUT_CLEANUP_ESCAPED_DESCENDANT").as_deref() != Some(OsStr::new("1")) {
        return;
    }
    let pid_path = env::var_os("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH").expect("fixture pid path");
    let exit_after_publish =
        env::var_os("NILS_OUTPUT_CLEANUP_EXIT_AFTER_PUBLISH").as_deref() == Some(OsStr::new("1"));
    let hard_exit_after_publish = env::var_os("NILS_OUTPUT_CLEANUP_HARD_EXIT_AFTER_PUBLISH")
        .as_deref()
        == Some(OsStr::new("1"));
    if let Some(start_path) = env::var_os("NILS_OUTPUT_CLEANUP_START_PATH") {
        while !Path::new(&start_path).exists() {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    let mut child = Command::new(env::current_exe().expect("current test executable"));
    child
        .arg("process_supervisor_intermediate_test_fixture")
        .arg("--nocapture")
        .env("NILS_PROCESS_SUPERVISOR_INTERMEDIATE", "1")
        .env("NILS_PROCESS_SUPERVISOR_TEST_PID_PATH", &pid_path)
        .stdin(Stdio::null())
        .process_group(0);
    if !exit_after_publish {
        child.env("NILS_PROCESS_SUPERVISOR_LEAF_INHERIT_OUTPUT", "1");
    } else {
        child.stdout(Stdio::null()).stderr(Stdio::null());
        if let Some(release_path) = env::var_os("NILS_OUTPUT_CLEANUP_RELEASE_PATH") {
            child.env(
                "NILS_PROCESS_SUPERVISOR_INTERMEDIATE_RELEASE_PATH",
                release_path,
            );
        } else {
            child.env("NILS_PROCESS_SUPERVISOR_INTERMEDIATE_EXIT_DELAY_MS", "500");
        }
    }
    let mut child = child
        .spawn()
        .expect("spawn escaped output fixture intermediate");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !Path::new(&pid_path).exists() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("escaped output fixture leaf did not publish its PID");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    child
        .wait()
        .expect("reap escaped output fixture intermediate");
    if exit_after_publish {
        std::thread::sleep(Duration::from_millis(100));
        if hard_exit_after_publish {
            unsafe { libc::_exit(exit::RUNTIME) };
        }
        return;
    }
    loop {
        std::thread::park_timeout(Duration::from_secs(1));
    }
}

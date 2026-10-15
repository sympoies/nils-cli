use super::*;

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn challenge_descriptor_streams() -> (UnixStream, UnixStream) {
    #[cfg(target_os = "linux")]
    {
        UnixStream::pair().expect("challenge descriptor pair")
    }

    #[cfg(target_os = "macos")]
    {
        let root = tempfile::Builder::new()
            .prefix("git-cli-challenge-")
            .tempdir_in("/tmp")
            .expect("challenge descriptor root");
        let socket_path = root.path().join("challenge.sock");
        let listener =
            UnixListener::bind(&socket_path).expect("bind challenge descriptor listener");
        let sender =
            UnixStream::connect(&socket_path).expect("connect challenge descriptor sender");
        let (receiver, _) = listener
            .accept()
            .expect("accept challenge descriptor receiver");
        (sender, receiver)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sealed_challenge_descriptor(payload: &[u8]) -> (UnixStream, libc::c_int) {
    let (mut sender, receiver) = challenge_descriptor_streams();
    sender.write_all(payload).expect("write challenge payload");
    sender
        .shutdown(std::net::Shutdown::Write)
        .expect("seal challenge payload");
    (sender, receiver.into_raw_fd())
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn challenge_descriptor_accepts_one_exact_parent_bound_bearer() {
    let (_sender, descriptor) = sealed_challenge_descriptor(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    let consumed_descriptor = unsafe { libc::fcntl(descriptor, libc::F_DUPFD_CLOEXEC, 3) };
    assert!(consumed_descriptor > libc::STDERR_FILENO);

    let challenge =
        read_challenge_descriptor_from_peer(descriptor, unsafe { libc::getpid() }, unsafe {
            libc::geteuid()
        })
        .expect("read exact challenge frame");

    assert_eq!(
        challenge,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert!(
        read_challenge_descriptor_from_peer(
            consumed_descriptor,
            unsafe { libc::getpid() },
            unsafe { libc::geteuid() },
        )
        .is_err(),
        "consumed descriptors must fail closed"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn challenge_descriptor_authenticates_across_a_real_spawn_boundary() {
    const CHILD_ENV: &str = "NILS_TEST_CHALLENGE_DESCRIPTOR_SPAWN_CHILD";
    const CHALLENGE: &[u8] = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    if env::var_os(CHILD_ENV).is_some() {
        let descriptor = libc::STDIN_FILENO;
        assert!(unsafe { libc::fcntl(descriptor, libc::F_GETFD) } >= 0);

        #[cfg(target_os = "macos")]
        {
            let credentials = macos_challenge_descriptor_peer_credentials(descriptor)
                .expect("read spawned-parent credentials");
            assert_eq!(credentials.pid, unsafe { libc::getppid() });
            assert_eq!(credentials.effective_uid, unsafe { libc::geteuid() });
            assert_eq!(credentials.socket_uid, unsafe { libc::geteuid() });
        }

        assert!(challenge_descriptor_peer_matches(
            descriptor,
            unsafe { libc::getppid() },
            unsafe { libc::geteuid() },
        ));
        assert_eq!(
            read_challenge_descriptor(descriptor).expect("read spawned challenge"),
            std::str::from_utf8(CHALLENGE).expect("challenge UTF-8")
        );
        return;
    }

    let (mut sender, receiver) = challenge_descriptor_streams();
    let mut command = std::process::Command::new(env::current_exe().expect("current test binary"));
    command
        .arg("challenge_descriptor_authenticates_across_a_real_spawn_boundary")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .stdin(std::process::Stdio::from(OwnedFd::from(receiver)))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(target_os = "macos")]
    {
        sender.write_all(CHALLENGE).expect("write challenge");
        sender
            .shutdown(std::net::Shutdown::Write)
            .expect("seal challenge");
    }

    let child = command.spawn().expect("spawn descriptor child");

    #[cfg(target_os = "linux")]
    {
        sender.write_all(CHALLENGE).expect("write challenge");
        sender
            .shutdown(std::net::Shutdown::Write)
            .expect("seal challenge");
    }

    let output = child.wait_with_output().expect("wait for descriptor child");
    drop(sender);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn challenge_descriptor_rejects_malformed_or_wrong_process_inputs() {
    assert!(
        read_challenge_descriptor_from_peer(libc::c_int::MAX, unsafe { libc::getpid() }, unsafe {
            libc::geteuid()
        },)
        .is_err(),
        "closed or invalid descriptors must fail closed"
    );

    for payload in [
        b"short".as_slice(),
        b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".as_slice(),
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".as_slice(),
    ] {
        let (_sender, descriptor) = sealed_challenge_descriptor(payload);
        assert!(
            read_challenge_descriptor_from_peer(descriptor, unsafe { libc::getpid() }, unsafe {
                libc::geteuid()
            },)
            .is_err(),
            "malformed challenge descriptor must fail closed"
        );
    }

    let (_sender, wrong_process) = sealed_challenge_descriptor(
        b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert!(
        read_challenge_descriptor_from_peer(wrong_process, unsafe { libc::getpid() } + 1, unsafe {
            libc::geteuid()
        },)
        .is_err(),
        "a descriptor whose peer is not the expected process must fail closed"
    );

    let regular_file = tempfile::tempfile()
        .expect("challenge regular file")
        .into_raw_fd();
    assert!(
        read_challenge_descriptor_from_peer(regular_file, unsafe { libc::getpid() }, unsafe {
            libc::geteuid()
        },)
        .is_err(),
        "non-socket descriptors must fail closed"
    );
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn challenge_descriptor_enforces_one_absolute_read_deadline() {
    let (mut sender, receiver) = challenge_descriptor_streams();
    let descriptor = receiver.into_raw_fd();
    let writer = std::thread::spawn(move || {
        for _ in 0..20 {
            if sender.write_all(b"a").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let started = Instant::now();
    let result = read_challenge_descriptor_from_peer_until(
        descriptor,
        unsafe { libc::getpid() },
        unsafe { libc::geteuid() },
        Duration::from_millis(100),
    );
    let elapsed = started.elapsed();
    assert!(result.is_err(), "partial challenge frame must fail closed");
    assert!(
        elapsed < Duration::from_millis(500),
        "descriptor read exceeded one absolute deadline: {elapsed:?}"
    );
    writer.join().expect("join challenge writer");
}

#[test]
fn challenge_descriptor_argument_rejects_missing_malformed_and_legacy_inputs() {
    for raw in [
        vec!["--reason-file".to_string(), "reason.txt".to_string()],
        vec![
            "--challenge-fd".to_string(),
            "not-a-descriptor".to_string(),
            "--reason-file".to_string(),
            "reason.txt".to_string(),
        ],
        vec![
            "--challenge-fd=2".to_string(),
            "--reason-file".to_string(),
            "reason.txt".to_string(),
        ],
        vec![
            "--challenge".to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string(),
            "--reason-file".to_string(),
            "reason.txt".to_string(),
        ],
    ] {
        assert!(
            parse_adopt_args(&raw).is_err(),
            "invalid descriptor argument must fail closed: {raw:?}"
        );
    }
}

#[test]
fn challenge_descriptor_argument_accepts_stdin_or_a_non_stdio_descriptor() {
    for descriptor in ["0", "3", "198"] {
        let parsed = parse_adopt_args(&[
            "--challenge-fd".to_string(),
            descriptor.to_string(),
            "--reason-file".to_string(),
            "reason.txt".to_string(),
        ])
        .expect("parse descriptor argument")
        .expect("adopt arguments");
        assert_eq!(parsed.challenge_fd.to_string(), descriptor);
    }
}

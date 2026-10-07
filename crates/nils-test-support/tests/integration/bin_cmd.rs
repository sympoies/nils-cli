use nils_test_support::{EnvGuard, GlobalStateLock, bin, cmd, write_exe};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn resolve_prefers_env_var_with_hyphen() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("bin-path");
    let _guard = EnvGuard::set(
        &lock,
        "CARGO_BIN_EXE_test-bin",
        path.to_str().expect("path"),
    );

    assert_eq!(bin::resolve("test-bin"), path);
}

#[test]
fn resolve_prefers_env_var_with_underscore() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let path = temp.path().join("bin-path");
    let _guard = EnvGuard::set(
        &lock,
        "CARGO_BIN_EXE_test_bin",
        path.to_str().expect("path"),
    );

    assert_eq!(bin::resolve("test-bin"), path);
}

#[test]
fn cmd_output_into_output_preserves_fields_and_exit_code() {
    let output = cmd::CmdOutput {
        code: 7,
        stdout: b"stdout bytes".to_vec(),
        stderr: b"stderr bytes".to_vec(),
    };

    let output = output.into_output();
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"stdout bytes");
    assert_eq!(output.stderr, b"stderr bytes");
}

#[test]
fn cmd_output_into_output_maps_negative_code_to_failure() {
    let output = cmd::CmdOutput {
        code: -1,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };

    let output = output.into_output();
    assert_eq!(output.status.code(), Some(1));
    assert!(!output.status.success());
}

#[cfg(unix)]
#[test]
fn run_captures_exit_code_stdout_stderr_and_env() {
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
printf "%s" "$TEST_ENV"
cat - 1>&2
exit 3
"#;
    write_exe(temp.path(), "cmd-test", script);

    let bin = temp.path().join("cmd-test");
    let output = cmd::run(&bin, &[], &[("TEST_ENV", "hello")], Some(b"world"));

    assert_eq!(output.code, 3);
    assert_eq!(output.success(), false);
    assert_eq!(output.stdout, b"hello");
    assert_eq!(output.stderr, b"world");
}

#[cfg(unix)]
#[test]
fn run_in_dir_sets_working_directory() {
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
pwd
"#;
    write_exe(temp.path(), "pwd-test", script);

    let bin = temp.path().join("pwd-test");
    let output = cmd::run_in_dir(temp.path(), &bin, &[], &[], None);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout = stdout.trim_end();
    let expected = std::fs::canonicalize(temp.path()).expect("canonical");
    let expected = expected.to_string_lossy();
    assert_eq!(stdout, expected);
}

#[cfg(unix)]
#[test]
fn run_with_env_remove_prefix_clears_matching_variables() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
printf "%s|%s" "${NTS_REMOVE_ME-unset}" "${NTS_KEEP_ME-unset}"
"#;
    write_exe(temp.path(), "env-prefix-test", script);
    let bin = temp.path().join("env-prefix-test");

    let _remove_guard = EnvGuard::set(&lock, "NTS_REMOVE_ME", "present");
    let _keep_guard = EnvGuard::set(&lock, "NTS_KEEP_ME", "present");

    let options = cmd::CmdOptions::new().with_env_remove_prefix("NTS_REMOVE_");
    let output = cmd::run_with(&bin, &[], &options);

    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_text(), "unset|present");
}

#[cfg(unix)]
#[test]
fn run_with_env_remove_many_clears_all_requested_variables() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
printf "%s|%s|%s" "${NTS_REMOVE_A-unset}" "${NTS_REMOVE_B-unset}" "${NTS_KEEP-unset}"
"#;
    write_exe(temp.path(), "env-remove-many-test", script);
    let bin = temp.path().join("env-remove-many-test");

    let _remove_a = EnvGuard::set(&lock, "NTS_REMOVE_A", "present");
    let _remove_b = EnvGuard::set(&lock, "NTS_REMOVE_B", "present");
    let _keep = EnvGuard::set(&lock, "NTS_KEEP", "present");

    let options = cmd::CmdOptions::new().with_env_remove_many(&["NTS_REMOVE_A", "NTS_REMOVE_B"]);
    let output = cmd::run_with(&bin, &[], &options);

    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_text(), "unset|unset|present");
}

#[cfg(unix)]
#[test]
fn run_resolved_in_dir_with_stdin_str_supports_optional_text_stdin() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
printf "%s|" "${NTS_VALUE-unset}"
cat -
"#;
    write_exe(temp.path(), "resolved-stdin-test", script);
    let bin = temp.path().join("resolved-stdin-test");
    let _guard = EnvGuard::set(
        &lock,
        "CARGO_BIN_EXE_resolved-stdin-test",
        bin.to_str().expect("path"),
    );

    let with_text = cmd::run_resolved_in_dir_with_stdin_str(
        "resolved-stdin-test",
        temp.path(),
        &[],
        &[("NTS_VALUE", "ok")],
        Some("payload"),
    );
    assert_eq!(with_text.code, 0);
    assert_eq!(with_text.stdout_text(), "ok|payload");

    let without_text = cmd::run_resolved_in_dir_with_stdin_str(
        "resolved-stdin-test",
        temp.path(),
        &[],
        &[("NTS_VALUE", "ok")],
        None,
    );
    assert_eq!(without_text.code, 0);
    assert_eq!(without_text.stdout_text(), "ok|");
}

#[cfg(unix)]
#[test]
fn run_with_env_set_wins_after_env_remove() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let script = r#"#!/bin/sh
printf "%s" "${NTS_VALUE-unset}"
"#;
    write_exe(temp.path(), "env-override-test", script);
    let bin = temp.path().join("env-override-test");

    let _guard = EnvGuard::set(&lock, "NTS_VALUE", "parent");
    let options = cmd::CmdOptions::new()
        .with_env_remove("NTS_VALUE")
        .with_env("NTS_VALUE", "child");
    let output = cmd::run_with(&bin, &[], &options);

    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_text(), "child");
}

/// `sibling_or_skip` is the only cross-crate surface of the sibling contract, so
/// it is the only part exercised from outside the crate. The classification it
/// wraps is crate-private and unit-tested next to the code.
///
/// The require flag is dropped explicitly because CI sets it for the whole job,
/// and this case is specifically about the behaviour when it is absent.
#[test]
fn sibling_or_skip_yields_none_for_an_absent_sibling_without_panicking() {
    let lock = GlobalStateLock::new();
    let _hyphen = EnvGuard::remove(&lock, "CARGO_BIN_EXE_nts-absent-sibling");
    let _underscore = EnvGuard::remove(&lock, "CARGO_BIN_EXE_nts_absent_sibling");
    let _require = EnvGuard::remove(&lock, "NILS_TEST_REQUIRE_SIBLING_BINS");

    assert_eq!(
        bin::sibling_or_skip("nts-absent-sibling", "nils-absent"),
        None
    );
}

/// The require flag exists so a lane that should have built every binary cannot
/// report green-but-empty when resolution regresses.
#[test]
#[should_panic(expected = "is not built for this run")]
fn sibling_or_skip_panics_for_an_absent_sibling_when_the_require_flag_is_set() {
    let lock = GlobalStateLock::new();
    let _hyphen = EnvGuard::remove(&lock, "CARGO_BIN_EXE_nts-absent-sibling");
    let _underscore = EnvGuard::remove(&lock, "CARGO_BIN_EXE_nts_absent_sibling");
    let _require = EnvGuard::set(&lock, "NILS_TEST_REQUIRE_SIBLING_BINS", "1");

    bin::sibling_or_skip("nts-absent-sibling", "nils-absent");
}

/// A selected-but-wrong artifact is an operator error, so it must fail rather
/// than skip: skipping would leave the suite green while it never ran against
/// the binary it names.
#[cfg(unix)]
#[test]
#[should_panic(expected = "from an earlier release")]
fn sibling_or_skip_panics_for_a_stale_sibling_instead_of_skipping() {
    let lock = GlobalStateLock::new();
    let temp = TempDir::new().expect("tempdir");
    let script = "#!/bin/sh\nprintf '%s\\n' 'nts-stale-skip 1.0.0 (v1.0.0, rustc 1.0.0)'\n";
    write_exe(temp.path(), "nts-stale-skip", script);
    let path = temp.path().join("nts-stale-skip");
    let _guard = EnvGuard::set(
        &lock,
        "CARGO_BIN_EXE_nts-stale-skip",
        path.to_str().expect("path"),
    );

    bin::sibling_or_skip("nts-stale-skip", "nils-stale");
}

/// Write and execute on the same thread while sibling threads spawn unrelated
/// processes. No thread rewrites the fixture during its exec: only a sibling
/// fork inheriting the writer descriptor can keep the inode text-busy.
#[cfg(target_os = "linux")]
#[test]
fn fixture_exec_retries_while_sibling_threads_spawn() {
    use std::io::ErrorKind;
    use std::process::Command;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    };
    use std::thread;

    let temp = TempDir::new().expect("fixture directory");
    let path = temp.path().join("tool");
    let running = Arc::new(AtomicBool::new(true));
    let barrier = Arc::new(Barrier::new(9));
    let siblings: Vec<_> = (0..8)
        .map(|_| {
            let running = Arc::clone(&running);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let mut spawns = 0;
                while running.load(Ordering::Relaxed) {
                    assert!(
                        Command::new("/bin/true")
                            .status()
                            .expect("sibling spawn")
                            .success()
                    );
                    spawns += 1;
                }
                spawns
            })
        })
        .collect();
    barrier.wait();

    let mut busy_attempts = 0;
    let mut failures = 0;
    for _ in 0..1000 {
        nils_test_support::fs::write_executable(&path, "#!/bin/sh\nprintf 'warm'\n");
        let result = cmd::retry_executable_file_busy(|| {
            let result = Command::new(&path).output();
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == ErrorKind::ExecutableFileBusy)
            {
                busy_attempts += 1;
            }
            result
        });
        match result {
            Ok(output) if output.status.success() && output.stdout == b"warm" => {}
            _ => failures += 1,
        }
    }
    running.store(false, Ordering::Relaxed);
    let spawns: usize = siblings
        .into_iter()
        .map(|sibling| sibling.join().expect("sibling thread"))
        .sum();
    eprintln!(
        "1000 write-then-exec iterations; {spawns} sibling spawns; {busy_attempts} busy attempts; {failures} failures"
    );
    assert!(spawns > 0, "sibling spawns exercised");
    assert_eq!(failures, 0, "fixture exec succeeds despite sibling forks");
}

#[test]
fn fixture_exec_returns_other_errors_without_retrying() {
    let temp = TempDir::new().expect("fixture directory");
    let missing = temp.path().join("missing");
    let mut attempts = 0;
    let error = cmd::retry_executable_file_busy(|| {
        attempts += 1;
        std::process::Command::new(&missing).output()
    })
    .expect_err("missing program fails");
    assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    assert_eq!(attempts, 1);
}

#[cfg(unix)]
#[test]
fn fixture_exec_preserves_nonzero_exit_without_retrying() {
    let mut attempts = 0;
    let output = cmd::retry_executable_file_busy(|| {
        attempts += 1;
        std::process::Command::new("/bin/sh")
            .args(["-c", "printf output; printf error >&2; exit 7"])
            .output()
    })
    .expect("fixture runs");
    assert_eq!(output.status.code(), Some(7));
    assert_eq!(output.stdout, b"output");
    assert_eq!(output.stderr, b"error");
    assert_eq!(attempts, 1);
}

#[cfg(target_os = "linux")]
#[test]
fn fixture_exec_bounds_persistent_executable_file_busy() {
    let temp = TempDir::new().expect("fixture directory");
    let path = temp.path().join("tool");
    nils_test_support::fs::write_executable(&path, "#!/bin/sh\nexit 0\n");
    let _writer = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("hold writer");
    let started = std::time::Instant::now();
    let error = cmd::retry_executable_file_busy(|| std::process::Command::new(&path).output())
        .expect_err("persistent busy error reaches the caller");
    assert_eq!(error.kind(), std::io::ErrorKind::ExecutableFileBusy);
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
}

use nils_test_support::{bin, cmd, write_exe, EnvGuard, GlobalStateLock};
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

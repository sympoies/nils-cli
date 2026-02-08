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

#[cfg(unix)]
#[test]
fn with_path_prepend_places_directory_before_existing_path() {
    let temp = TempDir::new().expect("tempdir");
    let first_dir = temp.path().join("first");
    let second_dir = temp.path().join("second");
    std::fs::create_dir_all(&first_dir).expect("create first dir");
    std::fs::create_dir_all(&second_dir).expect("create second dir");

    write_exe(
        &first_dir,
        "path-pick",
        r#"#!/bin/sh
printf "first"
"#,
    );
    write_exe(
        &second_dir,
        "path-pick",
        r#"#!/bin/sh
printf "second"
"#,
    );
    write_exe(
        temp.path(),
        "path-runner",
        r#"#!/bin/sh
path-pick
"#,
    );

    let bin = temp.path().join("path-runner");
    let base_path = second_dir.to_string_lossy().to_string();
    let base_options = cmd::CmdOptions::new().with_env("PATH", &base_path);
    let base = cmd::run_with(&bin, &[], &base_options);
    assert_eq!(base.code, 0);
    assert_eq!(base.stdout_text(), "second");

    let prefixed_options = base_options.with_path_prepend(&first_dir);
    let prefixed = cmd::run_with(&bin, &[], &prefixed_options);
    assert_eq!(prefixed.code, 0);
    assert_eq!(prefixed.stdout_text(), "first");
}

#[cfg(unix)]
#[test]
fn run_in_dir_with_overrides_options_cwd() {
    let temp = TempDir::new().expect("tempdir");
    let dir_arg = temp.path().join("arg-dir");
    let option_dir = temp.path().join("option-dir");
    std::fs::create_dir_all(&dir_arg).expect("create arg dir");
    std::fs::create_dir_all(&option_dir).expect("create option dir");

    write_exe(
        temp.path(),
        "pwd-override-test",
        r#"#!/bin/sh
pwd
"#,
    );
    let bin = temp.path().join("pwd-override-test");
    let options = cmd::CmdOptions::new().with_cwd(&option_dir);

    let output = cmd::run_in_dir_with(&dir_arg, &bin, &[], &options);
    let stdout = output.stdout_text();
    let stdout = stdout.trim_end();
    let expected = std::fs::canonicalize(&dir_arg).expect("canonical");
    let expected = expected.to_string_lossy();

    assert_eq!(output.code, 0);
    assert_eq!(stdout, expected);
}

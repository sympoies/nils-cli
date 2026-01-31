use std::path::PathBuf;
use std::process::{Command, Stdio};

use pretty_assertions::{assert_eq, assert_ne};

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn api_test_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-test").or_else(|_| std::env::var("CARGO_BIN_EXE_api_test"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-test");
    if bin.exists() {
        return bin;
    }

    panic!("api-test binary path: NotPresent");
}

fn run_api_test(args: &[&str]) -> CmdOutput {
    let output = Command::new(api_test_bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run api-test");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn help_includes_key_flags() {
    let out = run_api_test(&["--help"]);
    assert_eq!(out.code, 0);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("summary"));
    assert!(text.contains("--suite"));
    assert!(text.contains("--suite-file"));
}

#[test]
fn invalid_flag_exits_nonzero() {
    let out = run_api_test(&["--definitely-not-a-flag"]);
    assert_ne!(out.code, 0);
}

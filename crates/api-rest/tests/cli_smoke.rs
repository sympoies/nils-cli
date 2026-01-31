use std::path::PathBuf;
use std::process::{Command, Stdio};

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn api_rest_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-rest").or_else(|_| std::env::var("CARGO_BIN_EXE_api_rest"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-rest");
    if bin.exists() {
        return bin;
    }

    panic!("api-rest binary path: NotPresent");
}

fn run_api_rest(args: &[&str]) -> CmdOutput {
    let output = Command::new(api_rest_bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run api-rest");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn help_includes_key_flags() {
    let out = run_api_rest(&["--help"]);
    assert_eq!(out.code, 0);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("history"));
    assert!(text.contains("--config-dir"));
}

#[test]
fn invalid_flag_exits_nonzero() {
    let out = run_api_rest(&["--definitely-not-a-flag"]);
    assert_ne!(out.code, 0);
}

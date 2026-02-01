use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use pretty_assertions::assert_eq;

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

fn run_api_rest_with_stdin(args: &[&str], stdin: &[u8]) -> CmdOutput {
    let mut child = Command::new(api_rest_bin())
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn api-rest");

    let mut child_stdin = child.stdin.take().expect("stdin");
    child_stdin.write_all(stdin).expect("write stdin");
    drop(child_stdin);

    let output = child.wait_with_output().expect("wait api-rest");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

const SNIPPET: &str = "api-rest call --config-dir setup/rest --env staging --token service setup/rest/requests/health.request.json | jq .";

#[test]
fn report_from_cmd_dry_run_uses_positional_snippet() {
    let out = run_api_rest(&["report-from-cmd", "--dry-run", SNIPPET]);
    assert_eq!(out.code, 0);

    assert!(out.stdout.starts_with("api-rest report"));
    assert!(out
        .stdout
        .contains("--case 'health (staging, token:service)'"));
    assert!(out
        .stdout
        .contains("--request 'setup/rest/requests/health.request.json'"));
    assert!(out.stdout.contains("--config-dir 'setup/rest'"));
    assert!(out.stdout.contains("--env 'staging'"));
    assert!(out.stdout.contains("--token 'service'"));
    assert!(out.stdout.contains(" --run"));
}

#[test]
fn report_from_cmd_dry_run_uses_stdin_snippet() {
    let out = run_api_rest_with_stdin(
        &["report-from-cmd", "--dry-run", "--stdin"],
        format!("{SNIPPET}\n").as_bytes(),
    );
    assert_eq!(out.code, 0);

    assert!(out.stdout.starts_with("api-rest report"));
    assert!(out
        .stdout
        .contains("--case 'health (staging, token:service)'"));
    assert!(out
        .stdout
        .contains("--request 'setup/rest/requests/health.request.json'"));
}

#[test]
fn report_from_cmd_response_stdin_conflicts_with_snippet_stdin() {
    let out = run_api_rest(&["report-from-cmd", "--response", "-", "--stdin", "--dry-run"]);
    assert_eq!(out.code, 1);
    assert!(out
        .stderr
        .contains("When using --response -, stdin is reserved"));
}

#[test]
fn report_from_cmd_dry_run_includes_response_dash_and_omits_run() {
    let out = run_api_rest(&["report-from-cmd", "--dry-run", "--response", "-", SNIPPET]);
    assert_eq!(out.code, 0);

    assert!(out.stdout.starts_with("api-rest report"));
    assert!(out.stdout.contains("--response '-'"));
    assert!(!out.stdout.contains(" --run"));
}

use std::path::PathBuf;
use std::process::{Command, Stdio};

use pretty_assertions::{assert_eq, assert_ne};

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn api_gql_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_api-gql").or_else(|_| std::env::var("CARGO_BIN_EXE_api_gql"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("api-gql");
    if bin.exists() {
        return bin;
    }

    panic!("api-gql binary path: NotPresent");
}

fn run_api_gql(args: &[&str]) -> CmdOutput {
    let output = Command::new(api_gql_bin())
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run api-gql");

    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[test]
fn help_includes_key_flags() {
    let out = run_api_gql(&["--help"]);
    assert_eq!(out.code, 0);
    let text = format!("{}{}", out.stdout, out.stderr);
    assert!(text.contains("schema"));
    assert!(text.contains("report-from-cmd"));
    assert!(text.contains("--config-dir"));
    assert!(text.contains("--list-envs"));
}

#[test]
fn invalid_flag_exits_nonzero() {
    let out = run_api_gql(&["--definitely-not-a-flag"]);
    assert_ne!(out.code, 0);
}

#[test]
fn report_from_cmd_dry_run_exits_zero_and_prints_report_command() {
    let snippet = "api-gql call --env staging setup/graphql/operations/health.graphql";
    let out = run_api_gql(&["report-from-cmd", "--dry-run", snippet]);
    assert_eq!(out.code, 0);
    assert!(out.stdout.contains("api-gql report"));
    assert!(out.stdout.contains("--case"));
    assert!(out.stdout.contains("health"));
    assert!(out.stdout.contains("staging"));
}

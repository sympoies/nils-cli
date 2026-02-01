use std::path::PathBuf;
use std::process::Command;

fn cli_template_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_cli-template")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_cli_template"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("cli-template");
    if bin.exists() {
        return bin;
    }

    panic!("cli-template binary path: NotPresent");
}

#[test]
fn cli_template_runs_without_subcommand() {
    let output = Command::new(cli_template_bin())
        .output()
        .expect("run cli-template");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
}

#[test]
fn cli_template_hello_defaults_to_world() {
    let output = Command::new(cli_template_bin())
        .args(["hello"])
        .output()
        .expect("run hello");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Hello, world!"), "stdout={stdout}");
}

#[test]
fn cli_template_hello_accepts_name() {
    let output = Command::new(cli_template_bin())
        .args(["hello", "Nils"])
        .output()
        .expect("run hello");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Hello, Nils!"), "stdout={stdout}");
}

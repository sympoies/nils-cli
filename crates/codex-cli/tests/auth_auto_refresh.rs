use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn codex_cli_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_codex-cli")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_codex_cli"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("codex-cli");
    if bin.exists() {
        return bin;
    }

    panic!("codex-cli binary path: NotPresent");
}

fn run(args: &[&str], envs: &[(&str, &str)], path_envs: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    for (key, path) in path_envs {
        cmd.env(key, path);
    }
    cmd.output().expect("run codex-cli")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(output.status.code(), Some(code));
}

#[test]
fn auth_auto_refresh_invalid_min_days() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("auth.json");
    fs::write(&auth_file, r#"{"last_refresh":"2025-01-20T12:34:56Z"}"#).expect("write auth");

    let output = run(
        &["auth", "auto-refresh"],
        &[("CODEX_AUTO_REFRESH_MIN_DAYS", "oops")],
        &[("CODEX_AUTH_FILE", &auth_file)],
    );

    assert_exit(&output, 64);
    assert!(stderr(&output).contains("invalid CODEX_AUTO_REFRESH_MIN_DAYS"));
}

#[test]
fn auth_auto_refresh_backfills_timestamp() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("auth.json");
    let cache = dir.path().join("cache");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&cache).expect("cache dir");
    fs::create_dir_all(&secrets).expect("secrets dir");
    let last_refresh = "2025-01-20T12:34:56Z";
    fs::write(&auth_file, format!(r#"{{"last_refresh":"{}"}}"#, last_refresh))
        .expect("write auth");

    let output = run(
        &["auth", "auto-refresh"],
        &[("CODEX_AUTO_REFRESH_MIN_DAYS", "9999")],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_CACHE_DIR", &cache),
            ("CODEX_SECRET_DIR", &secrets),
        ],
    );

    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("refreshed=0 skipped=1 failed=0 (min_age_days=9999)"));

    let timestamp = cache.join("auth.json.timestamp");
    assert_eq!(fs::read_to_string(&timestamp).unwrap(), last_refresh);
}

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";
const PAYLOAD_BETA: &str = "eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ";

fn token(payload: &str) -> String {
    format!("{HEADER}.{payload}.sig")
}

fn auth_json(payload: &str, account_id: &str, refresh_token: &str, last_refresh: &str) -> String {
    format!(
        r#"{{"tokens":{{"access_token":"{}","id_token":"{}","refresh_token":"{}","account_id":"{}"}},"last_refresh":"{}"}}"#,
        token(payload),
        token(payload),
        refresh_token,
        account_id,
        last_refresh
    )
}

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

fn run(args: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut cmd = Command::new(codex_cli_bin());
    cmd.args(args);
    for (key, path) in envs {
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
fn auth_current_exact_match() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let auth_file = dir.path().join("auth.json");
    let content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_a", "2025-01-20T12:34:56Z");
    fs::write(&auth_file, &content).expect("write auth");

    let secret_file = secrets.join("alpha.json");
    fs::write(&secret_file, &content).expect("write secret");

    let output = run(
        &["auth", "current"],
        &[("CODEX_AUTH_FILE", &auth_file), ("CODEX_SECRET_DIR", &secrets)],
    );

    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("matches alpha.json"));
    assert!(!out.contains("identity; secret differs"));
}

#[test]
fn auth_current_identity_differs() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let auth_file = dir.path().join("auth.json");
    let auth_content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_a", "2025-01-20T12:34:56Z");
    fs::write(&auth_file, &auth_content).expect("write auth");

    let secret_file = secrets.join("alpha.json");
    let secret_content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_b", "2025-01-21T12:34:56Z");
    fs::write(&secret_file, &secret_content).expect("write secret");

    let output = run(
        &["auth", "current"],
        &[("CODEX_AUTH_FILE", &auth_file), ("CODEX_SECRET_DIR", &secrets)],
    );

    assert_exit(&output, 0);
    let out = stdout(&output);
    assert!(out.contains("matches alpha.json"));
    assert!(out.contains("identity; secret differs"));
}

#[test]
fn auth_current_no_match() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let auth_file = dir.path().join("auth.json");
    let auth_content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_a", "2025-01-20T12:34:56Z");
    fs::write(&auth_file, &auth_content).expect("write auth");

    let secret_file = secrets.join("beta.json");
    let secret_content = auth_json(PAYLOAD_BETA, "acct_002", "refresh_b", "2025-01-21T12:34:56Z");
    fs::write(&secret_file, &secret_content).expect("write secret");

    let output = run(
        &["auth", "current"],
        &[("CODEX_AUTH_FILE", &auth_file), ("CODEX_SECRET_DIR", &secrets)],
    );

    assert_exit(&output, 2);
    let out = stdout(&output);
    assert!(out.contains("does not match any known secret"));
}

#[test]
fn auth_current_missing_auth_file() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let auth_file = dir.path().join("missing.json");
    let output = run(
        &["auth", "current"],
        &[("CODEX_AUTH_FILE", &auth_file), ("CODEX_SECRET_DIR", &secrets)],
    );

    assert_exit(&output, 1);
    let err = stderr(&output);
    assert!(err.contains("not found"));
}

#[test]
fn auth_sync_updates_matching() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    let cache = dir.path().join("cache");
    fs::create_dir_all(&secrets).expect("secrets dir");
    fs::create_dir_all(&cache).expect("cache dir");

    let auth_file = dir.path().join("auth.json");
    let auth_content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_a", "2025-01-20T12:34:56Z");
    fs::write(&auth_file, &auth_content).expect("write auth");

    let match_secret = secrets.join("alpha.json");
    let match_content = auth_json(PAYLOAD_ALPHA, "acct_001", "refresh_b", "2025-01-21T12:34:56Z");
    fs::write(&match_secret, &match_content).expect("write matching secret");

    let other_secret = secrets.join("beta.json");
    let other_content = auth_json(PAYLOAD_BETA, "acct_002", "refresh_c", "2025-01-22T12:34:56Z");
    fs::write(&other_secret, &other_content).expect("write other secret");

    let output = run(
        &["auth", "sync"],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_DIR", &secrets),
            ("CODEX_SECRET_CACHE_DIR", &cache),
        ],
    );

    assert_exit(&output, 0);

    let synced = fs::read_to_string(&match_secret).expect("read synced secret");
    assert_eq!(synced, auth_content);

    let untouched = fs::read_to_string(&other_secret).expect("read other secret");
    assert_eq!(untouched, other_content);

    let match_timestamp = cache.join("alpha.json.timestamp");
    let auth_timestamp = cache.join("auth.json.timestamp");
    assert_eq!(fs::read_to_string(&match_timestamp).unwrap(), "2025-01-20T12:34:56Z");
    assert_eq!(fs::read_to_string(&auth_timestamp).unwrap(), "2025-01-20T12:34:56Z");
}

use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions, CmdOutput};
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";

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
    bin::resolve("codex-cli")
}

fn run(args: &[&str], envs: &[(&str, &Path)]) -> CmdOutput {
    let mut options = CmdOptions::default();
    for (key, path) in envs {
        let value = path.to_string_lossy();
        options = options.with_env(key, value.as_ref());
    }
    let bin = codex_cli_bin();
    cmd::run_with(&bin, args, &options)
}

fn stdout(output: &CmdOutput) -> String {
    output.stdout_text()
}

#[test]
fn auth_json_contract_current_success_includes_stable_fields() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");

    let auth_file = dir.path().join("auth.json");
    let content = auth_json(
        PAYLOAD_ALPHA,
        "acct_001",
        "refresh_a",
        "2025-01-20T12:34:56Z",
    );
    fs::write(&auth_file, &content).expect("write auth");
    fs::write(secrets.join("alpha.json"), &content).expect("write secret");

    let output = run(
        &["auth", "current", "--json"],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_DIR", &secrets),
        ],
    );
    assert_eq!(output.code, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.auth.v1");
    assert_eq!(payload["command"], "auth current");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["matched"], true);
    assert_eq!(payload["result"]["matched_secret"], "alpha.json");
}

#[test]
fn auth_json_contract_use_ambiguous_returns_structured_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let secrets = dir.path().join("secrets");
    fs::create_dir_all(&secrets).expect("secrets dir");
    let auth_file = dir.path().join("auth.json");
    fs::write(&auth_file, "{}").expect("write auth");

    let content = auth_json(
        PAYLOAD_ALPHA,
        "acct_001",
        "refresh_a",
        "2025-01-20T12:34:56Z",
    );
    fs::write(secrets.join("alpha.json"), &content).expect("write secret");
    fs::write(secrets.join("alpha-dup.json"), &content).expect("write secret");

    let output = run(
        &["auth", "use", "--json", "alpha@example.com"],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_DIR", &secrets),
        ],
    );
    assert_eq!(output.code, 2);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.auth.v1");
    assert_eq!(payload["command"], "auth use");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "ambiguous-secret");
    assert!(payload["error"]["details"]["candidates"].is_array());
}

#[test]
fn auth_json_contract_auto_refresh_unconfigured_returns_zeroed_result() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let auth_file = dir.path().join("missing_auth.json");
    let secret_dir = dir.path().join("secrets");
    fs::create_dir_all(&secret_dir).expect("secret dir");

    let output = run(
        &["auth", "auto-refresh", "--json"],
        &[
            ("CODEX_AUTH_FILE", &auth_file),
            ("CODEX_SECRET_DIR", &secret_dir),
        ],
    );
    assert_eq!(output.code, 0);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.auth.v1");
    assert_eq!(payload["command"], "auth auto-refresh");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["result"]["refreshed"], 0);
    assert_eq!(payload["result"]["failed"], 0);
}

#[test]
fn auth_json_contract_refresh_invalid_name_is_structured() {
    let output = run(&["auth", "refresh", "--json", "../bad.json"], &[]);
    assert_eq!(output.code, 64);
    let payload: Value = serde_json::from_str(&stdout(&output)).expect("json");
    assert_eq!(payload["schema_version"], "codex-cli.auth.v1");
    assert_eq!(payload["command"], "auth refresh");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["code"], "invalid-secret-file-name");
}

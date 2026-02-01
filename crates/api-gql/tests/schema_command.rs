use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[derive(Debug)]
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

fn run_api_gql(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(api_gql_bin());
    cmd.current_dir(cwd)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_remove("GQL_SCHEMA_FILE");

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run api-gql");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

fn write_str(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, contents).expect("write");
}

#[test]
fn schema_prints_resolved_path_when_fallback_file_exists() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("schema.graphql"),
        "type Query { ok: Boolean }\n",
    );

    let out = run_api_gql(root, &["schema", "--config-dir", "setup/graphql"], &[]);
    assert_eq!(out.code, 0, "stderr={}", out.stderr);

    let printed = out.stdout.trim();
    assert!(!printed.is_empty());
    let p = PathBuf::from(printed);
    assert!(p.ends_with("schema.graphql"), "stdout={}", out.stdout);
}

#[test]
fn schema_cat_prints_file_contents() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("schema.graphql"),
        "type Query { ok: Boolean }\n",
    );

    let out = run_api_gql(
        root,
        &["schema", "--config-dir", "setup/graphql", "--cat"],
        &[],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    assert!(out.stdout.contains("type Query"));
}

#[test]
fn schema_errors_when_not_configured() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    std::fs::create_dir_all(root.join("setup/graphql")).expect("mkdir setup");

    let out = run_api_gql(root, &["schema", "--config-dir", "setup/graphql"], &[]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Schema file not configured"));
    assert!(out.stderr.contains("schema.env"));
}

#[test]
fn schema_errors_when_schema_env_points_to_missing_file() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("schema.env"),
        "GQL_SCHEMA_FILE=missing.graphql\n",
    );

    let out = run_api_gql(root, &["schema", "--config-dir", "setup/graphql"], &[]);
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("Schema file not found:"));
    assert!(out.stderr.contains("missing.graphql"));
}

#[test]
fn schema_file_flag_overrides_env_and_schema_env() {
    let tmp = TempDir::new().expect("tmp");
    let root = tmp.path();
    let setup_dir = root.join("setup/graphql");
    std::fs::create_dir_all(&setup_dir).expect("mkdir setup");

    write_str(
        &setup_dir.join("schema.env"),
        "GQL_SCHEMA_FILE=missing.graphql\n",
    );
    write_str(
        &setup_dir.join("real.graphql"),
        "type Query { ok: Boolean }\n",
    );

    let out = run_api_gql(
        root,
        &[
            "schema",
            "--config-dir",
            "setup/graphql",
            "--file",
            "real.graphql",
        ],
        &[("GQL_SCHEMA_FILE", "also-missing.graphql")],
    );
    assert_eq!(out.code, 0, "stderr={}", out.stderr);
    let printed = out.stdout.trim();
    let p = PathBuf::from(printed);
    assert!(p.ends_with("real.graphql"), "stdout={}", out.stdout);
}

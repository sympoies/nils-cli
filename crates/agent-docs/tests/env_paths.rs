use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

struct CmdOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn write_markdown(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent dirs");
    }
    fs::write(path, "# fixture\n").expect("write fixture markdown");
}

fn run_agent_docs(cwd: &Path, args: &[&str], envs: &[(&str, &Path)], unset: &[&str]) -> CmdOutput {
    let mut command = Command::new(agent_docs_bin());
    command.current_dir(cwd).args(args);

    for key in unset {
        command.env_remove(key);
    }

    for (key, value) in envs {
        command.env(key, value);
    }

    let output = command.output().expect("run agent-docs");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn parse_json_stdout(output: &CmdOutput) -> Value {
    assert_eq!(output.code, 0, "stderr: {}", output.stderr);
    serde_json::from_str(&output.stdout).expect("parse json output")
}

fn canonical_string(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

fn json_path(value: &Value, key: &str) -> PathBuf {
    let raw = value[key]
        .as_str()
        .unwrap_or_else(|| panic!("json[{key}] should be string"));
    PathBuf::from(raw)
}

#[test]
fn resolve_uses_env_overrides_for_codex_home_and_project_path() {
    let home = TempDir::new().expect("create home");
    let project = TempDir::new().expect("create project");
    let cwd = TempDir::new().expect("create cwd");

    write_markdown(&home.path().join("DEVELOPMENT.md"));
    write_markdown(&project.path().join("DEVELOPMENT.md"));

    let output = run_agent_docs(
        cwd.path(),
        &["resolve", "--context", "project-dev", "--format", "json"],
        &[
            ("CODEX_HOME", home.path()),
            ("PROJECT_PATH", project.path()),
        ],
        &[],
    );

    let json = parse_json_stdout(&output);
    assert_eq!(
        canonical_string(&json_path(&json, "codex_home")),
        canonical_string(home.path())
    );
    assert_eq!(
        canonical_string(&json_path(&json, "project_path")),
        canonical_string(project.path())
    );
}

#[test]
fn resolve_falls_back_to_git_toplevel_when_project_path_not_set() {
    let home = TempDir::new().expect("create home");
    let repo = TempDir::new().expect("create repo");
    let nested = repo.path().join("nested/work");
    fs::create_dir_all(&nested).expect("create nested directory");

    let git_init = Command::new("git")
        .current_dir(repo.path())
        .args(["init"]) // NOPMD
        .output()
        .expect("run git init");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    write_markdown(&home.path().join("DEVELOPMENT.md"));
    write_markdown(&repo.path().join("DEVELOPMENT.md"));

    let output = run_agent_docs(
        &nested,
        &["resolve", "--context", "project-dev", "--format", "json"],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );

    let json = parse_json_stdout(&output);
    assert_eq!(
        canonical_string(&json_path(&json, "project_path")),
        canonical_string(repo.path())
    );
}

#[test]
fn resolve_falls_back_to_cwd_when_not_git_repo_and_no_project_path() {
    let home = TempDir::new().expect("create home");
    let cwd = TempDir::new().expect("create cwd");

    write_markdown(&home.path().join("DEVELOPMENT.md"));
    write_markdown(&cwd.path().join("DEVELOPMENT.md"));

    let output = run_agent_docs(
        cwd.path(),
        &["resolve", "--context", "project-dev", "--format", "json"],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );

    let json = parse_json_stdout(&output);
    assert_eq!(
        canonical_string(&json_path(&json, "project_path")),
        canonical_string(cwd.path())
    );
}

fn agent_docs_bin() -> PathBuf {
    for env_name in ["CARGO_BIN_EXE_agent-docs", "CARGO_BIN_EXE_agent_docs"] {
        if let Some(path) = std::env::var_os(env_name) {
            return PathBuf::from(path);
        }
    }

    let current = std::env::current_exe().expect("current exe");
    let Some(target_profile_dir) = current.parent().and_then(|path| path.parent()) else {
        panic!("failed to resolve target profile dir");
    };

    let candidate = target_profile_dir.join(format!("agent-docs{}", std::env::consts::EXE_SUFFIX));
    if candidate.exists() {
        return candidate;
    }

    panic!("agent-docs binary path not found: {}", candidate.display());
}

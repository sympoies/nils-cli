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

fn json_optional_path(value: &Value, key: &str) -> Option<PathBuf> {
    value[key].as_str().map(PathBuf::from)
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
fn resolve_detects_linked_worktree_metadata_when_project_path_not_set() {
    let home = TempDir::new().expect("create home");
    let workspace = TempDir::new().expect("create workspace");
    let repo = workspace.path().join("repo");
    let linked_worktree = workspace.path().join("linked");
    fs::create_dir_all(&repo).expect("create repo directory");

    let git_init = Command::new("git")
        .current_dir(&repo)
        .args(["init"]) // NOPMD
        .output()
        .expect("run git init");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    let git_user_name = Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.name", "Agent Docs Tests"])
        .output()
        .expect("set git user.name");
    assert!(
        git_user_name.status.success(),
        "git config user.name failed: {}",
        String::from_utf8_lossy(&git_user_name.stderr)
    );

    let git_user_email = Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.email", "agent-docs@example.test"])
        .output()
        .expect("set git user.email");
    assert!(
        git_user_email.status.success(),
        "git config user.email failed: {}",
        String::from_utf8_lossy(&git_user_email.stderr)
    );

    fs::write(repo.join("README.md"), "seed\n").expect("write initial commit file");
    let git_add = Command::new("git")
        .current_dir(&repo)
        .args(["add", "."])
        .output()
        .expect("run git add");
    assert!(
        git_add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&git_add.stderr)
    );

    let git_commit = Command::new("git")
        .current_dir(&repo)
        .args(["commit", "-m", "seed"])
        .output()
        .expect("run git commit");
    assert!(
        git_commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&git_commit.stderr)
    );

    let linked_worktree_arg = linked_worktree
        .to_str()
        .expect("linked worktree path should be utf-8");
    let git_worktree_add = Command::new("git")
        .current_dir(&repo)
        .args([
            "worktree",
            "add",
            linked_worktree_arg,
            "-b",
            "linked-worktree",
        ])
        .output()
        .expect("run git worktree add");
    assert!(
        git_worktree_add.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&git_worktree_add.stderr)
    );

    let nested = linked_worktree.join("nested/work");
    fs::create_dir_all(&nested).expect("create nested directory");

    write_markdown(&home.path().join("DEVELOPMENT.md"));
    write_markdown(&linked_worktree.join("DEVELOPMENT.md"));

    let output = run_agent_docs(
        &nested,
        &["resolve", "--context", "project-dev", "--format", "json"],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );

    let json = parse_json_stdout(&output);
    assert_eq!(
        canonical_string(&json_path(&json, "project_path")),
        canonical_string(&linked_worktree)
    );
    assert!(
        json["is_linked_worktree"]
            .as_bool()
            .expect("json[is_linked_worktree] should be bool")
    );
    assert_eq!(
        canonical_string(
            &json_optional_path(&json, "git_common_dir").expect("git_common_dir should be present")
        ),
        canonical_string(&repo.join(".git"))
    );
    assert_eq!(
        canonical_string(
            &json_optional_path(&json, "primary_worktree_path")
                .expect("primary_worktree_path should be present")
        ),
        canonical_string(&repo)
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
    assert!(
        !json["is_linked_worktree"]
            .as_bool()
            .expect("json[is_linked_worktree] should be bool")
    );
    assert!(json["git_common_dir"].is_null());
    assert!(json["primary_worktree_path"].is_null());
}

#[test]
fn cli_help_documents_worktree_mode_values() {
    let cwd = TempDir::new().expect("create cwd");
    let output = run_agent_docs(cwd.path(), &["--help"], &[], &[]);

    assert_eq!(
        output.code, 0,
        "--help should succeed: stderr={}",
        output.stderr
    );
    assert!(
        output.stdout.contains("worktree"),
        "--help should mention worktree fallback mode:\n{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("auto"),
        "--help should include auto mode:\n{}",
        output.stdout
    );
    assert!(
        output.stdout.contains("local-only"),
        "--help should include local-only mode:\n{}",
        output.stdout
    );
}

#[test]
fn resolve_strict_auto_uses_primary_worktree_fallback_but_local_only_keeps_local_strict_behavior() {
    let home = TempDir::new().expect("create home");
    let workspace = TempDir::new().expect("create workspace");
    let repo = workspace.path().join("repo");
    let linked_worktree = workspace.path().join("linked");
    fs::create_dir_all(&repo).expect("create repo directory");

    let git_init = Command::new("git")
        .current_dir(&repo)
        .args(["init"]) // NOPMD
        .output()
        .expect("run git init");
    assert!(
        git_init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&git_init.stderr)
    );

    let git_user_name = Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.name", "Agent Docs Tests"])
        .output()
        .expect("set git user.name");
    assert!(
        git_user_name.status.success(),
        "git config user.name failed: {}",
        String::from_utf8_lossy(&git_user_name.stderr)
    );

    let git_user_email = Command::new("git")
        .current_dir(&repo)
        .args(["config", "user.email", "agent-docs@example.test"])
        .output()
        .expect("set git user.email");
    assert!(
        git_user_email.status.success(),
        "git config user.email failed: {}",
        String::from_utf8_lossy(&git_user_email.stderr)
    );

    fs::write(repo.join("README.md"), "seed\n").expect("write initial commit file");
    let git_add = Command::new("git")
        .current_dir(&repo)
        .args(["add", "."])
        .output()
        .expect("run git add");
    assert!(
        git_add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&git_add.stderr)
    );

    let git_commit = Command::new("git")
        .current_dir(&repo)
        .args(["commit", "-m", "seed"])
        .output()
        .expect("run git commit");
    assert!(
        git_commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&git_commit.stderr)
    );

    let linked_worktree_arg = linked_worktree
        .to_str()
        .expect("linked worktree path should be utf-8");
    let git_worktree_add = Command::new("git")
        .current_dir(&repo)
        .args([
            "worktree",
            "add",
            linked_worktree_arg,
            "-b",
            "linked-worktree",
        ])
        .output()
        .expect("run git worktree add");
    assert!(
        git_worktree_add.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&git_worktree_add.stderr)
    );

    write_markdown(&home.path().join("DEVELOPMENT.md"));
    write_markdown(&repo.join("AGENTS.md"));
    write_markdown(&repo.join("DEVELOPMENT.md"));
    let local_development = linked_worktree.join("DEVELOPMENT.md");
    if local_development.exists() {
        fs::remove_file(&local_development).expect("remove local linked-worktree development");
    }
    let local_agents = linked_worktree.join("AGENTS.md");
    if local_agents.exists() {
        fs::remove_file(&local_agents).expect("remove local linked-worktree agents");
    }

    let auto_output = run_agent_docs(
        &linked_worktree,
        &[
            "resolve",
            "--context",
            "project-dev",
            "--format",
            "checklist",
            "--strict",
        ],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );
    assert_eq!(
        auto_output.code, 0,
        "auto mode should pass when fallback doc exists in primary worktree: stdout=\n{}\nstderr=\n{}",
        auto_output.stdout, auto_output.stderr
    );
    assert!(
        auto_output
            .stdout
            .contains("DEVELOPMENT.md status=present path="),
        "auto mode checklist should mark DEVELOPMENT.md as present:\n{}",
        auto_output.stdout
    );
    assert!(
        auto_output
            .stdout
            .contains(&repo.join("DEVELOPMENT.md").display().to_string()),
        "auto mode should resolve from primary worktree path:\n{}",
        auto_output.stdout
    );

    let local_only_output = run_agent_docs(
        &linked_worktree,
        &[
            "--worktree-fallback",
            "local-only",
            "resolve",
            "--context",
            "project-dev",
            "--format",
            "checklist",
            "--strict",
        ],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );
    assert_eq!(
        local_only_output.code, 1,
        "local-only mode should keep strict local behavior: stdout=\n{}\nstderr=\n{}",
        local_only_output.stdout, local_only_output.stderr
    );
    assert!(
        local_only_output
            .stdout
            .contains("DEVELOPMENT.md status=missing path="),
        "local-only mode checklist should keep DEVELOPMENT.md missing:\n{}",
        local_only_output.stdout
    );
    assert!(
        local_only_output
            .stdout
            .contains(&linked_worktree.join("DEVELOPMENT.md").display().to_string()),
        "local-only mode should report local project path:\n{}",
        local_only_output.stdout
    );

    let baseline_auto_output = run_agent_docs(
        &linked_worktree,
        &[
            "baseline", "--check", "--target", "project", "--strict", "--format", "text",
        ],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );
    assert_eq!(
        baseline_auto_output.code, 0,
        "auto baseline strict should pass with primary-worktree fallback: stdout=\n{}\nstderr=\n{}",
        baseline_auto_output.stdout, baseline_auto_output.stderr
    );

    let baseline_local_only_output = run_agent_docs(
        &linked_worktree,
        &[
            "--worktree-fallback",
            "local-only",
            "baseline",
            "--check",
            "--target",
            "project",
            "--strict",
            "--format",
            "text",
        ],
        &[("CODEX_HOME", home.path())],
        &["PROJECT_PATH"],
    );
    assert_eq!(
        baseline_local_only_output.code, 1,
        "local-only baseline strict should keep local-only failure semantics: stdout=\n{}\nstderr=\n{}",
        baseline_local_only_output.stdout, baseline_local_only_output.stderr
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

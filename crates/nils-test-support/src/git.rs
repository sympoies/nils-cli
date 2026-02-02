use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

#[derive(Debug, Clone)]
pub struct InitRepoOptions {
    pub branch: Option<String>,
    pub initial_commit: bool,
    pub initial_commit_name: String,
    pub initial_commit_contents: String,
    pub initial_commit_message: String,
}

impl InitRepoOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn without_branch(mut self) -> Self {
        self.branch = None;
        self
    }

    pub fn with_initial_commit(mut self) -> Self {
        self.initial_commit = true;
        self
    }
}

impl Default for InitRepoOptions {
    fn default() -> Self {
        Self {
            branch: Some("main".to_string()),
            initial_commit: false,
            initial_commit_name: "README.md".to_string(),
            initial_commit_contents: "init".to_string(),
            initial_commit_message: "init".to_string(),
        }
    }
}

pub fn git_output(dir: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git command failed to spawn")
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = git_output(dir, args);
    if !output.status.success() {
        panic!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub fn git_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> String {
    let output = git_output_with_env(dir, args, envs);
    if !output.status.success() {
        panic!(
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn git_output_with_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    for (key, value) in envs {
        cmd.env(key, value);
    }
    cmd.output().expect("git command failed to spawn")
}

pub fn init_repo_with(options: InitRepoOptions) -> TempDir {
    let dir = TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "-q"]);

    if let Some(branch) = options.branch.as_deref() {
        // Make the initial branch deterministic across environments.
        git(dir.path(), &["checkout", "-q", "-B", branch]);
    }

    git(dir.path(), &["config", "user.email", "test@example.com"]);
    git(dir.path(), &["config", "user.name", "Test User"]);
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    git(dir.path(), &["config", "tag.gpgSign", "false"]);

    if options.initial_commit {
        let file_path = dir.path().join(&options.initial_commit_name);
        fs::write(&file_path, &options.initial_commit_contents).expect("write initial commit");
        git(dir.path(), &["add", &options.initial_commit_name]);
        git(
            dir.path(),
            &["commit", "-m", &options.initial_commit_message],
        );
    }

    dir
}

pub fn commit_file(dir: &Path, name: &str, contents: &str, message: &str) -> String {
    let path = dir.join(name);
    fs::write(&path, contents).expect("write file");
    git(dir, &["add", name]);
    git(dir, &["commit", "-m", message]);
    git(dir, &["rev-parse", "HEAD"]).trim().to_string()
}

pub fn repo_id(dir: &Path) -> String {
    dir.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_string()
}

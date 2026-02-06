#![allow(dead_code)]

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_docs::env::ResolvedRoots;
use agent_docs::model::{Context, OutputFormat};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub struct FixtureWorkspace {
    _temp: TestTempDir,
    pub root: PathBuf,
    pub codex_home: PathBuf,
    pub project_path: PathBuf,
}

impl FixtureWorkspace {
    pub fn from_fixtures() -> Self {
        let temp = TestTempDir::new("agent-docs-resolve-builtin");
        let root = temp.path().to_path_buf();
        let codex_home = root.join("codex-home");
        let project_path = root.join("project");

        copy_fixture_tree(&fixture_path("home"), &codex_home);
        copy_fixture_tree(&fixture_path("project"), &project_path);

        Self {
            _temp: temp,
            root,
            codex_home,
            project_path,
        }
    }

    pub fn roots(&self) -> ResolvedRoots {
        ResolvedRoots {
            codex_home: self.codex_home.clone(),
            project_path: self.project_path.clone(),
        }
    }
}

impl Default for FixtureWorkspace {
    fn default() -> Self {
        Self::from_fixtures()
    }
}

pub fn fixture_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(relative)
}

pub fn remove_file_if_exists(path: &Path) {
    if path.exists() {
        fs::remove_file(path).expect("remove file");
    }
}

pub fn run_resolve_exit_code(
    workspace: &FixtureWorkspace,
    context: Context,
    format: OutputFormat,
    strict: bool,
) -> i32 {
    let mut args: Vec<OsString> = vec![
        OsString::from("agent-docs"),
        OsString::from("resolve"),
        OsString::from("--context"),
        OsString::from(context.as_str()),
        OsString::from("--format"),
        OsString::from(format.as_str()),
        OsString::from("--codex-home"),
        workspace.codex_home.as_os_str().to_owned(),
        OsString::from("--project-path"),
        workspace.project_path.as_os_str().to_owned(),
    ];

    if strict {
        args.push(OsString::from("--strict"));
    }

    agent_docs::run_with_args(args)
}

pub fn required_lines(text: &str) -> Vec<&str> {
    text.lines()
        .filter(|line| line.starts_with("[required]"))
        .collect()
}

#[derive(Debug)]
pub struct CliOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl CliOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

pub fn run_agent_docs_command(workspace: &FixtureWorkspace, args: &[&str]) -> CliOutput {
    let mut command = Command::new(agent_docs_bin_path());
    command
        .arg("--codex-home")
        .arg(&workspace.codex_home)
        .arg("--project-path")
        .arg(&workspace.project_path)
        .args(args);

    let output = command.output().expect("run agent-docs command");
    CliOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

pub fn write_text(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directory");
    }
    fs::write(path, body).expect("write file");
}

fn copy_fixture_tree(source: &Path, destination: &Path) {
    assert!(
        source.is_dir(),
        "fixture source missing: {}",
        source.display()
    );
    fs::create_dir_all(destination).expect("create destination fixture directory");

    let mut entries: Vec<_> = fs::read_dir(source)
        .expect("read fixture directory")
        .map(|entry| entry.expect("fixture entry"))
        .collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type().expect("fixture file type");
        if file_type.is_dir() {
            copy_fixture_tree(&source_path, &destination_path);
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).expect("copy fixture file");
        }
    }
}

struct TestTempDir {
    path: PathBuf,
}

impl TestTempDir {
    fn new(prefix: &str) -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dirname = format!("{prefix}-{}-{timestamp}-{sequence}", std::process::id());
        let path = std::env::temp_dir().join(dirname);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestTempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn agent_docs_bin_path() -> PathBuf {
    for env_name in ["CARGO_BIN_EXE_agent-docs", "CARGO_BIN_EXE_agent_docs"] {
        if let Some(path) = std::env::var_os(env_name) {
            return PathBuf::from(path);
        }
    }

    let current = std::env::current_exe().expect("current test executable");
    let Some(target_profile_dir) = current.parent().and_then(|path| path.parent()) else {
        panic!("failed to resolve target profile directory from current executable");
    };

    let candidate = target_profile_dir.join(format!("agent-docs{}", std::env::consts::EXE_SUFFIX));
    if candidate.exists() {
        return candidate;
    }

    panic!(
        "agent-docs binary path not found via env vars or fallback candidate {}",
        candidate.display()
    );
}

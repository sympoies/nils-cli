use gemini_cli::agent;
use nils_common::process as shared_process;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock")
}

struct EnvGuard {
    key: &'static str,
    old: Option<std::ffi::OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let old = std::env::var_os(key);
        // SAFETY: tests mutate process env with a global lock.
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(value) = self.old.take() {
            // SAFETY: tests mutate process env with a global lock.
            unsafe { std::env::set_var(self.key, value) };
        } else {
            // SAFETY: tests mutate process env with a global lock.
            unsafe { std::env::remove_var(self.key) };
        }
    }
}

struct CwdGuard {
    old: PathBuf,
}

impl CwdGuard {
    fn enter(path: &Path) -> Self {
        let old = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(path).expect("set cwd");
        Self { old }
    }
}

impl Drop for CwdGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.old);
    }
}

fn temp_dir(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "nils-gemini-cli-{label}-{}-{nanos}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temp dir");
    path
}

#[cfg(unix)]
fn write_executable(path: &Path, content: &str) {
    use std::os::unix::fs::PermissionsExt;

    fs::write(path, content).expect("write executable");
    let mut perms = fs::metadata(path).expect("metadata").permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("chmod");
}

fn real_git_path() -> String {
    shared_process::find_in_path("git")
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| panic!("git not found in PATH for tests"))
}

fn write_stub_git(dir: &Path) {
    let git = real_git_path();
    let script = format!(
        r#"#!/bin/sh
exec "{git}" "$@"
"#
    );
    write_executable(&dir.join("git"), &script);
}

fn write_stub_semantic_commit(dir: &Path) {
    write_executable(&dir.join("semantic-commit"), "#!/bin/sh\nexit 0\n");
}

fn write_stub_gemini(dir: &Path) {
    let script = r#"#!/bin/sh
set -eu
out_dir="${GEMINI_STUB_OUT_DIR:?missing GEMINI_STUB_OUT_DIR}"
i=0
for arg in "$@"; do
  printf '%s' "$arg" > "$out_dir/arg-$i"
  i=$((i+1))
done
"#;
    write_executable(&dir.join("gemini"), script);
}

fn init_repo(path: &Path) {
    let git = real_git_path();
    assert!(
        Command::new(&git)
            .current_dir(path)
            .arg("init")
            .status()
            .expect("git init")
            .success()
    );
    assert!(
        Command::new(&git)
            .current_dir(path)
            .args(["config", "user.name", "Test User"])
            .status()
            .expect("git config user.name")
            .success()
    );
    assert!(
        Command::new(&git)
            .current_dir(path)
            .args(["config", "user.email", "test@example.com"])
            .status()
            .expect("git config user.email")
            .success()
    );
}

#[test]
fn agent_commit_returns_1_when_git_missing() {
    let _lock = env_lock();
    let _path = EnvGuard::set("PATH", "");

    let options = agent::commit::CommitOptions {
        push: false,
        auto_stage: false,
        extra: Vec::new(),
    };
    let code = agent::commit::run(&options);
    assert_eq!(code, 1);
}

#[test]
fn agent_commit_semantic_mode_executes_gemini_with_template_and_push_note() {
    let _lock = env_lock();
    let dir = temp_dir("agent-commit-semantic");
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).expect("repo dir");
    init_repo(&repo);
    fs::write(repo.join("a.txt"), "hello").expect("write file");

    let zdotdir = dir.join("zdotdir");
    let prompts_dir = zdotdir.join("prompts");
    fs::create_dir_all(&prompts_dir).expect("prompts dir");
    fs::write(
        prompts_dir.join("semantic-commit-autostage.md"),
        "SEMANTIC_AUTOSTAGE\n",
    )
    .expect("write prompt template");

    let stub_dir = dir.join("bin");
    fs::create_dir_all(&stub_dir).expect("stub dir");
    write_stub_git(&stub_dir);
    write_stub_semantic_commit(&stub_dir);
    write_stub_gemini(&stub_dir);

    let out_dir = dir.join("out");
    fs::create_dir_all(&out_dir).expect("out dir");

    let _path = EnvGuard::set("PATH", stub_dir.as_os_str());
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _model = EnvGuard::set("GEMINI_CLI_MODEL", "m-test");
    let _reasoning = EnvGuard::set("GEMINI_CLI_REASONING", "low");
    let _zdotdir = EnvGuard::set("ZDOTDIR", zdotdir.as_os_str());
    let _out_dir = EnvGuard::set("GEMINI_STUB_OUT_DIR", out_dir.as_os_str());
    let _cwd = CwdGuard::enter(&repo);

    let options = agent::commit::CommitOptions {
        push: true,
        auto_stage: true,
        extra: vec!["extra".to_string(), "words".to_string()],
    };
    let code = agent::commit::run(&options);
    assert_eq!(code, 0);

    let prompt = fs::read_to_string(out_dir.join("arg-0")).expect("prompt");
    assert!(prompt.contains("SEMANTIC_AUTOSTAGE"));
    assert!(prompt.contains("Furthermore, please push the committed changes"));
    assert!(prompt.contains("Additional instructions from user:"));
    assert!(prompt.contains("extra words"));

    let arg2 = fs::read_to_string(out_dir.join("arg-2")).expect("model");
    assert_eq!(arg2, "m-test");

    let _ = fs::remove_dir_all(&dir);
}

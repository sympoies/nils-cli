use gemini_cli::agent;
use std::fs;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
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

fn write_gemini_stub(stub_path: &Path) {
    let script = r#"#!/bin/sh
set -eu
out="${GEMINI_TEST_ARGV_LOG:?missing GEMINI_TEST_ARGV_LOG}"
: > "$out"
for a in "$@"; do
  echo "$a" >> "$out"
done
"#;
    write_executable(stub_path, script);
}

fn read_args(log_path: &Path) -> Vec<String> {
    match fs::read_to_string(log_path) {
        Ok(raw) => raw.lines().map(|line| line.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn agent_prompt_requires_dangerous_mode() {
    let _lock = env_lock();
    let dir = temp_dir("agent-prompt-requires-dangerous");
    let stub = dir.join("gemini");
    let args_log = dir.join("argv.log");
    write_gemini_stub(&stub);

    let _path = EnvGuard::set("PATH", dir.as_os_str());
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "false");
    let _model = EnvGuard::set("GEMINI_CLI_MODEL", "m");
    let _reasoning = EnvGuard::set("GEMINI_CLI_REASONING", "low");
    let _argv = EnvGuard::set("GEMINI_TEST_ARGV_LOG", args_log.as_os_str());

    let mut stdin = BufReader::new(Cursor::new(""));
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let code = agent::prompt_with_io(&["hello".into()], &mut stdin, &mut stdout, &mut stderr);
    assert_eq!(code, 1);
    assert!(
        String::from_utf8_lossy(&stderr)
            .contains("gemini-tools:prompt: disabled (set GEMINI_ALLOW_DANGEROUS_ENABLED=true)")
    );
    assert!(read_args(&args_log).is_empty());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn agent_prompt_execs_gemini_with_expected_args() {
    let _lock = env_lock();
    let dir = temp_dir("agent-prompt-args");
    let stub = dir.join("gemini");
    let args_log = dir.join("argv.log");
    write_gemini_stub(&stub);

    let _path = EnvGuard::set("PATH", dir.as_os_str());
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _model = EnvGuard::set("GEMINI_CLI_MODEL", "m-test");
    let _reasoning = EnvGuard::set("GEMINI_CLI_REASONING", "high");
    let _argv = EnvGuard::set("GEMINI_TEST_ARGV_LOG", args_log.as_os_str());

    let mut stdin = BufReader::new(Cursor::new(""));
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let code = agent::prompt_with_io(
        &["hello".into(), "world".into()],
        &mut stdin,
        &mut stdout,
        &mut stderr,
    );
    assert_eq!(code, 0);
    assert!(stderr.is_empty());

    let args = read_args(&args_log);
    assert_eq!(
        args,
        vec![
            "--prompt=hello world",
            "--model",
            "m-test",
            "--approval-mode",
            "yolo",
        ]
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn agent_prompt_reads_stdin_when_no_args() {
    let _lock = env_lock();
    let dir = temp_dir("agent-prompt-stdin");
    let stub = dir.join("gemini");
    let args_log = dir.join("argv.log");
    write_gemini_stub(&stub);

    let _path = EnvGuard::set("PATH", dir.as_os_str());
    let _danger = EnvGuard::set("GEMINI_ALLOW_DANGEROUS_ENABLED", "true");
    let _model = EnvGuard::set("GEMINI_CLI_MODEL", "m");
    let _reasoning = EnvGuard::set("GEMINI_CLI_REASONING", "medium");
    let _argv = EnvGuard::set("GEMINI_TEST_ARGV_LOG", args_log.as_os_str());

    let mut stdin = BufReader::new(Cursor::new("from stdin\n"));
    let mut stdout: Vec<u8> = Vec::new();
    let mut stderr: Vec<u8> = Vec::new();
    let code = agent::prompt_with_io(&[], &mut stdin, &mut stdout, &mut stderr);
    assert_eq!(code, 0);
    assert_eq!(String::from_utf8_lossy(&stdout), "Prompt: ");
    assert!(stderr.is_empty());

    let args = read_args(&args_log);
    let prompt_flag = args.first().map(String::as_str).unwrap_or_default();
    assert_eq!(prompt_flag, "--prompt=from stdin");

    let _ = fs::remove_dir_all(&dir);
}

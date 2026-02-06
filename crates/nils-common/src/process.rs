use std::ffi::OsString;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output, Stdio};

#[derive(Debug)]
pub struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl ProcessOutput {
    pub fn into_std_output(self) -> Output {
        Output {
            status: self.status,
            stdout: self.stdout,
            stderr: self.stderr,
        }
    }

    pub fn stdout_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stdout).to_string()
    }

    pub fn stderr_lossy(&self) -> String {
        String::from_utf8_lossy(&self.stderr).to_string()
    }

    pub fn stdout_trimmed(&self) -> String {
        self.stdout_lossy().trim().to_string()
    }
}

impl From<Output> for ProcessOutput {
    fn from(output: Output) -> Self {
        Self {
            status: output.status,
            stdout: output.stdout,
            stderr: output.stderr,
        }
    }
}

#[derive(Debug)]
pub enum ProcessError {
    Io(io::Error),
    NonZero(ProcessOutput),
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::NonZero(output) => write!(f, "process exited with status {}", output.status),
        }
    }
}

impl std::error::Error for ProcessError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::NonZero(_) => None,
        }
    }
}

impl From<io::Error> for ProcessError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

pub fn run_output(program: &str, args: &[&str]) -> io::Result<ProcessOutput> {
    Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map(ProcessOutput::from)
}

pub fn run_checked(program: &str, args: &[&str]) -> Result<ProcessOutput, ProcessError> {
    let output = run_output(program, args)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(ProcessError::NonZero(output))
    }
}

pub fn run_stdout(program: &str, args: &[&str]) -> Result<String, ProcessError> {
    let output = run_checked(program, args)?;
    Ok(output.stdout_lossy())
}

pub fn run_stdout_trimmed(program: &str, args: &[&str]) -> Result<String, ProcessError> {
    let output = run_checked(program, args)?;
    Ok(output.stdout_trimmed())
}

pub fn run_status_quiet(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    Command::new(program)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
}

pub fn run_status_inherit(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    Command::new(program)
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}

pub fn cmd_exists(program: &str) -> bool {
    find_in_path(program).is_some()
}

pub fn find_in_path(program: &str) -> Option<PathBuf> {
    if looks_like_path(program) {
        let p = PathBuf::from(program);
        return is_executable_file(&p).then_some(p);
    }

    let path_var: OsString = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(program);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn looks_like_path(program: &str) -> bool {
    // Treat both separators as paths, even on unix. It is harmless and avoids surprises when a
    // caller passes a Windows-style path.
    program.contains('/') || program.contains('\\')
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{prepend_path, GlobalStateLock, StubBinDir};
    use std::fs;

    #[test]
    fn find_in_path_with_explicit_missing_path_returns_none() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("missing");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_non_executable_file_returns_none() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("file");
        fs::write(&path, "data").expect("write file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert!(found.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn find_in_path_with_executable_file_returns_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::TempDir::new().expect("tempdir");
        let path = dir.path().join("exec");
        fs::write(&path, "data").expect("write file");

        let mut perms = fs::metadata(&path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set permissions");

        let found = find_in_path(path.to_string_lossy().as_ref());

        assert_eq!(found, Some(path));
    }

    #[test]
    fn find_in_path_resolves_from_path_env() {
        let lock = GlobalStateLock::new();
        let stub = StubBinDir::new();
        stub.write_exe("hello-stub", "#!/bin/sh\necho hi\n");

        let _path_guard = prepend_path(&lock, stub.path());

        let found = find_in_path("hello-stub").expect("found");
        assert!(found.ends_with("hello-stub"));
    }

    #[cfg(unix)]
    #[test]
    fn run_output_returns_output_for_nonzero_status() {
        let output = run_output("sh", &["-c", "printf 'oops' 1>&2; printf 'out'; exit 2"])
            .expect("run output");

        assert!(!output.status.success());
        assert_eq!(output.stdout_lossy(), "out");
        assert_eq!(output.stderr_lossy(), "oops");
    }

    #[cfg(unix)]
    #[test]
    fn run_checked_returns_nonzero_error_with_captured_output() {
        let err = run_checked("sh", &["-c", "printf 'e' 1>&2; printf 'o'; exit 7"])
            .expect_err("expected nonzero error");

        match err {
            ProcessError::Io(_) => panic!("expected nonzero error"),
            ProcessError::NonZero(output) => {
                assert_eq!(output.stdout_lossy(), "o");
                assert_eq!(output.stderr_lossy(), "e");
                assert!(!output.status.success());
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_stdout_trimmed_trims_trailing_whitespace() {
        let stdout = run_stdout_trimmed("sh", &["-c", "printf ' hello \\n\\n'"]).expect("stdout");

        assert_eq!(stdout, "hello");
    }

    #[cfg(unix)]
    #[test]
    fn run_status_helpers_keep_stdio_contracts() {
        let quiet = run_status_quiet("sh", &["-c", "exit 0"]).expect("quiet status");
        assert!(quiet.success());

        let inherit = run_status_inherit("sh", &["-c", "exit 3"]).expect("inherit status");
        assert_eq!(inherit.code(), Some(3));
    }
}

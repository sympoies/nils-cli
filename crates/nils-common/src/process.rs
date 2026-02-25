use std::ffi::{OsStr, OsString};
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

pub type ProcessEnvPair<'a> = (&'a str, &'a str);

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
    run_output_with(program, args, None, &[])
}

pub fn run_output_in(program: &str, args: &[&str], cwd: &Path) -> io::Result<ProcessOutput> {
    run_output_with(program, args, Some(cwd), &[])
}

pub fn run_output_with(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[ProcessEnvPair<'_>],
) -> io::Result<ProcessOutput> {
    let mut command = command_with(program, args, cwd, env);
    command
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
    run_status_quiet_with(program, args, None, &[])
}

pub fn run_status_quiet_in(program: &str, args: &[&str], cwd: &Path) -> io::Result<ExitStatus> {
    run_status_quiet_with(program, args, Some(cwd), &[])
}

pub fn run_status_quiet_with(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[ProcessEnvPair<'_>],
) -> io::Result<ExitStatus> {
    let mut command = command_with(program, args, cwd, env);
    command.stdout(Stdio::null()).stderr(Stdio::null()).status()
}

pub fn run_status_inherit(program: &str, args: &[&str]) -> io::Result<ExitStatus> {
    run_status_inherit_with(program, args, None, &[])
}

pub fn run_status_inherit_in(program: &str, args: &[&str], cwd: &Path) -> io::Result<ExitStatus> {
    run_status_inherit_with(program, args, Some(cwd), &[])
}

pub fn run_status_inherit_with(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[ProcessEnvPair<'_>],
) -> io::Result<ExitStatus> {
    let mut command = command_with(program, args, cwd, env);
    command
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
}

pub fn cmd_exists(program: &str) -> bool {
    find_in_path(program).is_some()
}

pub fn browser_open_command() -> Option<&'static str> {
    if cmd_exists("open") {
        Some("open")
    } else if cmd_exists("xdg-open") {
        Some("xdg-open")
    } else {
        None
    }
}

pub fn is_headless_browser_launch_failure(stdout: &[u8], stderr: &[u8]) -> bool {
    let mut message = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if !stdout.is_empty() {
        message.push('\n');
        message.push_str(&String::from_utf8_lossy(stdout).to_ascii_lowercase());
    }

    if message.contains("no method available for opening")
        || message.contains("couldn't find a suitable web browser")
    {
        return true;
    }

    message.contains("not found")
        && ["www-browser", "links2", "elinks", "links", "lynx", "w3m"]
            .iter()
            .any(|candidate| message.contains(candidate))
}

pub fn find_in_path(program: &str) -> Option<PathBuf> {
    if looks_like_path(program) {
        let p = PathBuf::from(program);
        return is_executable_file(&p).then_some(p);
    }

    let path_var: OsString = std::env::var_os("PATH")?;
    let windows_extensions = if cfg!(windows) {
        Some(windows_pathext_extensions())
    } else {
        None
    };

    for dir in std::env::split_paths(&path_var) {
        for candidate in path_lookup_candidates(&dir, program, windows_extensions.as_deref()) {
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

fn path_lookup_candidates(
    dir: &Path,
    program: &str,
    windows_extensions: Option<&[OsString]>,
) -> Vec<PathBuf> {
    let mut candidates = vec![dir.join(program)];

    if let Some(windows_extensions) = windows_extensions
        && Path::new(program).extension().is_none()
    {
        for extension in windows_extensions {
            let mut file_name = OsString::from(program);
            file_name.push(extension);
            candidates.push(dir.join(file_name));
        }
    }

    candidates
}

fn windows_pathext_extensions() -> Vec<OsString> {
    let raw = std::env::var_os("PATHEXT")
        .unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH"));
    parse_windows_extensions(raw.as_os_str())
}

fn parse_windows_extensions(raw: &OsStr) -> Vec<OsString> {
    let mut extensions = Vec::new();
    let mut seen_lowercase = Vec::new();

    for segment in raw.to_string_lossy().split(';') {
        let segment = segment.trim();
        if segment.is_empty() {
            continue;
        }

        let normalized = if segment.starts_with('.') {
            segment.to_string()
        } else {
            format!(".{segment}")
        };
        let lowercase = normalized.to_ascii_lowercase();
        if seen_lowercase.iter().any(|existing| existing == &lowercase) {
            continue;
        }

        seen_lowercase.push(lowercase);
        extensions.push(OsString::from(normalized));
    }

    extensions
}

fn looks_like_path(program: &str) -> bool {
    // Treat both separators as paths, even on unix. It is harmless and avoids surprises when a
    // caller passes a Windows-style path.
    program.contains('/') || program.contains('\\')
}

fn command_with<'a>(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    env: &[ProcessEnvPair<'a>],
) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if !env.is_empty() {
        command.envs(env.iter().copied());
    }
    command
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
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir, prepend_path};
    use std::fs;

    #[cfg(unix)]
    fn shell_program() -> &'static str {
        "/bin/sh"
    }

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

    #[test]
    fn parse_windows_extensions_normalizes_and_deduplicates_entries() {
        let parsed = parse_windows_extensions(OsStr::new("EXE; .Cmd ; ; .BAT ;.exe"));
        assert_eq!(
            parsed,
            vec![
                OsString::from(".EXE"),
                OsString::from(".Cmd"),
                OsString::from(".BAT"),
            ]
        );
    }

    #[test]
    fn path_lookup_candidates_adds_windows_extensions_for_extensionless_program() {
        let dir = Path::new("/tmp/path-candidates");
        let windows_extensions = vec![OsString::from(".EXE"), OsString::from(".CMD")];

        let candidates = path_lookup_candidates(dir, "git", Some(windows_extensions.as_slice()));

        assert_eq!(
            candidates,
            vec![dir.join("git"), dir.join("git.EXE"), dir.join("git.CMD"),]
        );
    }

    #[test]
    fn path_lookup_candidates_skips_windows_extensions_when_program_already_has_extension() {
        let dir = Path::new("/tmp/path-candidates");
        let windows_extensions = vec![OsString::from(".EXE"), OsString::from(".CMD")];

        let candidates =
            path_lookup_candidates(dir, "git.exe", Some(windows_extensions.as_slice()));

        assert_eq!(candidates, vec![dir.join("git.exe")]);
    }

    #[cfg(unix)]
    #[test]
    fn run_output_returns_output_for_nonzero_status() {
        let output = run_output(
            shell_program(),
            &["-c", "printf 'oops' 1>&2; printf 'out'; exit 2"],
        )
        .expect("run output");

        assert!(!output.status.success());
        assert_eq!(output.stdout_lossy(), "out");
        assert_eq!(output.stderr_lossy(), "oops");
    }

    #[cfg(unix)]
    #[test]
    fn run_output_with_applies_cwd_and_env_overrides() {
        let cwd = tempfile::TempDir::new().expect("tempdir");
        let output = run_output_with(
            shell_program(),
            &["-c", "printf '%s|%s' \"$PWD\" \"$NILS_PROCESS_TEST_ENV\""],
            Some(cwd.path()),
            &[("NILS_PROCESS_TEST_ENV", "ok")],
        )
        .expect("run output with cwd/env");

        let rendered = output.stdout_trimmed();
        let (reported_pwd, reported_flag) = rendered
            .split_once('|')
            .expect("expected delimiter in output");
        assert_eq!(reported_flag, "ok");

        let expected = cwd.path().canonicalize().expect("canonicalize cwd");
        let reported = Path::new(reported_pwd)
            .canonicalize()
            .expect("canonicalize reported pwd");
        assert_eq!(reported, expected);
    }

    #[cfg(unix)]
    #[test]
    fn run_checked_returns_nonzero_error_with_captured_output() {
        let err = run_checked(
            shell_program(),
            &["-c", "printf 'e' 1>&2; printf 'o'; exit 7"],
        )
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
        let stdout =
            run_stdout_trimmed(shell_program(), &["-c", "printf ' hello \\n\\n'"]).expect("stdout");

        assert_eq!(stdout, "hello");
    }

    #[cfg(unix)]
    #[test]
    fn run_status_helpers_keep_stdio_contracts() {
        let quiet = run_status_quiet(shell_program(), &["-c", "exit 0"]).expect("quiet status");
        assert!(quiet.success());

        let inherit =
            run_status_inherit(shell_program(), &["-c", "exit 3"]).expect("inherit status");
        assert_eq!(inherit.code(), Some(3));
    }

    #[cfg(unix)]
    #[test]
    fn run_status_quiet_with_applies_env_overrides() {
        let status = run_status_quiet_with(
            shell_program(),
            &["-c", "test \"$NILS_PROCESS_TEST_FLAG\" = on"],
            None,
            &[("NILS_PROCESS_TEST_FLAG", "on")],
        )
        .expect("status with env");

        assert!(status.success());
    }

    #[test]
    fn browser_open_command_prefers_open_then_xdg_open() {
        let lock = GlobalStateLock::new();

        let both = StubBinDir::new();
        both.write_exe("open", "#!/bin/sh\nexit 0\n");
        both.write_exe("xdg-open", "#!/bin/sh\nexit 0\n");
        let _both_path_guard = EnvGuard::set(&lock, "PATH", &both.path_str());
        assert_eq!(browser_open_command(), Some("open"));
        drop(_both_path_guard);

        let xdg_only = StubBinDir::new();
        xdg_only.write_exe("xdg-open", "#!/bin/sh\nexit 0\n");
        let _xdg_path_guard = EnvGuard::set(&lock, "PATH", &xdg_only.path_str());
        assert_eq!(browser_open_command(), Some("xdg-open"));
        drop(_xdg_path_guard);

        let empty = tempfile::TempDir::new().expect("tempdir");
        let empty_path = empty.path().to_string_lossy().to_string();
        let _empty_path_guard = EnvGuard::set(&lock, "PATH", &empty_path);
        assert_eq!(browser_open_command(), None);
    }

    #[test]
    fn headless_browser_launch_failure_detection_matches_xdg_open_signals() {
        let stderr =
            b"/usr/bin/open: 882: www-browser: not found\nxdg-open: no method available for opening 'https://example.com'\n";
        assert!(is_headless_browser_launch_failure(&[], stderr));
    }

    #[test]
    fn headless_browser_launch_failure_detection_does_not_mask_other_errors() {
        assert!(!is_headless_browser_launch_failure(
            &[],
            b"open: permission denied\n"
        ));
    }
}

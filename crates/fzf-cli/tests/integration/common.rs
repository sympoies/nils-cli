#![allow(dead_code)]

use nils_test_support::{StubBinDir, cmd};
use std::path::Path;

pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Build the child options with forge identity and config lookups pointed at an
/// empty per-run directory, so the developer's forge profiles cannot leak into
/// the child process. The returned directory must outlive the child process.
fn isolated_options(dir: &Path, envs: &[(&str, &str)]) -> (tempfile::TempDir, cmd::CmdOptions) {
    let isolated = tempfile::TempDir::new().expect("isolated config dir");
    let xdg_config = isolated.path().join("xdg-config");
    let xdg_state = isolated.path().join("xdg-state");
    let mut all_envs = vec![
        ("XDG_CONFIG_HOME", xdg_config.to_str().expect("config path")),
        ("XDG_STATE_HOME", xdg_state.to_str().expect("state path")),
    ];
    all_envs.extend_from_slice(envs);
    let options =
        cmd::options_in_dir_with_envs(dir, &all_envs).with_env_remove("FORGE_IDENTITY_PRINCIPAL");
    (isolated, options)
}

pub fn run_fzf_cli(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) -> CmdOutput {
    let (_isolated, mut options) = isolated_options(dir, envs);
    if let Some(input) = stdin {
        options = options.with_stdin_str(input);
    }
    let output = cmd::run_resolved("fzf-cli", args, &options);
    CmdOutput {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn run_fzf_cli_with_stub_path(
    dir: &Path,
    stub_path: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) -> CmdOutput {
    let (_isolated, options) = isolated_options(dir, envs);
    let mut options = options.with_path_prepend(stub_path);
    if let Some(input) = stdin {
        options = options.with_stdin_str(input);
    }
    let output = cmd::run_resolved("fzf-cli", args, &options);
    CmdOutput {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn run_fzf_cli_with_stub_only_path(
    dir: &Path,
    stub_path: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) -> CmdOutput {
    let path = stub_path.to_string_lossy().to_string();
    let (_isolated, options) = isolated_options(dir, envs);
    let mut options = options.with_env("PATH", &path);
    if let Some(input) = stdin {
        options = options.with_stdin_str(input);
    }
    let output = cmd::run_resolved("fzf-cli", args, &options);
    CmdOutput {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn make_stub_dir() -> StubBinDir {
    StubBinDir::new()
}

pub fn fzf_stub_script() -> &'static str {
    nils_test_support::stubs::fzf_stub_script()
}

pub fn write_exe(dir: &Path, name: &str, content: &str) {
    nils_test_support::write_exe(dir, name, content);
}

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[allow(unused_imports)]
pub use nils_test_support::write_exe;
use nils_test_support::StubBinDir;

#[allow(dead_code)]
pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    #[allow(dead_code)]
    pub stderr: String,
}

pub fn fzf_cli_bin() -> PathBuf {
    if let Ok(bin) =
        std::env::var("CARGO_BIN_EXE_fzf-cli").or_else(|_| std::env::var("CARGO_BIN_EXE_fzf_cli"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("fzf-cli");
    if bin.exists() {
        return bin;
    }

    panic!("fzf-cli binary path: NotPresent");
}

pub fn run_fzf_cli(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) -> CmdOutput {
    let mut cmd = Command::new(fzf_cli_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (k, v) in envs {
        cmd.env(k, v);
    }

    if stdin.is_some() {
        cmd.stdin(Stdio::piped());
    } else {
        cmd.stdin(Stdio::null());
    }

    let mut child = cmd.spawn().expect("spawn fzf-cli");
    if let Some(input) = stdin {
        if let Some(mut child_stdin) = child.stdin.take() {
            use std::io::Write;
            child_stdin
                .write_all(input.as_bytes())
                .expect("write stdin");
        }
    }

    let output = child.wait_with_output().expect("wait fzf-cli");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

#[allow(dead_code)]
pub fn make_stub_dir() -> StubBinDir {
    StubBinDir::new()
}

#[allow(dead_code)]
pub fn fzf_stub_script() -> &'static str {
    nils_test_support::stubs::fzf_stub_script()
}

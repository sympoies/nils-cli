use nils_test_support::bin;
use nils_test_support::cmd::{self, CmdOptions};
use std::path::{Path, PathBuf};

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
    bin::resolve("fzf-cli")
}

pub fn run_fzf_cli(
    dir: &Path,
    args: &[&str],
    envs: &[(&str, &str)],
    stdin: Option<&str>,
) -> CmdOutput {
    let mut options = CmdOptions::default().with_cwd(dir);
    for (k, v) in envs {
        options = options.with_env(k, v);
    }
    if let Some(input) = stdin {
        options = options.with_stdin_str(input);
    }
    let bin = fzf_cli_bin();
    let output = cmd::run_with(&bin, args, &options);
    CmdOutput {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
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

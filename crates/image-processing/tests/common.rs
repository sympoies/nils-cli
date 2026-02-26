use nils_test_support::cmd;
use std::path::Path;

use nils_test_support::StubBinDir;

pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn run_image_processing(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let output = cmd::run_resolved_in_dir("image-processing", dir, args, envs, None);
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

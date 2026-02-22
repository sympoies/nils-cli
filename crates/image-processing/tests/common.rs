#![allow(dead_code)]

use nils_test_support::bin;
use nils_test_support::cmd;
use std::path::{Path, PathBuf};

use nils_test_support::StubBinDir;

pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn image_processing_bin() -> PathBuf {
    bin::resolve("image-processing")
}

pub fn run_image_processing(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let output = cmd::run_resolved_in_dir("image-processing", dir, args, envs, None);
    CmdOutput {
        code: output.code,
        stdout: output.stdout_text(),
        stderr: output.stderr_text(),
    }
}

pub fn make_stub_dir() -> StubBinDir {
    StubBinDir::new()
}

pub fn write_exe(dir: &Path, name: &str, content: impl AsRef<str>) {
    nils_test_support::write_exe(dir, name, content.as_ref());
}

pub fn identify_stub_script() -> String {
    nils_test_support::stubs::identify_stub_script()
}

pub fn convert_stub_script() -> String {
    nils_test_support::stubs::convert_stub_script()
}

pub fn magick_stub_script() -> String {
    nils_test_support::stubs::magick_stub_script()
}

pub fn dwebp_stub_script() -> String {
    nils_test_support::stubs::dwebp_stub_script()
}

pub fn cwebp_stub_script() -> String {
    nils_test_support::stubs::cwebp_stub_script()
}

pub fn djpeg_stub_script() -> String {
    nils_test_support::stubs::djpeg_stub_script()
}

pub fn cjpeg_stub_script() -> String {
    nils_test_support::stubs::cjpeg_stub_script()
}

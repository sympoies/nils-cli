#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use nils_test_support::StubBinDir;

pub struct CmdOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn image_processing_bin() -> PathBuf {
    if let Ok(bin) = std::env::var("CARGO_BIN_EXE_image-processing")
        .or_else(|_| std::env::var("CARGO_BIN_EXE_image_processing"))
    {
        return PathBuf::from(bin);
    }

    let exe = std::env::current_exe().expect("current exe");
    let target_dir = exe.parent().and_then(|p| p.parent()).expect("target dir");
    let bin = target_dir.join("image-processing");
    if bin.exists() {
        return bin;
    }

    panic!("image-processing binary path: NotPresent");
}

pub fn run_image_processing(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> CmdOutput {
    let mut cmd = Command::new(image_processing_bin());
    cmd.args(args)
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());

    for (k, v) in envs {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("run image-processing");
    CmdOutput {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
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

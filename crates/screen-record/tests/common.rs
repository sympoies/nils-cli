#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nils_test_support::bin::resolve;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
use tempfile::TempDir;

pub struct ScreenRecordHarness {
    home_dir: TempDir,
    agents_home: PathBuf,
}

impl ScreenRecordHarness {
    pub fn new() -> Self {
        let home_dir = TempDir::new().expect("tempdir");
        let agents_home = home_dir.path().join(".agents");
        std::fs::create_dir_all(agents_home.join("out")).expect("create AGENTS_HOME/out");

        Self {
            home_dir,
            agents_home,
        }
    }

    pub fn screen_record_bin(&self) -> PathBuf {
        resolve("screen-record")
    }

    pub fn cmd_options(&self, cwd: &Path) -> CmdOptions {
        let home = self.home_dir.path().to_string_lossy().to_string();
        let agents_home = self.agents_home.to_string_lossy().to_string();
        CmdOptions::new()
            .with_cwd(cwd)
            .with_env("HOME", &home)
            .with_env("AGENTS_HOME", &agents_home)
            .with_env("AGENTS_SCREEN_RECORD_TEST_MODE", "1")
            .with_env("AGENTS_SCREEN_RECORD_TEST_TIMESTAMP", "20260101-000000")
    }

    pub fn run(&self, cwd: &Path, args: &[&str]) -> CmdOutput {
        run_with(&self.screen_record_bin(), args, &self.cmd_options(cwd))
    }

    pub fn run_with_options(&self, cwd: &Path, args: &[&str], options: CmdOptions) -> CmdOutput {
        run_with(&self.screen_record_bin(), args, &options.with_cwd(cwd))
    }
}

impl Default for ScreenRecordHarness {
    fn default() -> Self {
        Self::new()
    }
}

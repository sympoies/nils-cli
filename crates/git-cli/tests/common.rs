#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nils_test_support::StubBinDir;
use nils_test_support::bin::resolve;
use nils_test_support::cmd::{CmdOptions, CmdOutput, run_with};
pub use nils_test_support::git::git;
use nils_test_support::git::{InitRepoOptions, init_repo_with};
use tempfile::TempDir;

pub struct GitCliHarness {
    home_dir: TempDir,
    xdg_config_home: PathBuf,
    stub_bin_dir: StubBinDir,
}

impl GitCliHarness {
    pub fn new() -> Self {
        let home_dir = TempDir::new().expect("tempdir");
        let xdg_config_home = home_dir.path().join(".config");
        std::fs::create_dir_all(&xdg_config_home).expect("create XDG_CONFIG_HOME");

        let stub_bin_dir = StubBinDir::new();
        let pbcopy = nils_test_support::stubs::pbcopy_stub_script();
        stub_bin_dir.write_exe("pbcopy", pbcopy.as_str());
        let wl_copy = nils_test_support::stubs::wl_copy_stub_script();
        stub_bin_dir.write_exe("wl-copy", wl_copy.as_str());
        let xclip = nils_test_support::stubs::xclip_stub_script();
        stub_bin_dir.write_exe("xclip", xclip.as_str());
        let xsel = nils_test_support::stubs::xsel_stub_script();
        stub_bin_dir.write_exe("xsel", xsel.as_str());
        stub_bin_dir.write_exe("file", nils_test_support::stubs::file_stub_script());
        let git_scope = nils_test_support::stubs::git_scope_stub_script();
        stub_bin_dir.write_exe("git-scope", git_scope.as_str());

        Self {
            home_dir,
            xdg_config_home,
            stub_bin_dir,
        }
    }

    pub fn git_cli_bin(&self) -> PathBuf {
        resolve("git-cli")
    }

    pub fn cmd_options(&self, cwd: &Path) -> CmdOptions {
        let home = self.home_dir.path().to_string_lossy().to_string();
        let xdg_config_home = self.xdg_config_home.to_string_lossy().to_string();
        CmdOptions::new()
            .with_cwd(cwd)
            .with_path_prepend(self.stub_bin_dir.path())
            .with_env("HOME", &home)
            .with_env("XDG_CONFIG_HOME", &xdg_config_home)
            .with_env("GIT_CONFIG_NOSYSTEM", "1")
            .with_env("GIT_CONFIG_GLOBAL", "/dev/null")
            .with_env("GIT_PAGER", "cat")
            .with_env("PAGER", "cat")
            .with_env("TERM", "dumb")
            .with_env("TZ", "UTC")
            .with_env("LC_ALL", "C")
            .with_env_remove_prefix("GIT_TRACE")
    }

    pub fn run(&self, cwd: &Path, args: &[&str]) -> CmdOutput {
        run_with(&self.git_cli_bin(), args, &self.cmd_options(cwd))
    }
}

impl Default for GitCliHarness {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init_repo() -> tempfile::TempDir {
    init_repo_with(
        InitRepoOptions::new()
            .with_branch("main")
            .with_initial_commit(),
    )
}

pub fn init_bare_remote() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("tempdir");
    git(dir.path(), &["init", "--bare", "-q"]);
    dir
}

pub fn write_context_json_git_stub(stubs: &StubBinDir) {
    stubs.write_exe(
        "git",
        r#"#!/bin/bash
set -euo pipefail

args=("$@")

if [[ ${#args[@]} -ge 2 && "${args[0]}" == "rev-parse" && "${args[1]}" == "--is-inside-work-tree" ]]; then
  exit 0
fi

if [[ ${#args[@]} -ge 4 && "${args[0]}" == "diff" && "${args[1]}" == "--cached" && "${args[2]}" == "--quiet" && "${args[3]}" == "--exit-code" ]]; then
  exit 1
fi

if [[ ${#args[@]} -ge 2 && "${args[0]}" == "symbolic-ref" && "${args[1]}" == "--quiet" ]]; then
  echo "main"
  exit 0
fi

if [[ ${#args[@]} -ge 2 && "${args[0]}" == "rev-parse" && "${args[1]}" == "--short" ]]; then
  echo "abc123"
  exit 0
fi

if [[ ${#args[@]} -ge 2 && "${args[0]}" == "rev-parse" && "${args[1]}" == "--show-toplevel" ]]; then
  pwd
  exit 0
fi

if [[ ${#args[@]} -ge 5 && "${args[0]}" == "-c" && "${args[1]}" == "core.quotepath=false" && "${args[2]}" == "diff" && "${args[3]}" == "--cached" && "${args[4]}" == "--no-color" ]]; then
  echo "diff --git a/hello.txt b/hello.txt"
  exit 0
fi

if [[ ${#args[@]} -ge 6 && "${args[0]}" == "-c" && "${args[1]}" == "core.quotepath=false" && "${args[2]}" == "diff" && "${args[3]}" == "--cached" && "${args[4]}" == "--name-status" && "${args[5]}" == "-z" ]]; then
  printf "A\0hello.txt\0"
  exit 0
fi

if [[ ${#args[@]} -ge 6 && "${args[0]}" == "-c" && "${args[1]}" == "core.quotepath=false" && "${args[2]}" == "diff" && "${args[3]}" == "--cached" && "${args[4]}" == "--numstat" ]]; then
  last_index=$((${#args[@]} - 1))
  path="${args[$last_index]}"
  printf "1\t0\t%s\n" "$path"
  exit 0
fi

exit 0
"#,
    );
}

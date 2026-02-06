use anyhow::{Context, Result};
use nils_common::process;
use std::process::Output;

pub fn cmd_exists(cmd: &str) -> bool {
    process::cmd_exists(cmd)
}

pub fn run_output(cmd: &str, args: &[&str]) -> Result<Output> {
    process::run_output(cmd, args)
        .map(|output| output.into_std_output())
        .with_context(|| format!("spawn {cmd}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock, StubBinDir};
    use tempfile::TempDir;

    #[test]
    fn run_output_returns_ok_for_nonzero_exit_status() {
        let lock = GlobalStateLock::new();
        let stubs = StubBinDir::new();
        stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "rev-parse" && "${2:-}" == "--is-inside-work-tree" ]]; then
  exit 128
fi
exit 0
"#,
        );

        let _path_guard = EnvGuard::set(&lock, "PATH", &stubs.path_str());
        let output = run_output("git", &["rev-parse", "--is-inside-work-tree"]).expect("output");
        assert!(!output.status.success());
    }

    #[test]
    fn git_repo_probe_semantics_are_stable() {
        fn probe() -> bool {
            run_output("git", &["rev-parse", "--is-inside-work-tree"])
                .map(|output| output.status.success())
                .unwrap_or(false)
        }

        let lock = GlobalStateLock::new();

        let success_stubs = StubBinDir::new();
        success_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "rev-parse" && "${2:-}" == "--is-inside-work-tree" ]]; then
  exit 0
fi
exit 1
"#,
        );
        let success_path_guard = EnvGuard::set(&lock, "PATH", &success_stubs.path_str());
        assert!(probe());
        drop(success_path_guard);

        let fail_stubs = StubBinDir::new();
        fail_stubs.write_exe(
            "git",
            r#"#!/bin/bash
set -euo pipefail
if [[ "${1:-}" == "rev-parse" && "${2:-}" == "--is-inside-work-tree" ]]; then
  exit 128
fi
exit 1
"#,
        );
        let fail_path_guard = EnvGuard::set(&lock, "PATH", &fail_stubs.path_str());
        assert!(!probe());
        drop(fail_path_guard);

        let empty = TempDir::new().expect("tempdir");
        let empty_path = empty.path().to_string_lossy().to_string();
        let _missing_path_guard = EnvGuard::set(&lock, "PATH", &empty_path);
        assert!(!probe());
    }
}

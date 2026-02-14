mod common;

use common::{GitCliHarness, git, init_repo};
use nils_test_support::StubBinDir;
use nils_test_support::cmd::{CmdOutput, run_with};
use std::path::Path;

fn run_with_open_stub(harness: &GitCliHarness, cwd: &Path, args: &[&str]) -> CmdOutput {
    let stubs = StubBinDir::new();
    stubs.write_exe(
        "open",
        r#"#!/bin/bash
set -euo pipefail
exit 0
"#,
    );
    let options = harness.cmd_options(cwd).with_path_prepend(stubs.path());
    run_with(&harness.git_cli_bin(), args, &options)
}

#[test]
fn open_repo_opens_normalized_remote_homepage() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );

    let output = run_with_open_stub(&harness, dir.path(), &["open", "repo"]);

    assert_eq!(output.code, 0);
    assert_eq!(output.stderr_text(), "");
    assert_eq!(
        output.stdout_text(),
        "🌐 Opened: https://github.com/acme/repo\n"
    );
}

#[test]
fn open_commit_opens_commit_page_for_ref() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );
    let sha = git(dir.path(), &["rev-parse", "HEAD"])
        .trim_end_matches(['\n', '\r'])
        .to_string();

    let output = run_with_open_stub(&harness, dir.path(), &["open", "commit", "HEAD"]);

    assert_eq!(output.code, 0);
    assert_eq!(output.stderr_text(), "");
    assert_eq!(
        output.stdout_text(),
        format!("🔗 Opened: https://github.com/acme/repo/commit/{sha}\n")
    );
}

#[test]
fn open_file_encodes_path_spaces() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );

    let output = run_with_open_stub(
        &harness,
        dir.path(),
        &["open", "file", "docs/my file.md", "main"],
    );

    assert_eq!(output.code, 0);
    assert_eq!(output.stderr_text(), "");
    assert_eq!(
        output.stdout_text(),
        "📄 Opened: https://github.com/acme/repo/blob/main/docs/my%20file.md\n"
    );
}

#[test]
fn open_actions_rejects_non_github_provider() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@gitlab.com:acme/repo.git"],
    );

    let output = harness.run(dir.path(), &["open", "actions"]);

    assert_eq!(output.code, 1);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(
        output.stderr_text(),
        "❗ actions is only supported for GitHub remotes.\n"
    );
}

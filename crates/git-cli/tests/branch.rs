mod common;

use common::{init_repo, GitCliHarness};
use nils_test_support::cmd::{run_with, CmdOutput};
use nils_test_support::git::{commit_file, git};
use std::path::Path;

fn run_with_stdin(harness: &GitCliHarness, cwd: &Path, args: &[&str], stdin: &str) -> CmdOutput {
    let options = harness.cmd_options(cwd).with_stdin_str(stdin);
    run_with(&harness.git_cli_bin(), args, &options)
}

fn setup_repo_with_branches() -> tempfile::TempDir {
    let dir = init_repo();
    commit_file(dir.path(), "file.txt", "base\n", "base");

    git(dir.path(), &["checkout", "-b", "feature-merged"]);
    commit_file(dir.path(), "feature.txt", "merged\n", "feature");
    git(dir.path(), &["checkout", "main"]);
    git(
        dir.path(),
        &["merge", "--no-ff", "feature-merged", "-m", "merge feature"],
    );

    git(dir.path(), &["checkout", "-b", "feature-squash"]);
    commit_file(dir.path(), "squash.txt", "squash\n", "squash work");
    let squash_sha = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "main"]);
    git(dir.path(), &["cherry-pick", "-n", squash_sha.trim()]);
    git(dir.path(), &["commit", "-m", "squash commit"]);

    git(dir.path(), &["checkout", "-b", "develop"]);
    commit_file(dir.path(), "dev.txt", "dev\n", "dev work");
    git(dir.path(), &["checkout", "main"]);

    dir
}

#[test]
fn branch_cleanup_merged_lists_candidates_and_aborts() {
    let harness = GitCliHarness::new();
    let dir = setup_repo_with_branches();

    let output = run_with_stdin(&harness, dir.path(), &["branch", "cleanup"], "n\n");

    assert_eq!(output.code, 1);
    assert!(output
        .stdout_text()
        .contains("🧹 Merged branches to delete (base: HEAD):"));
    assert!(output.stdout_text().contains("  - feature-merged"));
    assert!(!output.stdout_text().contains("feature-squash"));
    assert!(!output.stdout_text().contains("develop"));
    assert!(output.stdout_text().contains("🚫 Aborted"));
}

#[test]
fn branch_cleanup_squash_lists_squash_candidates_and_aborts() {
    let harness = GitCliHarness::new();
    let dir = setup_repo_with_branches();

    let output = run_with_stdin(
        &harness,
        dir.path(),
        &["branch", "cleanup", "--squash"],
        "n\n",
    );

    assert_eq!(output.code, 1);
    assert!(output
        .stdout_text()
        .contains("🧹 Branches to delete (base: HEAD, mode: squash):"));
    assert!(output.stdout_text().contains("  - feature-merged"));
    assert!(output.stdout_text().contains("  - feature-squash"));
    assert!(!output.stdout_text().contains("develop"));
    assert!(output.stdout_text().contains("🚫 Aborted"));
}

#[test]
fn branch_cleanup_protects_base_and_main() {
    let harness = GitCliHarness::new();
    let dir = setup_repo_with_branches();

    let output = run_with_stdin(
        &harness,
        dir.path(),
        &["branch", "cleanup", "--base", "main"],
        "n\n",
    );

    assert_eq!(output.code, 1);
    assert!(output
        .stdout_text()
        .contains("🧹 Merged branches to delete (base: main):"));
    assert!(output.stdout_text().contains("  - feature-merged"));
    assert!(!output.stdout_text().contains("  - main"));
    assert!(output.stdout_text().contains("🚫 Aborted"));
}

#[test]
fn branch_cleanup_no_candidates_message() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    commit_file(dir.path(), "file.txt", "base\n", "base");

    let output = harness.run(dir.path(), &["branch", "cleanup"]);

    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_text(), "✅ No deletable merged branches.\n");
}

#[test]
fn branch_cleanup_squash_no_candidates_message() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    commit_file(dir.path(), "file.txt", "base\n", "base");

    let output = harness.run(dir.path(), &["branch", "cleanup", "--squash"]);

    assert_eq!(output.code, 0);
    assert_eq!(output.stdout_text(), "✅ No deletable branches found.\n");
}

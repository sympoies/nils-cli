use crate::common;
use common::{GitCliHarness, git, init_repo};
use nils_test_support::StubBinDir;
use nils_test_support::cmd::{CmdOutput, run_with};
use std::path::Path;

fn run_with_open_script(
    harness: &GitCliHarness,
    cwd: &Path,
    args: &[&str],
    open_script: &str,
) -> CmdOutput {
    let stubs = StubBinDir::new();
    stubs.write_exe("open", open_script);
    let options = harness.cmd_options(cwd).with_path_prepend(stubs.path());
    run_with(&harness.git_cli_bin(), args, &options)
}

fn run_with_open_stub(harness: &GitCliHarness, cwd: &Path, args: &[&str]) -> CmdOutput {
    run_with_open_script(
        harness,
        cwd,
        args,
        r#"#!/bin/bash
set -euo pipefail
exit 0
"#,
    )
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
fn open_leaf_help_does_not_launch_browser() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );

    for args in [
        &["open", "compare", "--help"][..],
        &["open", "file", "--help"][..],
        &["open", "blame", "--help"][..],
    ] {
        let output = run_with_open_script(
            &harness,
            dir.path(),
            args,
            r#"#!/bin/bash
echo "open should not be called" >&2
exit 99
"#,
        );

        assert_eq!(output.code, 0, "args: {args:?}");
        assert_eq!(output.stderr_text(), "", "args: {args:?}");
        assert!(
            output.stdout_text().contains("Usage:\n  git-cli open"),
            "args: {args:?}, stdout: {}",
            output.stdout_text()
        );
    }
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

#[test]
fn open_repo_headless_environment_prints_clear_manual_open_warning() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );

    let output = run_with_open_script(
        &harness,
        dir.path(),
        &["open", "repo"],
        r#"#!/bin/bash
set -euo pipefail
echo "/usr/bin/open: 882: www-browser: not found" >&2
echo "xdg-open: no method available for opening '$1'" >&2
exit 3
"#,
    );

    assert_eq!(output.code, 0);
    assert_eq!(
        output.stdout_text(),
        "🔗 URL: https://github.com/acme/repo\n"
    );
    assert_eq!(
        output.stderr_text(),
        "⚠️  Could not launch a browser in this environment; open the URL manually.\n"
    );
}

#[test]
fn open_repo_non_headless_open_error_still_fails() {
    let harness = GitCliHarness::new();
    let dir = init_repo();
    git(
        dir.path(),
        &["remote", "add", "origin", "git@github.com:acme/repo.git"],
    );

    let output = run_with_open_script(
        &harness,
        dir.path(),
        &["open", "repo"],
        r#"#!/bin/bash
set -euo pipefail
echo "open: permission denied" >&2
exit 126
"#,
    );

    assert_eq!(output.code, 126);
    assert_eq!(output.stdout_text(), "");
    assert_eq!(output.stderr_text(), "open: permission denied\n");
}

#[test]
fn open_pr_identity_missing_credential_refuses_without_browser_or_account_fallback() {
    use pretty_assertions::assert_eq;
    let repo = init_repo();
    let config = tempfile::tempdir().unwrap();
    let stubs = StubBinDir::new();
    let dir = config.path().join("forge-cli");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("identity.toml"),
        include_str!("../../../nils-common/tests/fixtures/identity/policy.toml"),
    )
    .unwrap();
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:sandbox/widget.git",
        ],
    );
    git(repo.path(), &["config", "branch.main.remote", "origin"]);
    git(
        repo.path(),
        &["config", "branch.main.merge", "refs/heads/main"],
    );
    stubs.write_exe(
        "gh",
        "#!/bin/sh\necho unexpected-credential-fallback >&2\nexit 99\n",
    );
    stubs.write_exe(
        "open",
        "#!/bin/sh\necho unexpected-browser-fallback >&2\nexit 99\n",
    );
    let output = std::process::Command::new(nils_test_support::bin::resolve("git-cli"))
        .current_dir(repo.path())
        .args(["open", "pr"])
        .env("XDG_CONFIG_HOME", config.path())
        .env("XDG_STATE_HOME", config.path().join("state"))
        .env("FORGE_IDENTITY_PRINCIPAL", "contributor")
        .env_remove("FIXTURE_ACCOUNT_A_CREDENTIAL")
        .env(
            "PATH",
            format!(
                "{}:{}",
                stubs.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("identity_credential_missing"), "{stderr}");
    assert!(!stderr.contains("unexpected-"));
}

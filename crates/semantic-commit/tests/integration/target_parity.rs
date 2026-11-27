use crate::identity_fixture::Fixture;
use nils_test_support::cmd::run_with;
use pretty_assertions::assert_eq;
use std::{fs, path::Path};

fn check_parity(operation: &str) {
    let Some(f) = Fixture::new(false) else { return };
    f.git(&[
        "remote",
        "set-url",
        "--add",
        "--push",
        "origin",
        "https://github.com/sandbox/widget.git",
    ]);
    f.git(&[
        "remote",
        "set-url",
        "--add",
        "--push",
        "origin",
        "https://github.com/sandbox/other.git",
    ]);
    let primary = &f.repo;
    for args in [
        &["config", "branch.main.remote", "origin"][..],
        &["config", "branch.main.merge", "refs/heads/main"][..],
        &["update-ref", "refs/remotes/origin/main", "HEAD"][..],
    ] {
        assert_eq!(f.git_at(primary, args).code, 0);
    }
    {
        let repo = if operation == "default-branch" {
            primary
        } else {
            &f.worktree
        };
        fs::write(repo.join("parity-change"), b"staged\n").unwrap();
        assert_eq!(f.git_at(repo, &["add", "parity-change"]).code, 0);
        let head = f
            .git_at(repo, &["rev-parse", "HEAD"])
            .stdout_text()
            .trim()
            .to_owned();
        let receipt = f.root.path().join("receipt.json");
        let probe = f.root.path().join("probe-log");
        f.stubs.write_exe(
            "gh",
            "#!/bin/sh\nprintf probe >> \"$FIXTURE_PROBE_LOG\"\nexit 1\n",
        );
        let run = |dry: bool| {
            let mut args = vec![operation, "--repo", repo.to_str().unwrap()];
            if operation == "default-branch" {
                args.extend(["--message", "fix: parity", "--expect-head", &head]);
                if !dry {
                    args.extend(["--receipt-out", receipt.to_str().unwrap()]);
                }
            } else {
                args.extend(["--target", "HEAD"]);
            }
            if dry {
                args.push("--dry-run");
            }
            let mut options = f
                .options(repo)
                .with_env("FIXTURE_PROBE_LOG", probe.to_str().unwrap());
            if dry {
                options = options.with_env_remove("REPLAY_TEST_CREDENTIAL");
            }
            run_with(
                Path::new(env!("CARGO_BIN_EXE_semantic-commit")),
                &args,
                &options,
            )
        };
        let actual = run(false);
        assert_eq!(actual.code, 1, "{operation}: {}", actual.stderr_text());
        assert!(
            actual.stderr_text().contains("identity_target_ambiguous"),
            "{operation}: {}",
            actual.stderr_text()
        );
        let audit = f.root.path().join("state/forge-cli/identity-audit.jsonl");
        let before = fs::read(&audit).unwrap();
        let preview = run(true);
        assert_eq!(preview.code, 1, "{operation}: {}", preview.stdout_text());
        assert!(preview.stderr_text().contains("identity_target_ambiguous"));
        assert_eq!(fs::read(&audit).unwrap(), before);
        assert_eq!(
            f.git_at(
                repo,
                &[
                    "remote",
                    "set-url",
                    "--delete",
                    "--push",
                    "origin",
                    "https://github.com/sandbox/other.git"
                ]
            )
            .code,
            0
        );
        let preview = run(true);
        assert_eq!(preview.code, 0, "{operation}: {}", preview.stderr_text());
        assert!(!probe.exists());
        assert_eq!(fs::read(audit).unwrap(), before);
    }
}

#[test]
fn dry_run_authoring_target_parity_default_branch() {
    check_parity("default-branch");
}
#[test]
fn dry_run_authoring_target_parity_fixup() {
    check_parity("fixup");
}
#[test]
fn dry_run_authoring_target_parity_squash() {
    check_parity("squash");
}

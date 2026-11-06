//! Config parsing and create/deliver decisions for per-kind evidence policy.
use std::fs;

use forge_cli::config::ForgeConfig;
use nils_test_support::fixtures::init_subject_repo;
use nils_test_support::git::git;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::support::{StubEnv, parse_envelope, run_forge_cli_in};

fn install_local_remote(repo: &std::path::Path, root: &std::path::Path) {
    let remote = root.join("remote.git");
    git(root, &["init", "--bare", remote.to_str().unwrap()]);
    let rewrite = format!("url.file://{}.insteadOf", remote.display());
    git(
        repo,
        &["config", &rewrite, "https://github.com/example/project.git"],
    );
    git(repo, &["push", "-q", "origin", "main"]);
}

#[test]
fn test_first_kinds_config_accepts_known_kinds_and_empty_lists() {
    for list in [
        r#"["bug"]"#,
        r#"["feature", "bug", "docs", "chore", "ci", "refactor", "test"]"#,
        "[]",
    ] {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(".forge-cli.toml"),
            format!("[test_first]\nrequire = true\nrequire_kinds = {list}\n"),
        )
        .unwrap();
        let cfg = ForgeConfig::load_from(temp.path(), Some(temp.path()));
        assert_eq!(cfg.warnings, Vec::<String>::new(), "list={list}");
    }
}

#[test]
fn test_first_kinds_config_warns_on_invalid_lists() {
    for list in ["true", r#""bug""#, "[1]", r#"["unknown"]"#, r#"["bug", 1]"#] {
        let temp = TempDir::new().unwrap();
        fs::write(
            temp.path().join(".forge-cli.toml"),
            format!("[test_first]\nrequire_kinds = {list}\n"),
        )
        .unwrap();
        let cfg = ForgeConfig::load_from(temp.path(), Some(temp.path()));
        assert!(
            cfg.warnings.iter().any(|warning| warning.starts_with(
                "invalid-config-value:test_first.require_kinds"
            )),
            "list={list}, warnings={:?}", cfg.warnings
        );
    }
}

#[test]
fn test_first_kinds_create_and_deliver_dry_run_agree() {
    for (settings, gated) in [
        ("require = true\nrequire_kinds = [\"bug\"]", vec!["bug"]),
        ("require = true", vec!["feature", "bug"]),
        ("require = true\nrequire_kinds = [\"docs\"]", vec!["docs"]),
        ("require = true\nrequire_kinds = []", vec![]),
        ("require = false\nrequire_kinds = [\"bug\"]", vec![]),
        ("require_kinds = [\"bug\"]", vec![]),
        ("require = false\nrequire_kinds = [\"unknown\"]", vec![]),
    ] {
        for kind in nils_common::git::PrKind::all() {
            let temp = TempDir::new().unwrap();
            let repo = init_subject_repo(
                temp.path(),
                "repo",
                "https://github.com/example/project.git",
            );
            fs::write(
                repo.join(".forge-cli.toml"),
                format!("[test_first]\n{settings}\n"),
            )
            .unwrap();
            git(&repo, &["add", ".forge-cli.toml"]);
            git(&repo, &["commit", "-qm", "configure evidence policy"]);
            install_local_remote(&repo, temp.path());
            git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
            let branch = format!("{}/sample", kind.branch_prefix());
            git(&repo, &["checkout", "-qb", &branch]);
            git(
                &repo,
                &[
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    "HEAD",
                ],
            );
            git(
                &repo,
                &["branch", "--set-upstream-to", &format!("origin/{branch}")],
            );
            let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
            for operation in ["create", "deliver"] {
                let out = run_forge_cli_in(
                    &stub,
                    &[
                        "--provider",
                        "github",
                        "--host",
                        "github.com",
                        "--repo",
                        "example/project",
                        "--format",
                        "json",
                        "--dry-run",
                        "pr",
                        operation,
                        "--kind",
                        kind.as_str(),
                        "--base",
                        "main",
                        "--title",
                        "Configure evidence",
                        "--body",
                        "## Summary\nExample\n\n## Test plan\nUnit tests\n",
                    ],
                    Some(&repo),
                );
                let envelope = parse_envelope(&out.stdout);
                let required = gated.contains(&kind.as_str());
                if operation == "create" {
                    assert_eq!(
                        out.code,
                        if required { 65 } else { 0 },
                        "{settings} {kind:?}: {out:?}"
                    );
                    if required {
                        assert_eq!(envelope["error"]["code"], "test_first_evidence_required");
                        let message = envelope["error"]["message"].as_str().unwrap();
                        assert!(
                            message.contains(&format!("--kind {}", kind.as_str())),
                            "{message}"
                        );
                        assert!(message.contains("--test-first-evidence <dir>"), "{message}");
                    }
                } else {
                    assert_eq!(out.code, 0, "{out:?}");
                    let verdict = envelope["data"]["local_preflight"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|item| item["rule"] == "test_first");
                    assert_eq!(
                        verdict.is_some_and(|v| v["ok"] == false),
                        required,
                        "{settings} {kind:?}: {out:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_first_kinds_invalid_list_is_reported_by_create_and_deliver() {
    for inherited in ["[]", "[\"docs\"]"] {
        for invalid in ["true", "[1]", "[\"feature\", \"bugs\"]"] {
            let temp = TempDir::new().unwrap();
            let repo = init_subject_repo(
                temp.path(),
                "repo",
                "https://github.com/example/project.git",
            );
            fs::write(
                repo.join(".forge-cli.toml"),
                format!("[test_first]\nrequire_kinds = {invalid}\n"),
            )
            .unwrap();
            git(&repo, &["add", ".forge-cli.toml"]);
            git(&repo, &["commit", "-qm", "configure evidence policy"]);
            git(&repo, &["checkout", "-qb", "feat/sample"]);
            git(
                &repo,
                &["update-ref", "refs/remotes/origin/feat/sample", "HEAD"],
            );
            git(
                &repo,
                &["branch", "--set-upstream-to", "origin/feat/sample"],
            );
            let global_dir = temp.path().join("global");
            fs::create_dir_all(global_dir.join("forge-cli")).unwrap();
            fs::write(
                global_dir.join("forge-cli/config.toml"),
                format!("[test_first]\nrequire = true\nrequire_kinds = {inherited}\n"),
            )
            .unwrap();
            let stub = StubEnv::new()
                .env("XDG_CONFIG_HOME", global_dir.to_string_lossy())
                .gh_stub("#!/bin/sh\nexit 99\n");
            for operation in ["create", "deliver"] {
                let out = run_forge_cli_in(
                    &stub,
                    &[
                        "--provider",
                        "github",
                        "--format",
                        "json",
                        "--dry-run",
                        "pr",
                        operation,
                        "--kind",
                        "feature",
                        "--base",
                        "main",
                        "--title",
                        "Configure evidence",
                        "--body",
                        "## Summary\nExample\n\n## Test plan\nUnit tests\n",
                    ],
                    Some(&repo),
                );
                let envelope = parse_envelope(&out.stdout);
                let (code, message) = if operation == "create" {
                    assert_eq!(out.code, 65, "{inherited} {invalid}: {out:?}");
                    (&envelope["error"]["code"], &envelope["error"]["message"])
                } else {
                    assert_eq!(out.code, 0, "{out:?}");
                    let verdict = envelope["data"]["local_preflight"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|item| item["rule"] == "test_first")
                        .unwrap();
                    assert_eq!(verdict["ok"], false, "{inherited} {invalid}: {out:?}");
                    (&verdict["code"], &verdict["message"])
                };
                assert_eq!(code, "invalid_test_first_config");
                assert!(
                    message
                        .as_str()
                        .unwrap()
                        .contains("test_first.require_kinds"),
                    "{message}"
                );
                assert!(message.as_str().unwrap().contains("bug"), "{message}");
            }
            let text = run_forge_cli_in(
                &stub,
                &[
                    "--provider",
                    "github",
                    "--format",
                    "text",
                    "pr",
                    "create",
                    "--kind",
                    "feature",
                    "--base",
                    "main",
                    "--title",
                    "Configure evidence",
                    "--body",
                    "## Summary\nExample\n\n## Test plan\nUnit tests\n",
                ],
                Some(&repo),
            );
            assert_eq!(text.code, 65, "{text:?}");
            assert!(text.stderr.contains("test_first.require_kinds"), "{text:?}");
            assert!(text.stderr.contains("bug"), "{text:?}");
        }
    }
}

#[test]
fn test_kind_requires_evidence_when_existing_assertions_change() {
    for (change, required) in [
        ("#[test]\nfn example() { assert_eq!(1, 2); }\n", true),
        (
            "#[test]\nfn example() { assert_eq!(1, 1); }\n#[test]\nfn added() { assert!(true); }\n",
            false,
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let repo = init_subject_repo(
            temp.path(),
            "repo",
            "https://github.com/example/project.git",
        );
        fs::create_dir(repo.join("tests")).unwrap();
        fs::write(
            repo.join("tests/example.rs"),
            "#[test]\nfn example() { assert_eq!(1, 1); }\n",
        )
        .unwrap();
        fs::write(
            repo.join(".forge-cli.toml"),
            "[test_first]\nrequire = true\n",
        )
        .unwrap();
        fs::write(repo.join("notes with spaces.md"), "before\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "baseline"]);
        install_local_remote(&repo, temp.path());
        git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        git(&repo, &["checkout", "-qb", "test/example"]);
        fs::write(repo.join("tests/example.rs"), change).unwrap();
        fs::write(repo.join("notes with spaces.md"), "after\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "test change"]);
        git(
            &repo,
            &["update-ref", "refs/remotes/origin/test/example", "HEAD"],
        );
        git(
            &repo,
            &["branch", "--set-upstream-to", "origin/test/example"],
        );
        let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
        for operation in ["create", "deliver"] {
            let out = run_forge_cli_in(
                &stub,
                &[
                    "--provider",
                    "github",
                    "--host",
                    "github.com",
                    "--repo",
                    "example/project",
                    "--format",
                    "json",
                    "--dry-run",
                    "pr",
                    operation,
                    "--kind",
                    "test",
                    "--base",
                    "main",
                    "--title",
                    "Change test",
                    "--body",
                    "## Summary\nExample\n\n## Test plan\nUnit tests\n",
                ],
                Some(&repo),
            );
            if operation == "deliver" {
                assert_eq!(out.code, 0, "{out:?}");
                let envelope = parse_envelope(&out.stdout);
                let verdict = envelope["data"]["local_preflight"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["rule"] == "test_first");
                assert_eq!(
                    verdict.is_some_and(|v| v["ok"] == false),
                    required,
                    "{out:?}"
                );
            } else {
                assert_eq!(out.code, if required { 65 } else { 0 }, "{out:?}");
                if required {
                    assert_eq!(
                        parse_envelope(&out.stdout)["error"]["code"],
                        "test_first_evidence_required"
                    );
                }
            }
        }
    }
}

#[test]
fn test_kind_fetches_current_base_and_refuses_failed_fetch() {
    let temp = TempDir::new().unwrap();
    let repo = init_subject_repo(
        temp.path(),
        "repo",
        "https://github.com/example/project.git",
    );
    let remote = temp.path().join("remote.git");
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    let rewrite = format!("url.file://{}.insteadOf", remote.display());
    git(
        &repo,
        &["config", &rewrite, "https://github.com/example/project.git"],
    );
    fs::write(
        repo.join(".forge-cli.toml"),
        "[test_first]\nrequire = true\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "baseline"]);
    git(&repo, &["push", "-q", "origin", "main"]);
    let old = git(&repo, &["rev-parse", "HEAD"]);
    fs::create_dir(repo.join("tests")).unwrap();
    fs::write(
        repo.join("tests/example.rs"),
        "#[test]\nfn example() { assert_eq!(1, 1); }\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "add test upstream"]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["checkout", "-qb", "test/example"]);
    fs::write(
        repo.join("tests/example.rs"),
        "#[test]\nfn example() { assert_eq!(1, 2); }\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "change test"]);
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/test/example", "HEAD"],
    );
    git(
        &repo,
        &["branch", "--set-upstream-to", "origin/test/example"],
    );
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/main", old.trim()],
    );
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
    let args = [
        "--provider",
        "github",
        "--host",
        "github.com",
        "--repo",
        "example/project",
        "--format",
        "json",
        "--dry-run",
        "pr",
        "create",
        "--kind",
        "test",
        "--base",
        "main",
        "--title",
        "Change test",
        "--body",
        "## Summary\nExample\n\n## Test plan\nUnit tests\n",
    ];
    let out = run_forge_cli_in(&stub, &args, Some(&repo));
    assert_eq!(out.code, 65, "{out:?}");
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "test_first_evidence_required"
    );
    git(&repo, &["config", "--unset", &rewrite]);
    let missing = format!(
        "url.file://{}.insteadOf",
        temp.path().join("missing.git").display()
    );
    git(
        &repo,
        &["config", &missing, "https://github.com/example/project.git"],
    );
    let out = run_forge_cli_in(&stub, &args, Some(&repo));
    assert_eq!(out.code, 65, "{out:?}");
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "test_changes_fetch_failed"
    );
}

#[test]
fn test_kind_uses_target_base_instead_of_fork_base() {
    let temp = TempDir::new().unwrap();
    let repo = init_subject_repo(temp.path(), "repo", "https://github.com/fork/project.git");
    let fork = temp.path().join("fork.git");
    let target = temp.path().join("target.git");
    git(temp.path(), &["init", "--bare", fork.to_str().unwrap()]);
    git(temp.path(), &["init", "--bare", target.to_str().unwrap()]);
    let fork_rewrite = format!("url.file://{}.insteadOf", fork.display());
    let target_rewrite = format!("url.file://{}.insteadOf", target.display());
    git(
        &repo,
        &[
            "config",
            &fork_rewrite,
            "https://github.com/fork/project.git",
        ],
    );
    git(
        &repo,
        &[
            "config",
            &target_rewrite,
            "https://github.com/example/project.git",
        ],
    );
    fs::write(
        repo.join(".forge-cli.toml"),
        "[test_first]\nrequire = true\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "baseline"]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(
        &repo,
        &[
            "config",
            "remote.origin.pushurl",
            "git@github.com:fork/project.git",
        ],
    );
    fs::create_dir(repo.join("tests")).unwrap();
    fs::write(
        repo.join("tests/example.rs"),
        "#[test]\nfn example() { assert!(true); }\n",
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "target adds test"]);
    git(
        &repo,
        &[
            "push",
            "-q",
            "https://github.com/example/project.git",
            "main",
        ],
    );
    git(
        temp.path(),
        &[
            "--git-dir",
            target.to_str().unwrap(),
            "symbolic-ref",
            "HEAD",
            "refs/heads/main",
        ],
    );
    git(&repo, &["checkout", "-qb", "test/example"]);
    fs::remove_file(repo.join("tests/example.rs")).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "remove test"]);
    git(
        &repo,
        &["update-ref", "refs/remotes/origin/test/example", "HEAD"],
    );
    git(
        &repo,
        &["branch", "--set-upstream-to", "origin/test/example"],
    );
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
    for operation in ["create", "deliver"] {
        let out = run_forge_cli_in(
            &stub,
            &[
                "--provider",
                "github",
                "--host",
                "github.com",
                "--repo",
                "example/project",
                "--format",
                "json",
                "--dry-run",
                "pr",
                operation,
                "--kind",
                "test",
                "--head",
                "fork:test/example",
                "--base",
                "main",
                "--title",
                "Change test",
                "--body",
                "## Summary\nExample\n\n## Test plan\nUnit tests\n",
            ],
            Some(&repo),
        );
        let envelope = parse_envelope(&out.stdout);
        if operation == "create" {
            assert_eq!(out.code, 65, "{out:?}");
            assert_eq!(envelope["error"]["code"], "test_first_evidence_required");
        } else {
            assert_eq!(out.code, 0, "{out:?}");
            let verdict = envelope["data"]["local_preflight"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["rule"] == "test_first")
                .unwrap();
            assert_eq!(verdict["code"], "test_first_evidence_required");
        }
    }
    let out = run_forge_cli_in(
        &stub,
        &[
            "--provider",
            "github",
            "--host",
            "github.com",
            "--repo",
            "example/project",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "deliver",
            "--kind",
            "test",
            "--head",
            "fork:test/example",
            "--title",
            "Change test",
            "--body",
            "## Summary\nExample\n\n## Test plan\nUnit tests\n",
        ],
        Some(&repo),
    );
    assert_eq!(out.code, 0, "{out:?}");
    let envelope = parse_envelope(&out.stdout);
    let verdict = envelope["data"]["local_preflight"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["rule"] == "test_first")
        .unwrap();
    assert_eq!(verdict["code"], "test_first_evidence_required");
}

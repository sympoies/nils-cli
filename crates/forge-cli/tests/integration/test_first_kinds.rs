//! Config parsing and create/deliver decisions for per-kind evidence policy.
use std::fs;

use forge_cli::config::ForgeConfig;
use nils_test_support::fixtures::init_subject_repo;
use nils_test_support::git::git;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::support::{StubEnv, parse_envelope, run_forge_cli_in};

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

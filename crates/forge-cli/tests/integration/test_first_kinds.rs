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
                        "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
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
                        "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
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
                    "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
                ],
                Some(&repo),
            );
            assert_eq!(text.code, 65, "{text:?}");
            assert!(text.stderr.contains("test_first.require_kinds"), "{text:?}");
            assert!(text.stderr.contains("bug"), "{text:?}");
        }
    }
}

const EXISTING_TEST: &str =
    "#[test]\nfn example() {\n    let delay = 1;\n    assert_eq!(delay, 1);\n}\n";
const ADDITIVE_TEST: &str = "#[test]\nfn example() {\n    let delay = 1;\n    assert_eq!(delay, 1);\n    assert!(delay > 0);\n}\n";

const NEW_TEST_WITH_SETUP: &str =
    "#[test]\nfn added() {\n    let value = crate_value();\n    assert_eq!(value, 1);\n}\n";

#[test]
fn test_kind_distinguishes_additive_coverage_from_repairs_and_production() {
    let existing = EXISTING_TEST;
    let additive = ADDITIVE_TEST;
    let new_test = NEW_TEST_WITH_SETUP;
    for (kinds, path, change, required) in [
        ("bug", "tests/example.rs", additive.to_owned(), false),
        ("test", "tests/example.rs", additive.to_owned(), false),
        ("test", "tests/new.rs", new_test.to_owned(), false),
        (
            "test",
            "crates/math/tests/new.rs",
            new_test.to_owned(),
            false,
        ),
        (
            "test",
            "tests/example.rs",
            format!("{existing}\n{NEW_TEST_WITH_SETUP}"),
            false,
        ),
        (
            "bug",
            "src/lib.rs",
            "pub fn crate_value() -> u8 { 2 }\n".to_owned(),
            true,
        ),
        (
            "test",
            "src/lib.rs",
            "pub fn crate_value() -> u8 { 2 }\n".to_owned(),
            true,
        ),
        (
            "bug",
            "tests/example.rs",
            existing.replace("delay = 1", "delay = 2"),
            true,
        ),
        (
            "bug",
            "tests/example.rs",
            existing.replace(
                "    let delay",
                "    std::thread::yield_now();\n    let delay",
            ),
            true,
        ),
        ("bug", "src/test_support.rs", new_test.to_owned(), true),
        (
            "test",
            "tests/fixtures/input.json",
            "{\"sample\": 2}\n".to_owned(),
            true,
        ),
        (
            "bug",
            "tests/example.rs",
            existing.replace("delay, 1", "delay, 2"),
            true,
        ),
        (
            "bug",
            "tests/new.rs",
            format!("#[ignore]\n{new_test}"),
            true,
        ),
    ] {
        check_test_kind_comparison(kinds, path, &change, required);
    }
}

#[test]
fn test_kind_requires_evidence_for_mixed_insertions() {
    let timing_repair = EXISTING_TEST.replace(
        "    let delay",
        "    std::thread::yield_now();\n    let delay",
    );
    for change in [
        format!("{timing_repair}\n{NEW_TEST_WITH_SETUP}"),
        ADDITIVE_TEST.replace(
            "    let delay",
            "    std::thread::yield_now();\n    let delay",
        ),
        EXISTING_TEST.replace(
            "    assert_eq!",
            "    assert!(true); return;\n    assert_eq!",
        ),
        EXISTING_TEST.replace(
            "    assert_eq!(delay, 1);",
            "    if delay > 1 {\n    assert_eq!(delay, 1);\n    }\n    assert!(delay > 0);",
        ),
    ] {
        check_test_kind_comparison("bug", "tests/example.rs", &change, true);
    }
}

#[test]
fn test_kind_requires_evidence_for_nested_production_paths() {
    check_test_kind_comparison(
        "bug",
        "src/domain/tests/runtime.rs",
        "pub fn original() -> u8 { 1 }\npub fn adjust(value: u8) -> u8 { assert!(value > 0); value }\n",
        true,
    );
}

fn check_test_kind_comparison(kinds: &str, path: &str, change: &str, required: bool) {
    let temp = TempDir::new().unwrap();
    let repo = init_subject_repo(
        temp.path(),
        "repo",
        "https://github.com/example/project.git",
    );
    fs::create_dir(repo.join("tests")).unwrap();
    fs::create_dir(repo.join("src")).unwrap();
    fs::create_dir(repo.join("tests/fixtures")).unwrap();
    fs::write(repo.join("tests/example.rs"), EXISTING_TEST).unwrap();
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn crate_value() -> u8 { 1 }\n",
    )
    .unwrap();
    if path == "src/domain/tests/runtime.rs" {
        fs::create_dir_all(repo.join("src/domain/tests")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "#[path = \"domain/tests/runtime.rs\"] pub mod runtime;\n",
        )
        .unwrap();
        fs::write(repo.join(path), "pub fn original() -> u8 { 1 }\n").unwrap();
    }
    fs::write(repo.join("tests/fixtures/input.json"), "{\"sample\": 1}\n").unwrap();
    fs::write(
        repo.join(".forge-cli.toml"),
        format!("[test_first]\nrequire = true\nrequire_kinds = [\"{kinds}\"]\n"),
    )
    .unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "baseline"]);
    install_local_remote(&repo, temp.path());
    git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    git(&repo, &["checkout", "-qb", "test/example"]);
    fs::write(repo.join("tests/example.rs"), ADDITIVE_TEST).unwrap();
    let changed_file = repo.join(path);
    fs::create_dir_all(changed_file.parent().unwrap()).unwrap();
    fs::write(changed_file, change).unwrap();
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
    if kinds == "test" && path == "tests/new.rs" {
        check_live_test_only_create_and_adoption(&repo);
    }
    let workdir = if kinds == "bug" && path == "src/lib.rs" {
        git(&repo, &["config", "diff.relative", "true"]);
        repo.join("tests")
    } else {
        repo.clone()
    };
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
                "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
            ],
            Some(&workdir),
        );
        if operation == "deliver" {
            assert_eq!(out.code, 0, "{kinds} {path} {operation}: {out:?}");
            let envelope = parse_envelope(&out.stdout);
            let verdict = envelope["data"]["local_preflight"]
                .as_array()
                .unwrap()
                .iter()
                .find(|v| v["rule"] == "test_first");
            assert_eq!(
                verdict.is_some_and(|v| v["ok"] == false),
                required,
                "{kinds} {path} {operation}: {out:?}"
            );
        } else {
            assert_eq!(
                out.code,
                if required { 65 } else { 0 },
                "{kinds} {path} {operation}: {out:?}"
            );
            if required {
                assert_eq!(
                    parse_envelope(&out.stdout)["error"]["code"],
                    "test_first_evidence_required"
                );
            }
        }
    }
}

fn check_live_test_only_create_and_adoption(repo: &std::path::Path) {
    let head = git(repo, &["rev-parse", "HEAD"]);
    let head = head.trim();
    let base = git(repo, &["rev-parse", "HEAD^"]);
    let base = base.trim();
    let valid_body =
        "## Summary\nAdd coverage.\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests.";
    let missing_declaration = "## Summary\nAdd coverage.\n\n## Test plan\nUnit tests.";
    let view = serde_json::json!({
        "number": 3,
        "url": "https://github.com/example/project/pull/3",
        "state": "OPEN",
        "headRefName": "test/example",
        "headRefOid": head,
        "baseRefName": "main",
        "title": "Add coverage",
        "body": valid_body,
        "isDraft": true,
    });
    let stub = StubEnv::new().gh_stub(
        r#"#!/bin/sh
case "$1 $2" in
  "auth status") printf '%s\n' '  ✓ Logged in to github.com account reviewer (keyring)' >&2 ;;
  "repo view") printf '%s\n' '{"name":"project","owner":{"login":"example"},"url":"https://github.com/example/project","defaultBranchRef":{"name":"main"},"mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":false}' ;;
  "pr list") printf '[%s]\n' "$PR_VIEW" ;;
  "pr create") printf '%s\n' 'https://github.com/example/project/pull/3' ;;
  "pr view") printf '%s\n' "$PR_VIEW" ;;
  *) exit 99 ;;
esac
"#,
    );
    let invoke = |operation, body, provider_view: &serde_json::Value, dry_run| {
        let mut args = vec![
            "--provider",
            "github",
            "--host",
            "github.com",
            "--repo",
            "example/project",
            "--format",
            "json",
            "pr",
            operation,
            "--kind",
            "test",
            "--base",
            "main",
            "--title",
            "Add coverage",
            "--body",
            body,
        ];
        if dry_run {
            args.push("--dry-run");
        }
        if operation == "deliver" {
            args.push("--no-merge");
        }
        super::support::run_forge_cli_in_with_env(
            &stub,
            &args,
            Some(repo),
            &[("PR_VIEW", &provider_view.to_string())],
        )
    };

    let created = invoke("create", valid_body, &view, false);
    assert_eq!(created.code, 0, "{created:?}");
    let created = parse_envelope(&created.stdout);
    assert_eq!(created["data"]["draft"], true);
    assert_eq!(created["data"]["head_sha"], head);

    let missing = invoke("create", missing_declaration, &view, false);
    assert_eq!(missing.code, 65, "{missing:?}");
    assert_eq!(
        parse_envelope(&missing.stdout)["error"]["code"],
        "test_changes_undeclared"
    );

    let missing = invoke("deliver", missing_declaration, &view, true);
    let preview = parse_envelope(&missing.stdout);
    let verdict = preview["data"]["local_preflight"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["rule"] == "body_test_changes")
        .unwrap();
    assert_eq!(verdict["code"], "test_changes_undeclared");

    let mut existing = view;
    existing["body"] = missing_declaration.into();
    let adopted = invoke("deliver", valid_body, &existing, false);
    assert_eq!(adopted.code, 65, "{adopted:?}");
    assert_eq!(
        parse_envelope(&adopted.stdout)["error"]["code"],
        "test_changes_undeclared"
    );

    existing["body"] = valid_body.into();
    existing["headRefOid"] = base.into();
    let adopted = invoke("deliver", valid_body, &existing, false);
    assert_eq!(adopted.code, 65, "{adopted:?}");
    assert_eq!(
        parse_envelope(&adopted.stdout)["error"]["code"],
        "test_first_evidence_provider_head_mismatch",
    );
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
        "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
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
                "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
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
            "## Summary\nExample\n\n## Test changes\nNone.\n\n## Test plan\nUnit tests\n",
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

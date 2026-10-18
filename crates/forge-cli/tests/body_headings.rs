//! Repository-configured section headings at the public CLI boundary.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;

use std::fs;
use std::path::Path;

use nils_test_support::git::{InitRepoOptions, git, init_repo_with};
use pretty_assertions::assert_eq;
use serde_json::Value;
use support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli_in};
use tempfile::TempDir;

const LOCALIZED_CONFIG: &str =
    "[body]\nsummary_heading = \"## 摘要\"\ntest_plan_heading = \"## 驗證\"\n";
const LOCALIZED_BODY: &str = "## 摘要\n\nChange description.\n\n## 驗證\n\nTests passed.\n";
const ENGLISH_BODY: &str = "## Summary\n\nChange description.\n\n## Test plan\n\nTests passed.\n";
const FORBIDDEN_BACKEND: &str = "#!/bin/sh\necho 'unexpected provider call' >&2\nexit 99\n";

fn repository(config: &str) -> TempDir {
    let repo = init_repo_with(InitRepoOptions::new().with_branch("feat/sample"));
    fs::write(repo.path().join(".forge-cli.toml"), config).unwrap();
    git(repo.path(), &["add", ".forge-cli.toml"]);
    git(repo.path(), &["commit", "-qm", "fixture configuration"]);
    git(
        repo.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/project.git",
        ],
    );
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    git(
        repo.path(),
        &["update-ref", "refs/remotes/origin/feat/sample", head.trim()],
    );
    git(
        repo.path(),
        &["branch", "--set-upstream-to=origin/feat/sample"],
    );
    repo
}

fn invoke(stub: &StubEnv, repo: &Path, command: &str, body: &str, dry_run: bool) -> CmdOutput {
    let mut args = vec!["--provider", "github", "--format", "json"];
    if dry_run {
        args.push("--dry-run");
    }
    args.extend(["pr", command]);
    if command == "edit" {
        args.push("7");
    } else {
        args.extend([
            "--kind",
            "feature",
            "--title",
            "feat: sample change",
            "--base",
            "main",
        ]);
    }
    if command == "deliver" {
        args.push("--no-merge");
    }
    args.extend(["--body", body]);
    run_forge_cli_in(stub, &args, Some(repo))
}

fn verdict<'a>(envelope: &'a Value, rule: &str) -> &'a Value {
    envelope["data"]["local_preflight"]
        .as_array()
        .unwrap()
        .iter()
        .find(|verdict| verdict["rule"] == rule)
        .unwrap_or_else(|| panic!("missing {rule} verdict: {envelope}"))
}

#[test]
fn delivery_preview_uses_repository_headings() {
    let repo = repository(LOCALIZED_CONFIG);
    let stub = StubEnv::new().gh_stub(FORBIDDEN_BACKEND);
    let out = invoke(&stub, repo.path(), "deliver", LOCALIZED_BODY, true);
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let envelope = parse_envelope(&out.stdout);
    for rule in ["body_summary", "body_test_plan"] {
        assert_eq!(verdict(&envelope, rule)["ok"], true, "{envelope}");
    }
}

#[test]
fn create_and_edit_use_repository_headings_for_both_providers() {
    let repo = repository(LOCALIZED_CONFIG);
    let stub = StubEnv::new()
        .gh_stub(FORBIDDEN_BACKEND)
        .glab_stub(FORBIDDEN_BACKEND);
    for provider in ["github", "gitlab"] {
        for command in ["create", "edit"] {
            let mut args = vec![
                "--provider",
                provider,
                "--repo",
                "example/project",
                "--format",
                "json",
                "--dry-run",
                "pr",
                command,
            ];
            if command == "edit" {
                args.push("7");
            } else {
                args.extend([
                    "--kind",
                    "feature",
                    "--title",
                    "feat: sample change",
                    "--base",
                    "main",
                ]);
            }
            args.extend(["--body", LOCALIZED_BODY]);
            let out = run_forge_cli_in(&stub, &args, Some(repo.path()));
            assert_eq!(
                out.code, 0,
                "{provider} {command}: {} {}",
                out.stdout, out.stderr
            );
        }
    }
}

#[test]
fn default_and_partial_configuration_preserve_english_fallbacks() {
    for (config, body) in [
        ("", ENGLISH_BODY),
        (
            "[body]\nsummary_heading = \"## 摘要\"\n",
            "## 摘要\nDescription.\n## Test plan\nTests.\n",
        ),
        (
            "[body]\ntest_plan_heading = \"## 驗證\"\n",
            "## Summary\nDescription.\n## 驗證\nTests.\n",
        ),
    ] {
        let repo = repository(config);
        let stub = StubEnv::new().gh_stub(FORBIDDEN_BACKEND);
        for command in ["create", "edit", "deliver"] {
            let out = invoke(&stub, repo.path(), command, body, true);
            assert_eq!(out.code, 0, "{command}: {} {}", out.stdout, out.stderr);
            if command == "deliver" {
                let envelope = parse_envelope(&out.stdout);
                for rule in ["body_summary", "body_test_plan"] {
                    assert_eq!(verdict(&envelope, rule)["ok"], true, "{envelope}");
                }
            }
        }
    }
}

#[test]
fn configuration_is_explicit_and_sections_must_be_non_empty_h2s() {
    for (config, body, missing) in [
        ("", LOCALIZED_BODY, "body_summary"),
        (LOCALIZED_CONFIG, ENGLISH_BODY, "body_summary"),
        (
            LOCALIZED_CONFIG,
            "## 摘要\n\n## 驗證\nTests.\n",
            "body_summary",
        ),
        (
            LOCALIZED_CONFIG,
            "## 摘要\nDescription.\n\n## 驗證\n",
            "body_test_plan",
        ),
        (
            LOCALIZED_CONFIG,
            "### 摘要\nDescription.\n## 驗證\nTests.\n",
            "body_summary",
        ),
    ] {
        let repo = repository(config);
        let stub = StubEnv::new().gh_stub(FORBIDDEN_BACKEND);
        for command in ["create", "edit", "deliver"] {
            let out = invoke(&stub, repo.path(), command, body, true);
            let envelope = parse_envelope(&out.stdout);
            if command == "deliver" {
                assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
                assert_eq!(verdict(&envelope, missing)["ok"], false, "{envelope}");
            } else {
                assert_eq!(out.code, 65, "{command}: {} {}", out.stdout, out.stderr);
                let code = envelope["error"]["code"].as_str().unwrap();
                assert!(code.starts_with("body_missing_"), "{envelope}");
            }
        }
    }
}

fn delivery_backend(repo: &Path, adopt: bool, body: &str) -> StubEnv {
    let head = git(repo, &["rev-parse", "HEAD"]);
    let view = serde_json::json!({
        "number": 7, "url": "https://github.com/example/project/pull/7",
        "state": "OPEN", "isDraft": true, "title": "feat: sample change",
        "headRefName": "feat/sample", "headRefOid": head.trim(),
        "headRepository": {"nameWithOwner": "example/project"},
        "baseRefName": "main", "mergeable": "MERGEABLE", "labels": [], "body": body
    });
    let list = if adopt {
        serde_json::json!([{
            "number": 7, "url": "https://github.com/example/project/pull/7",
            "state": "OPEN", "title": "feat: sample change", "headRefName": "feat/sample"
        }])
    } else {
        serde_json::json!([])
    };
    let stub = StubEnv::new().gh_stub(r#"#!/bin/sh
set -eu
printf '%s\n' "$1 $2" >> "$CALLS"
case "$1 $2" in
  "auth status")
    printf 'github.com\n  ✓ Logged in to github.com account fixture-user (keyring)\n  - Token scopes: repo\n' >&2 ;;
  "repo view")
    printf '%s\n' '{"name":"project","owner":{"login":"example"},"url":"https://github.com/example/project","defaultBranchRef":{"name":"main"},"squashMergeAllowed":true}' ;;
  "pr list") printf '%s\n' "$PR_LIST" ;;
  "pr view") printf '%s\n' "$PR_VIEW" ;;
  "pr create")
    if [ "$ADOPT" = true ]; then echo 'unexpected create during adoption' >&2; exit 99; fi
    printf '%s\n' 'https://github.com/example/project/pull/7' ;;
  "api graphql")
    case "$*" in
      *ForgeCheckRequirements*)
        printf '%s\n' '{"data":{"repository":{"ref":{"branchProtectionRule":{"requiredStatusChecks":[{"context":"test","app":null}]}}}}}' ;;
      *"authorAssociation body createdAt"*)
        printf '%s\n' '{"data":{"viewer":{"login":"fixture-user"},"repository":{"pullRequest":{"comments":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}' ;;
      *) echo 'unexpected GraphQL query' >&2; exit 99 ;;
    esac ;;
  "api repos/example/project/rules/branches/main?per_page=100")
    printf '%s\n' '[]' ;;
  "api repos/example/project/commits/"*/check-runs*)
    printf '%s\n' '{"total_count":1,"check_runs":[{"name":"test","status":"completed","conclusion":"success"}]}' ;;
  "api repos/example/project/commits/"*/status*)
    printf '%s\n' '{"total_count":0,"statuses":[]}' ;;
  *) echo "unexpected provider call: $*" >&2; exit 99 ;;
esac
"#);
    let calls = stub.tempdir.path().join("calls");
    stub.env("CALLS", calls.to_string_lossy())
        .env("PR_VIEW", view.to_string())
        .env("PR_LIST", list.to_string())
        .env("ADOPT", adopt.to_string())
}

#[test]
fn delivery_create_and_adopt_accept_configured_provider_bodies() {
    for adopt in [false, true] {
        let repo = repository(LOCALIZED_CONFIG);
        let stub = delivery_backend(repo.path(), adopt, LOCALIZED_BODY);
        // Adoption validates the provider body rather than caller create inputs.
        let input = if adopt {
            "unused create input"
        } else {
            LOCALIZED_BODY
        };
        let out = invoke(&stub, repo.path(), "deliver", input, false);
        assert_eq!(out.code, 0, "adopt={adopt}: {} {}", out.stdout, out.stderr);
        let envelope = parse_envelope(&out.stdout);
        assert_eq!(envelope["data"]["pr"]["merged"], false, "{envelope}");
        let calls = fs::read_to_string(stub.tempdir.path().join("calls")).unwrap();
        assert_eq!(calls.contains("pr create"), !adopt, "{calls}");
        assert!(
            calls.contains("api repos/example/project/commits/"),
            "{calls}"
        );
    }
}

#[test]
fn adoption_rejects_provider_body_missing_configured_sections() {
    let repo = repository(LOCALIZED_CONFIG);
    let stub = delivery_backend(repo.path(), true, ENGLISH_BODY);
    let out = invoke(&stub, repo.path(), "deliver", LOCALIZED_BODY, false);
    assert_eq!(out.code, 65, "{} {}", out.stdout, out.stderr);
    assert!(
        out.stdout.contains("body_missing_sections"),
        "{}",
        out.stdout
    );
    let calls = fs::read_to_string(stub.tempdir.path().join("calls")).unwrap();
    assert!(!calls.contains("pr create"), "{calls}");
    assert!(
        !calls.contains("api repos/example/project/commits/"),
        "{calls}"
    );
}

#[test]
fn repository_overrides_global_headings_from_a_nested_working_directory() {
    let repo = repository("[body]\nsummary_heading = \"## 摘要\"\n");
    let nested = repo.path().join("nested");
    fs::create_dir(&nested).unwrap();
    let stub = StubEnv::new().gh_stub(FORBIDDEN_BACKEND);
    let global_dir = stub.tempdir.path().join("xdg-config/forge-cli");
    fs::create_dir_all(&global_dir).unwrap();
    fs::write(
        global_dir.join("config.toml"),
        "[body]\nsummary_heading = \"## Global summary\"\ntest_plan_heading = \"## Global tests\"\n",
    )
    .unwrap();
    let body = "## 摘要\nDescription.\n## Global tests\nTests.\n";
    for command in ["create", "edit", "deliver"] {
        let out = invoke(&stub, &nested, command, body, true);
        assert_eq!(out.code, 0, "{command}: {} {}", out.stdout, out.stderr);
        if command == "deliver" {
            let envelope = parse_envelope(&out.stdout);
            for rule in ["body_summary", "body_test_plan"] {
                assert_eq!(verdict(&envelope, rule)["ok"], true, "{envelope}");
            }
        }
    }
}

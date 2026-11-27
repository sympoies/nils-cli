//! Exercise declared-scope admission through the CLI against committed comparisons.
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use nils_test_support::fixtures::init_subject_repo;
use nils_test_support::git::git;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::TempDir;

use super::support::{StubEnv, parse_envelope, run_forge_cli_in_with_env};

const REPO_URL: &str = "https://github.com/example/project.git";
const SCRIPT_TEST: &str = "scripts/test/test_sample.py";
const PACKAGE_TEST: &str = "packages/widget/test/test_sample.py";
const FIXTURE_TEST: &str = "packages/widget/test/test_fixture.py";
const FIXTURE: &str = "packages/widget/test/fixtures/expected.py";
const PRODUCT: &str = "sample.py";
const BASE_CONFIG: &str = "[test_first]\nrequire = true\nrequire_kinds = [\"bug\"]\n";
const SCOPES: &str = "test_paths = [\"scripts/test/\", \"packages/*/test/\"]\n";
const BASE_TEST: &str =
    "from sample import response\n\n\ndef test_existing():\n    assert response() > 0\n";
const ADDITION: &str =
    "\n\ndef test_exact_response():\n    assert response() == 1, 'unexpected response'\n";
const BODY: &str = "## Summary\nCheck a sample response.\n\n## Test changes\nNone.\n\n## Test plan\nRun the sample tests.";

const BROWSER_TEST: &str = "scripts/test/idle-link.browser.test.ts";
const EDGE_TEST: &str = "packages/edge/test/crossOwnerSessionAccess.test.ts";
const WAIT_TEST: &str = "packages/ui/test/terminalPaperContrast.browser.test.ts";
const VISIBILITY: &str = "packages/edge/src/visibility.cjs";
const BROWSER_BASE: &str = include_str!("../fixtures/test_scopes/idle-link.base.ts");
const BROWSER_ADDITION: &str = include_str!("../fixtures/test_scopes/idle-link.additive.ts");
const BROWSER_BODY: &str = concat!(
    "## Summary\nCheck history polling after visibility changes.\n\n",
    "## Test changes\nNone.\n\n## Test plan\nRun the visibility tests.\n\n",
    "## Test-only evidence\n",
    "- Base command `node --input-type=commonjs - < scripts/test/idle-link.browser.test.ts` passed on the base.\n",
    "- Added coverage command `node --input-type=commonjs - < scripts/test/idle-link.browser.test.ts` passed with the added assertions.\n",
    "- Mutation command `node --input-type=commonjs - < scripts/test/idle-link.browser.test.ts` failed the expected assertion when hidden polling was enabled.\n",
);

#[test]
fn declared_scopes_admit_passing_additive_coverage() {
    let temp = TempDir::new().unwrap();
    let repo = comparison(&temp, SCOPES);
    run_browser_sample(&repo, true);
    write(&repo, BROWSER_TEST, BROWSER_ADDITION);
    run_browser_sample(&repo, true);
    write(
        &repo,
        VISIBILITY,
        "exports.pollHistory = (_visible, read) => read('/history/items');\n",
    );
    run_browser_sample(&repo, false);
    git(&repo, &["restore", VISIBILITY]);
    commit_comparison(&repo);
    check_routes(&repo, false, "additive browser fixture", BROWSER_BODY);
    check_routes(&repo, true, "browser fixture without review evidence", BODY);

    let temp = TempDir::new().unwrap();
    let repo = comparison(&temp, SCOPES);
    run_sample(&repo, FIXTURE_TEST, "test_minimum", true);
    for path in [SCRIPT_TEST, PACKAGE_TEST] {
        run_sample(&repo, path, "test_existing", true);
        write(&repo, path, &format!("{BASE_TEST}{ADDITION}"));
        run_sample(&repo, path, "test_exact_response", true);
    }
    write(&repo, PRODUCT, "def response():\n    return 2\n");
    for path in [SCRIPT_TEST, PACKAGE_TEST] {
        run_sample(&repo, path, "test_exact_response", false);
    }
    git(&repo, &["restore", PRODUCT]);
    commit_comparison(&repo);
    check_routes(&repo, false, "pure addition", BODY);
}

#[test]
fn declared_scopes_keep_behavior_change_evidence() {
    let wrapped = BROWSER_ADDITION.replace(
        "  assert.ok(fixture.outboxReads > 0, 'visible view polls the outbox');\n",
        "  if (visible) {\n  assert.ok(fixture.outboxReads > 0, 'visible view polls the outbox');\n  }\n",
    );
    for (case, path, change) in [
        (
            "edited assertion",
            EDGE_TEST,
            "const assert = require('node:assert/strict');\nconst f = { calls: [] };\nconst testedCalls = () => f.calls.filter(call => call !== 'receipt');\nconst closed = testedCalls().length;\nassert.equal(testedCalls().length, closed, 'no refused call');\n",
        ),
        (
            "deleted assertion",
            PACKAGE_TEST,
            "from sample import response\n\n\ndef test_existing():\n    pass\n",
        ),
        (
            "new skip",
            SCRIPT_TEST,
            "\n\ndef test_ignored():\n    import pytest\n    pytest.skip('sample')\n",
        ),
        ("weakened fixture", FIXTURE, "MINIMUM = 0\n"),
        (
            "race repair",
            PACKAGE_TEST,
            "from sample import response\n\n\ndef test_existing():\n    import time\n    time.sleep(0.01)\n    assert response() > 0\n",
        ),
        (
            "fixture wait repair",
            WAIT_TEST,
            "test('renders cells', () => {\n  render(waitForCells);\n  assert.ok(cellsReady());\n});\n",
        ),
        ("wrapped existing assertion", BROWSER_TEST, &wrapped),
        (
            "mixed product",
            VISIBILITY,
            "exports.pollHistory = (_visible, read) => read('/history/items');\n",
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let repo = comparison(&temp, SCOPES);
        write(&repo, SCRIPT_TEST, &format!("{BASE_TEST}{ADDITION}"));
        let change = if case == "new skip" {
            format!("{BASE_TEST}{ADDITION}{change}")
        } else {
            change.to_string()
        };
        write(&repo, path, &change);
        commit_comparison(&repo);
        check_routes(&repo, true, case, BROWSER_BODY);
    }
}

#[test]
fn declared_scopes_require_base_configuration() {
    for (base_scopes, path, head_config) in [
        ("", SCRIPT_TEST, false),
        ("", SCRIPT_TEST, true),
        (SCOPES, "packages/widget/nested/test/test_sample.py", false),
        (SCOPES, "scripts/test-support/test_sample.py", false),
        (
            "test_paths = [\"scripts/test/\", \"../src/\"]\n",
            SCRIPT_TEST,
            false,
        ),
    ] {
        let temp = TempDir::new().unwrap();
        let repo = comparison(&temp, base_scopes);
        write(&repo, path, &format!("{BASE_TEST}{ADDITION}"));
        commit_comparison(&repo);
        if head_config {
            // Keep the selected PR diff pure while the checkout contains a declaration.
            git(&repo, &["checkout", "-qb", "test/declare-scope"]);
            write(&repo, ".forge-cli.toml", &format!("{BASE_CONFIG}{SCOPES}"));
            git(&repo, &["add", ".forge-cli.toml"]);
            git(&repo, &["commit", "-qm", "declare scope on the head"]);
        }
        check_routes(&repo, true, path, BODY);
    }
}

fn comparison(temp: &TempDir, scopes: &str) -> std::path::PathBuf {
    let repo = init_subject_repo(temp.path(), "repo", REPO_URL);
    write(&repo, ".forge-cli.toml", &format!("{BASE_CONFIG}{scopes}"));
    write(&repo, PRODUCT, "def response():\n    return 1\n");
    write(
        &repo,
        VISIBILITY,
        "exports.pollHistory = (visible, read) => { if (visible) read('/history/items'); };\n",
    );
    write(&repo, BROWSER_TEST, BROWSER_BASE);
    write(
        &repo,
        EDGE_TEST,
        "const assert = require('node:assert/strict');\nconst f = { calls: [] };\nconst closed = f.calls.length;\nassert.equal(f.calls.length, closed, 'no refused call');\n",
    );
    write(
        &repo,
        WAIT_TEST,
        "test('renders cells', () => {\n  render(() => setTimeout(measure, 300));\n  assert.ok(cellsReady());\n});\n",
    );
    write(&repo, SCRIPT_TEST, BASE_TEST);
    write(&repo, PACKAGE_TEST, BASE_TEST);
    write(&repo, FIXTURE, "MINIMUM = 1\n");
    write(
        &repo,
        FIXTURE_TEST,
        "from pathlib import Path\n\nfrom sample import response\n\n\ndef test_minimum():\n    path = Path(__file__).with_name('fixtures') / 'expected.py'\n    import runpy\n    assert response() >= runpy.run_path(str(path))['MINIMUM']\n",
    );
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "sample baseline"]);
    let remote = temp.path().join("remote.git");
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    let rewrite = format!("url.file://{}.insteadOf", remote.display());
    git(&repo, &["config", &rewrite, REPO_URL]);
    git(&repo, &["push", "-q", "origin", "main"]);
    git(&repo, &["checkout", "-qb", "test/example"]);
    repo
}

fn write(repo: &Path, path: &str, contents: &str) {
    let path = repo.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn commit_comparison(repo: &Path) {
    git(repo, &["add", "."]);
    git(repo, &["commit", "-qm", "change sample tests"]);
    git(repo, &["push", "-qu", "origin", "test/example"]);
}

fn run_sample(repo: &Path, path: &str, test: &str, passes: bool) {
    let output = Command::new("python3")
        .current_dir(repo)
        .args([
            "-B",
            "-E",
            "-S",
            "-c",
            "import runpy, sys; runpy.run_path(sys.argv[1])[sys.argv[2]]()",
            path,
            test,
        ])
        .output()
        .expect("spawn sample test");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(if passes { 0 } else { 1 }),
        "{stderr}"
    );
    if !passes {
        assert!(
            stderr.contains("AssertionError: unexpected response"),
            "{stderr}"
        );
    }
}

fn run_browser_sample(repo: &Path, passes: bool) {
    let input = fs::File::open(repo.join(BROWSER_TEST)).unwrap();
    let output = nils_test_support::node::command()
        .current_dir(repo)
        .env_remove("NODE_OPTIONS")
        .args(["--input-type=commonjs", "-"])
        .stdin(Stdio::from(input))
        .output()
        .expect("spawn browser-shaped fixture");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(if passes { 0 } else { 1 }),
        "{stderr}"
    );
    if !passes {
        assert!(
            stderr
                .contains("AssertionError [ERR_ASSERTION]: hidden view must stop polling history"),
            "{stderr}"
        );
    }
}

fn check_routes(repo: &Path, required: bool, case: &str, body: &str) {
    let stub = StubEnv::new().gh_gate_stub(
        r#"#!/bin/sh
case "$1 $2" in
  "auth status") printf '%s\n' '  ✓ Logged in to github.com account reviewer (keyring)' >&2 ;;
  "repo view") printf '%s\n' '{"name":"project","owner":{"login":"example"},"url":"https://github.com/example/project","defaultBranchRef":{"name":"main"},"mergeCommitAllowed":false,"squashMergeAllowed":true,"rebaseMergeAllowed":false}' ;;
  "pr list") printf '%s\n' "$PR_LIST" ;;
  "pr create") printf '%s\n' 'https://github.com/example/project/pull/3' ;;
  "pr view") printf '%s\n' "$PR_VIEW" ;;
  "pr checks") printf '%s\n' "$PR_CHECKS" ;;
  "api graphql")
    case "$*" in
      *"authorAssociation body createdAt"*)
        printf '%s\n' '{"data":{"viewer":{"login":"reviewer"},"repository":{"pullRequest":{"comments":{"nodes":[],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}'
        ;;
      *) printf 'Unexpected fake-provider request: %s\n' "$*" >&2; exit 99 ;;
    esac
    ;;
  *) printf 'Unexpected fake-provider request: %s\n' "$*" >&2; exit 99 ;;
esac
"#,
    );
    let head = git(repo, &["rev-parse", "test/example"]);
    let view = json!({
        "number": 3,
        "url": "https://github.com/example/project/pull/3",
        "state": "OPEN",
        "headRefName": "test/example",
        "headRefOid": head.trim(),
        "baseRefName": "main",
        "title": "Check sample response",
        "body": body,
        "isDraft": true,
    });
    for (operation, dry_run, adopt) in [
        ("create", true, false),
        ("deliver", true, false),
        ("create", false, false),
        ("deliver", false, false),
        ("deliver", false, true),
    ] {
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
            "--head",
            "test/example",
            "--title",
            "Check sample response",
            "--body",
            body,
        ];
        if dry_run {
            args.push("--dry-run");
        }
        if operation == "deliver" {
            args.push("--no-merge");
        }
        let list = if adopt { json!([view]) } else { json!([]) };
        let out = run_forge_cli_in_with_env(
            &stub,
            &args,
            Some(repo),
            &[
                ("PR_VIEW", &view.to_string()),
                ("PR_LIST", &list.to_string()),
                (
                    "PR_CHECKS",
                    include_str!("../fixtures/github/pr_checks/all_success.json"),
                ),
            ],
        );
        let envelope = parse_envelope(&out.stdout);
        let code = if operation == "deliver" && dry_run {
            assert_eq!(out.code, 0, "{case}: {out:?}");
            envelope["data"]["local_preflight"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["rule"] == "test_first" && item["ok"] == false)
                .and_then(|item| item["code"].as_str())
        } else {
            assert_eq!(
                out.code,
                if required { 65 } else { 0 },
                "{case}, {operation}, dry_run={dry_run}, adopt={adopt}: {out:?}"
            );
            if adopt && !required {
                assert!(
                    envelope["data"]["steps"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|step| step["step"] == "adopt" && step["ok"] == true),
                    "{out:?}"
                );
            }
            envelope["error"]["code"].as_str()
        };
        assert_eq!(
            code,
            required.then_some("test_first_evidence_required"),
            "{case}, {operation}: {out:?}"
        );
    }
}

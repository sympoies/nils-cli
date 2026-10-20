//! Publication uses portable display references while canonical evidence stays immutable.
use std::{fs, path::Path};

use agent_workflow_primitives::test_first_evidence::run_with_args;
use nils_test_support::fixtures::SubjectBoundEvidenceFixture;
use nils_test_support::git::git;
use pretty_assertions::{assert_eq, assert_ne};
use sha2::{Digest, Sha256};

use super::support::{StubEnv, parse_envelope, run_forge_cli_in};

const BODY: &str = "## Summary\n\nSynthetic change.\n\n## Test plan\n\nFocused checks passed.\n";

fn evidence(dir: &Path, command: &str, args: &[&str]) {
    let mut argv = vec![
        "test-first-evidence".to_owned(),
        command.to_owned(),
        "--out".to_owned(),
        dir.to_str().unwrap().to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));
    assert_eq!(run_with_args(argv), 0);
}

fn fixture() -> SubjectBoundEvidenceFixture {
    fixture_with_references(&[
        "/home/fixture/out/triage.log",
        "/Users/fixture/out/proof.json",
    ])
}

fn fixture_with_references(references: &[&str]) -> SubjectBoundEvidenceFixture {
    let fixture = SubjectBoundEvidenceFixture::new(
        "https://github.com/example/project.git",
        &[(".forge-cli.toml", "[test_first]\nrequire = true\n")],
        |phase, repo, dir| {
            run_with_args([
                "test-first-evidence",
                phase,
                "--out",
                dir.to_str().unwrap(),
                "--project-path",
                repo.to_str().unwrap(),
            ])
        },
    );
    git(&fixture.repo, &["checkout", "-qb", "fix/portable"]);
    git(
        &fixture.repo,
        &["update-ref", "refs/remotes/origin/fix/portable", "HEAD"],
    );
    git(
        &fixture.repo,
        &["branch", "--set-upstream-to=origin/fix/portable"],
    );
    evidence(
        &fixture.evidence,
        "record-final",
        &[
            "--command",
            "synthetic affected",
            "--scope",
            "affected-suite",
            "--status",
            "fail",
        ],
    );
    evidence(
        &fixture.evidence,
        "record-final",
        &[
            "--command",
            "synthetic full",
            "--scope",
            "full",
            "--status",
            "pass",
        ],
    );
    let mut resolution = vec![
        "--command",
        "synthetic affected",
        "--scope",
        "affected-suite",
        "--attempt",
        "1",
        "--passing-command",
        "synthetic full",
        "--passing-scope",
        "full",
        "--passing-attempt",
        "1",
        "--reason",
        "synthetic passing validation covers the same source",
    ];
    for reference in references {
        resolution.extend(["--investigation", reference]);
    }
    evidence(&fixture.evidence, "resolve-final", &resolution);
    fixture
}

fn create_args<'a>(operation: &'a str, evidence: &'a str) -> Vec<&'a str> {
    vec![
        "--provider",
        "github",
        "--format",
        "json",
        "pr",
        operation,
        "--kind",
        "bug",
        "--title",
        "fix: portable investigation references",
        "--body",
        BODY,
        "--base",
        "main",
        "--head",
        "fix/portable",
        "--test-first-evidence",
        evidence,
    ]
}

#[test]
fn create_and_deliver_previews_accept_immutable_home_references() {
    let fixture = fixture();
    let file = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
    for operation in ["create", "deliver"] {
        let out = fixture.evidence.to_str().unwrap();
        let mut args = create_args(operation, out);
        args.insert(0, "--dry-run");
        if operation == "deliver" {
            args.push("--no-merge");
        }
        let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
        assert_eq!(result.code, 0, "{}\n{}", result.stdout, result.stderr);
        if operation == "deliver" {
            let envelope = parse_envelope(&result.stdout);
            let verdict = envelope["data"]["local_preflight"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["rule"] == "test_first")
                .unwrap();
            assert_eq!(verdict["ok"], true);
        }
    }
    assert_eq!(fs::read(&file).unwrap(), before);
}

fn publication_stub(stub: StubEnv, capture: &Path) -> StubEnv {
    stub.env("BODY_CAPTURE", capture.to_str().unwrap()).gh_stub(r#"#!/bin/sh
set -eu
case "$1 $2" in
  "auth status") printf 'github.com\n  Logged in to github.com account fixture\n' >&2 ;;
  "repo view") printf '%s\n' '{"name":"project","owner":{"login":"example"},"url":"https://github.com/example/project","defaultBranchRef":{"name":"main"}}' ;;
  "pr list") printf '[]\n' ;;
  "pr create")
    previous=
    for arg in "$@"; do
      if [ "$previous" = "--body-file" ]; then cp "$arg" "$BODY_CAPTURE"; fi
      previous="$arg"
    done
    printf 'fixture publication stop after body capture\n' >&2
    exit 97 ;;
  *) printf 'unexpected fixture backend call\n' >&2; exit 98 ;;
esac
"#)
}

#[test]
fn macro_enrichment_reaches_stub_with_portable_final_summary() {
    let fixture = fixture();
    let file = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("published-body.md");
    let stub = publication_stub(stub, &capture);
    let out = fixture.evidence.to_str().unwrap();
    let mut args = create_args("deliver", out);
    args.push("--no-merge");
    let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
    assert!(
        capture.is_file(),
        "macro failed before publication: {}\n{}",
        result.stdout,
        result.stderr
    );
    assert_ne!(
        result.code, 0,
        "the fixture deliberately stops after capture"
    );
    let body = fs::read_to_string(capture).unwrap();
    assert!(body.starts_with(BODY));
    assert!(body.contains("## Final validation dispositions"));
    assert!(body.contains("FAIL: `synthetic affected`"));
    assert!(body.contains("Resolved by `synthetic full`"));
    assert!(body.contains("$HOME/out/triage.log; $HOME/out/proof.json"));
    assert!(nils_common::provider_payload::scan_local_paths(&body).is_empty());
    forge_cli::validations::no_local_path(&body, "body").expect("normal publication guard");
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
    assert!(
        String::from_utf8(before)
            .unwrap()
            .contains("/home/fixture/out/triage.log")
    );
    assert_eq!(git(&fixture.repo, &["status", "--porcelain"]), "");
}

#[test]
fn nested_home_tail_keeps_the_existing_publication_guard_refusal() {
    let fixture = fixture_with_references(&["/home/fixture/archive/home/example/report.log"]);
    let file = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("published-body.md");
    let stub = publication_stub(stub, &capture);
    let out = fixture.evidence.to_str().unwrap();
    let mut args = create_args("deliver", out);
    args.push("--no-merge");
    let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
    assert_eq!(result.code, 65, "{}\n{}", result.stdout, result.stderr);
    assert_eq!(
        parse_envelope(&result.stdout)["error"]["code"],
        "local_path_present"
    );
    assert!(
        !capture.exists(),
        "the guard must not publish a corrupted reference"
    );
    assert_eq!(fs::read(file).unwrap(), before);
}

#[test]
fn create_and_deliver_refuse_uri_home_components_before_publication() {
    let fixture = fixture_with_references(&[
        "https://example.invalid/home/fixture/report",
        "https://example.invalid/Users/fixture/report",
    ]);
    let file = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let stub = StubEnv::new();
    let capture = stub.tempdir.path().join("published-body.md");
    let stub = publication_stub(stub, &capture);
    for operation in ["create", "deliver"] {
        let out = fixture.evidence.to_str().unwrap();
        let mut args = create_args(operation, out);
        if operation == "deliver" {
            args.push("--no-merge");
        }
        let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
        assert_eq!(result.code, 65, "{}\n{}", result.stdout, result.stderr);
        assert_eq!(
            parse_envelope(&result.stdout)["error"]["code"],
            "local_path_present"
        );
        assert!(!capture.exists(), "URI must not be corrupted and published");
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
    }
}

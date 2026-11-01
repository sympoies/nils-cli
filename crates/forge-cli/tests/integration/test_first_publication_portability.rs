//! Publication exports typed disposition metadata while canonical evidence stays immutable.
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
    fixture_with_commands(references, "synthetic affected", "synthetic full")
}

fn fixture_with_commands(
    references: &[&str],
    failed_command: &str,
    passing_command: &str,
) -> SubjectBoundEvidenceFixture {
    fixture_with_disposition(
        references,
        failed_command,
        passing_command,
        "synthetic validation result",
        "synthetic passing validation covers the same source",
    )
}

fn fixture_with_disposition(
    references: &[&str],
    failed_command: &str,
    passing_command: &str,
    summary: &str,
    reason: &str,
) -> SubjectBoundEvidenceFixture {
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
            failed_command,
            "--scope",
            "affected-suite",
            "--status",
            "fail",
            "--summary",
            summary,
        ],
    );
    evidence(
        &fixture.evidence,
        "record-final",
        &[
            "--command",
            passing_command,
            "--scope",
            "full",
            "--status",
            "pass",
            "--summary",
            summary,
        ],
    );
    let mut resolution = vec![
        "--command",
        failed_command,
        "--scope",
        "affected-suite",
        "--attempt",
        "1",
        "--passing-command",
        passing_command,
        "--passing-scope",
        "full",
        "--passing-attempt",
        "1",
        "--reason",
        reason,
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
        "fix: private evidence disposition export",
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
fn create_and_deliver_keep_local_disposition_details_in_immutable_evidence() {
    let summary = "validation report saved at /tmp/evidence/validation.log";
    let reason = "same target confirmed in /var/tmp/evidence/investigation.json";
    let investigation = "/opt/evidence/analysis.log";
    let failed = "cargo test affected > /tmp/evidence/affected.log";
    let passing = "bash /opt/evidence/checks.sh";
    let fixture = fixture_with_disposition(&[investigation], failed, passing, summary, reason);
    let file = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    for operation in ["create", "deliver"] {
        let stub = StubEnv::new();
        let capture = stub.tempdir.path().join("published-body.md");
        let stub = publication_stub(stub, &capture);
        let mut args = create_args(operation, fixture.evidence.to_str().unwrap());
        if operation == "deliver" {
            args.push("--no-merge");
        }
        let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
        assert!(
            capture.is_file(),
            "must reach the publication owner: {}\n{}",
            result.stdout,
            result.stderr
        );
        assert_ne!(result.code, 0, "stub stops after capturing the actual body");
        let body = fs::read_to_string(capture).unwrap();
        assert!(body.starts_with(BODY));
        assert!(body.contains(&format!(
                "FAIL: command `sha256:{}` (scope affected-suite, attempt 1).",
                Sha256::digest(failed.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )));
        assert!(body.contains(&format!(
                "PASS: command `sha256:{}` (scope full, attempt 1).",
                Sha256::digest(passing.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )));
        assert!(body.contains(&format!(
                "Resolved by command `sha256:{}` (scope full, attempt 1).",
                Sha256::digest(passing.as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )));
        for private_detail in [failed, passing, summary, reason, investigation] {
            assert!(
                !body.contains(private_detail),
                "{operation} must not export local-only disposition details"
            );
        }
        assert_eq!(fs::read(&file).unwrap(), before);
    }
    let retained = String::from_utf8(before).unwrap();
    for private_detail in [failed, passing, summary, reason, investigation] {
        assert!(retained.contains(private_detail));
    }
    assert_eq!(git(&fixture.repo, &["status", "--porcelain"]), "");
}

#[test]
fn draft_preview_admits_a_resolved_validation_chain_without_rewriting_history() {
    let fixture = fixture_with_references(&["synthetic investigation"]);
    for (command, status) in [("synthetic full", "fail"), ("synthetic final", "pass")] {
        evidence(
            &fixture.evidence,
            "record-final",
            &["--command", command, "--scope", "full", "--status", status],
        );
    }
    evidence(
        &fixture.evidence,
        "resolve-final",
        &[
            "--command",
            "synthetic full",
            "--scope",
            "full",
            "--attempt",
            "2",
            "--passing-command",
            "synthetic final",
            "--passing-scope",
            "full",
            "--passing-attempt",
            "1",
            "--reason",
            "same source and covered target",
            "--investigation",
            "later failure investigated",
        ],
    );
    let path = fixture.evidence.join("test-first-evidence.json");
    let before = fs::read(&path).unwrap();
    let stub = StubEnv::new().gh_stub("#!/bin/sh\nexit 99\n");
    let mut args = create_args("create", fixture.evidence.to_str().unwrap());
    args.insert(0, "--dry-run");
    let result = run_forge_cli_in(&stub, &args, Some(&fixture.repo));
    assert_eq!(result.code, 0, "{}\n{}", result.stdout, result.stderr);
    assert_eq!(
        fs::read(path).unwrap(),
        before,
        "draft preview preserves all evidence"
    );
    assert_eq!(git(&fixture.repo, &["status", "--porcelain"]), "");
}

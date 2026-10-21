use std::{fs, path::Path};

use agent_workflow_primitives::test_first_evidence::{
    final_resolution_summary, run_with_args, verify_dir,
};
use nils_common::provider_payload::{scan_local_paths, validate_no_local_paths};
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};

fn record(dir: &Path, command: &str, args: &[&str]) {
    let mut argv = vec![
        "test-first-evidence".to_owned(),
        command.to_owned(),
        "--out".to_owned(),
        dir.to_str().unwrap().to_owned(),
        "--format".to_owned(),
        "json".to_owned(),
    ];
    argv.extend(args.iter().map(|arg| (*arg).to_owned()));
    assert_eq!(run_with_args(argv), 0, "fixture command: {command}");
}

fn resolved_fixture(dir: &Path, investigations: &[&str], reason: &str) {
    resolved_fixture_with_commands(
        dir,
        investigations,
        reason,
        &["synthetic affected-a", "synthetic affected-b"],
        "synthetic canonical",
    );
}

fn resolved_fixture_with_commands(
    dir: &Path,
    investigations: &[&str],
    reason: &str,
    failed_commands: &[&str],
    passing_command: &str,
) {
    record(
        dir,
        "init",
        &[
            "--classification",
            "bug-fix",
            "--changed-behavior",
            "final investigation references are portable at publication",
        ],
    );
    record(
        dir,
        "record-impact",
        &["--none", "--reason", "synthetic fixture"],
    );
    record(
        dir,
        "record-failing",
        &[
            "--command",
            "synthetic regression",
            "--exit-code",
            "1",
            "--summary",
            "publication rejects an immutable reference",
            "--expected-failure",
            "portable investigation reference",
            "--observed-failure",
            "machine-local reference in exported summary",
        ],
    );
    for &command in failed_commands {
        record(
            dir,
            "record-final",
            &[
                "--command",
                command,
                "--scope",
                "affected-suite",
                "--status",
                "fail",
            ],
        );
    }
    record(
        dir,
        "record-final",
        &[
            "--command",
            passing_command,
            "--scope",
            "full",
            "--status",
            "pass",
            "--summary",
            "synthetic passing final validation",
        ],
    );
    record(
        dir,
        "record-final",
        &[
            "--command",
            "synthetic focused",
            "--scope",
            "focused",
            "--status",
            "pass",
        ],
    );
    for &command in failed_commands {
        let mut args = vec![
            "--command",
            command,
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
        for reference in investigations {
            args.extend(["--investigation", reference]);
        }
        record(dir, "resolve-final", &args);
    }
    record(dir, "record-gap", &["--none"]);
    assert!(verify_dir(dir).unwrap().complete);
}

#[test]
fn immutable_home_investigations_export_as_portable_provider_text() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("evidence");
    let references = [
        "/home/fixture/out/triage.log",
        "/Users/fixture/out/report.json",
        "docs/repro.md",
        "https://example.invalid/artifacts/report",
    ];
    resolved_fixture(&dir, &references, "same source and covered target");
    let file = dir.join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let original = verify_dir(&dir).unwrap().record;
    let resolutions = serde_json::to_value(&original.final_validations).unwrap();
    let summary = final_resolution_summary(&original);
    let body = format!("## Summary\nSynthetic change\n\n## Test plan\nFocused checks\n\n{summary}");

    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
    assert_eq!(
        serde_json::to_value(&verify_dir(&dir).unwrap().record.final_validations).unwrap(),
        resolutions,
    );
    assert_eq!(resolutions.as_array().unwrap().len(), 4);
    assert_eq!(
        summary.matches("Resolved by `synthetic canonical`").count(),
        2
    );
    assert!(
        scan_local_paths(&body).is_empty(),
        "export contains home-absolute investigation references"
    );
    validate_no_local_paths(&body, "body")
        .expect("normal publication guard accepts exported summary");
    assert_eq!(summary.matches("$HOME/out/triage.log").count(), 2);
    assert_eq!(summary.matches("$HOME/out/report.json").count(), 2);
    assert_eq!(summary.matches("docs/repro.md").count(), 2);
    assert_eq!(
        summary
            .matches("https://example.invalid/artifacts/report")
            .count(),
        2
    );
    assert!(String::from_utf8(before).unwrap().contains(references[0]));
}

#[test]
fn ordinary_investigation_text_and_final_history_keep_their_presentation() {
    let temp = tempfile::tempdir().unwrap();
    resolved_fixture(
        temp.path(),
        &[
            "docs/repro.md",
            "https://example.invalid/report",
            "$HOME/out/proof.log",
        ],
        "same source and covered target",
    );
    let summary = final_resolution_summary(&verify_dir(temp.path()).unwrap().record);
    assert!(summary.contains("FAIL: `synthetic affected-a` (scope affected-suite, attempt 1)."));
    assert!(summary.contains("FAIL: `synthetic affected-b` (scope affected-suite, attempt 1)."));
    assert!(summary.contains(
        "PASS: `synthetic canonical` (scope full, attempt 1). synthetic passing final validation"
    ));
    assert_eq!(
        summary
            .matches(
                "Investigation: docs/repro.md; https://example.invalid/report; $HOME/out/proof.log."
            )
            .count(),
        2,
    );
}

#[test]
fn unrelated_home_paths_still_fail_the_publication_guard() {
    let temp = tempfile::tempdir().unwrap();
    resolved_fixture(
        temp.path(),
        &["docs/repro.md"],
        "inspect /home/fixture/private before publication",
    );
    let summary = final_resolution_summary(&verify_dir(temp.path()).unwrap().record);
    assert!(!scan_local_paths(&summary).is_empty());
}

#[test]
fn nested_home_components_are_rewritten_once_from_the_original_reference() {
    let temp = tempfile::tempdir().unwrap();
    resolved_fixture(
        temp.path(),
        &[
            "/home/fixture/archive/home/example/report.log",
            "compare /home/fixture/archive/home/example/report.log with /home/example/out/other.log",
        ],
        "same source and covered target",
    );
    let before = fs::read(temp.path().join("test-first-evidence.json")).unwrap();
    let summary = final_resolution_summary(&verify_dir(temp.path()).unwrap().record);
    assert!(
        summary.contains("$HOME/archive/home/example/report.log; compare $HOME/archive/home/example/report.log with $HOME/out/other.log"),
        "each original path must keep its tail and be converted once: {summary}",
    );
    assert!(!summary.contains("archive$HOME"));
    // The existing guard conservatively scans nested home-like components too.
    // Preserve its refusal rather than accepting a corrupted reference.
    assert!(!scan_local_paths(&summary).is_empty());
    assert_eq!(
        fs::read(temp.path().join("test-first-evidence.json")).unwrap(),
        before
    );
}

#[test]
fn uri_home_components_are_preserved_for_the_existing_publication_guard() {
    let temp = tempfile::tempdir().unwrap();
    let references = [
        "https://example.invalid/home/fixture/report",
        "https://example.invalid/Users/fixture/report",
        "[report](https://example.invalid/home/fixture/report)",
        "/home/fixture/report then https://example.invalid/home/fixture/report then /home/fixture/report",
        "file:/home/fixture/report",
    ];
    resolved_fixture(temp.path(), &references, "same source and covered target");
    let file = temp.path().join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let summary = final_resolution_summary(&verify_dir(temp.path()).unwrap().record);

    for reference in [
        &references[0],
        &references[1],
        &references[2],
        &references[4],
    ] {
        assert!(
            summary.contains(reference),
            "URI must remain intact: {summary}"
        );
    }
    assert!(summary.contains(
        "$HOME/report then https://example.invalid/home/fixture/report then $HOME/report"
    ));
    assert!(!summary.contains("example.invalid$HOME"));
    assert!(!summary.contains("file:$HOME"));
    assert!(validate_no_local_paths(&summary, "body").is_err());
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
}

#[test]
fn immutable_validation_command_redirections_export_as_portable_display() {
    let temp = tempfile::tempdir().unwrap();
    let failed = [
        "cargo test affected > /home/fixture/out/affected.log 2>&1",
        "cargo test consumer > /Users/fixture/out/consumer.log 2>&1",
    ];
    let passing = "bash scripts/checks.sh > /home/fixture/out/full.log 2>&1";
    resolved_fixture_with_commands(
        temp.path(),
        &["docs/repro.md"],
        "same source",
        &failed,
        passing,
    );
    let file = temp.path().join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let record = verify_dir(temp.path()).unwrap().record;
    let history = serde_json::to_value(&record.final_validations).unwrap();
    let summary = final_resolution_summary(&record);

    validate_no_local_paths(&summary, "body")
        .expect("portable command display passes normal guard");
    assert!(summary.contains("FAIL: `cargo test affected > $HOME/out/affected.log 2>&1`"));
    assert!(summary.contains("FAIL: `cargo test consumer > $HOME/out/consumer.log 2>&1`"));
    assert!(summary.contains("PASS: `bash scripts/checks.sh > $HOME/out/full.log 2>&1`"));
    assert_eq!(
        summary
            .matches("Resolved by `bash scripts/checks.sh > $HOME/out/full.log 2>&1`")
            .count(),
        2
    );
    assert_eq!(
        serde_json::to_value(&verify_dir(temp.path()).unwrap().record.final_validations).unwrap(),
        history
    );
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
    let retained = String::from_utf8(before).unwrap();
    for command in failed.into_iter().chain([passing]) {
        assert!(retained.contains(command));
    }
}

#[test]
fn validation_command_uri_components_stay_intact_and_guarded() {
    let temp = tempfile::tempdir().unwrap();
    let command = "curl https://example.invalid/home/fixture/report > /home/fixture/out/fetch.log";
    resolved_fixture_with_commands(
        temp.path(),
        &["docs/repro.md"],
        "same source",
        &["synthetic failed"],
        command,
    );
    let file = temp.path().join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let summary = final_resolution_summary(&verify_dir(temp.path()).unwrap().record);
    assert!(summary.contains("https://example.invalid/home/fixture/report > $HOME/out/fetch.log"));
    assert!(!summary.contains("example.invalid$HOME"));
    assert!(validate_no_local_paths(&summary, "body").is_err());
    assert_eq!(fs::read(file).unwrap(), before);
}

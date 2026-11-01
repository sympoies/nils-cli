use std::{fs, path::Path};

use agent_workflow_primitives::test_first_evidence::{
    final_resolution_summary, run_with_args, verify_dir,
};
use pretty_assertions::{assert_eq, assert_ne};
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
            "public dispositions retain only typed validation metadata",
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

fn command_digest(command: &str) -> String {
    Sha256::digest(command.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn public_dispositions_keep_order_and_resolution_identity_without_opaque_text() {
    let temp = tempfile::tempdir().unwrap();
    let failed = [
        "bash /Users/fixture/check.sh > /tmp/evidence/result.log",
        "cargo test affected > /home/fixture/out/affected.log 2>&1",
        "cmd /C C:\\evidence\\check.cmd",
        "curl https://example.invalid/home/fixture/report > /opt/evidence/fetch.log",
    ];
    let passing = "bash /var/tmp/evidence/checks.sh";
    let reason = "confirmed in /tmp/evidence/investigation.json";
    let references = [
        "/home/fixture/archive/home/example/report.log",
        "https://example.invalid/Users/fixture/report",
        "file:/home/fixture/report",
        "docs/repro.md",
        "opaque investigation text `with Markdown`",
    ];
    resolved_fixture_with_commands(temp.path(), &references, reason, &failed, passing);
    let file = temp.path().join("test-first-evidence.json");
    let before = fs::read(&file).unwrap();
    let hash = Sha256::digest(&before);
    let record = verify_dir(temp.path()).unwrap().record;
    let history = serde_json::to_value(&record.final_validations).unwrap();
    let summary = final_resolution_summary(&record);
    let passing_digest = command_digest(passing);
    let mut expected = format!(
        "## Final validation dispositions\n\n- PASS: command `sha256:{}` (scope focused, attempt 1).",
        command_digest("synthetic focused")
    );
    for command in failed {
        expected.push_str(&format!(
            "\n- FAIL: command `sha256:{}` (scope affected-suite, attempt 1). Resolved by command `sha256:{passing_digest}` (scope full, attempt 1).",
            command_digest(command)
        ));
    }
    expected.push_str(&format!(
        "\n- PASS: command `sha256:{passing_digest}` (scope full, attempt 1).\n"
    ));
    assert_eq!(summary, expected);
    for detail in failed
        .into_iter()
        .chain([passing, reason])
        .chain(references)
    {
        assert!(!summary.contains(detail));
    }
    assert!(!summary.contains("synthetic passing final validation"));
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(Sha256::digest(fs::read(&file).unwrap()), hash);
    assert_eq!(
        serde_json::to_value(&verify_dir(temp.path()).unwrap().record.final_validations).unwrap(),
        history
    );
}

#[test]
fn public_command_identity_uses_exact_bytes_and_keeps_scope_and_attempt() {
    let temp = tempfile::tempdir().unwrap();
    resolved_fixture_with_commands(
        temp.path(),
        &["synthetic investigation"],
        "same target",
        &["command", "other command"],
        "canonical",
    );
    let mut record = verify_dir(temp.path()).unwrap().record;
    // Historical records may lack scope; the fixed projection must remain typed.
    let first = record
        .final_validations
        .iter_mut()
        .find(|v| v.command == "command")
        .unwrap();
    first.scope = None;
    let second = record
        .final_validations
        .iter_mut()
        .find(|v| v.command == "other command")
        .unwrap();
    second.command = "command ".to_owned();
    second.attempt = 2;
    let summary = final_resolution_summary(&record);
    assert!(summary.contains(&format!(
        "FAIL: command `sha256:{}` (scope unknown, attempt 1).",
        command_digest("command")
    )));
    assert!(summary.contains(&format!(
        "FAIL: command `sha256:{}` (scope affected-suite, attempt 2).",
        command_digest("command ")
    )));
    assert_ne!(command_digest("command"), command_digest("command "));
    for validation in &mut record.final_validations {
        validation.resolution = None;
    }
    assert_eq!(final_resolution_summary(&record), "");
}

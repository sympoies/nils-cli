use std::fs;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::subject_binding::{git, init_repo, run};

#[test]
fn verify_reports_structural_and_subject_steps_together_in_both_formats() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = init_repo(tmp.path(), "repo", None);
    let out = tmp.path().join("evidence");
    assert_eq!(
        run(
            tmp.path(),
            &[
                "init",
                "--out",
                out.to_str().unwrap(),
                "--classification",
                "behavior-change"
            ]
        )
        .code,
        0
    );
    for format in ["json", "text"] {
        let result = run(
            tmp.path(),
            &[
                "verify",
                "--out",
                out.to_str().unwrap(),
                "--project-path",
                repo.to_str().unwrap(),
                "--format",
                format,
            ],
        );
        assert_eq!(result.code, 1);
        let rendered = if format == "json" {
            result.stdout_text()
        } else {
            result.stderr_text()
        };
        for field in [
            "changed_added_or_removed_behavior",
            "test_impacts_or_none",
            "failing_tests_or_waiver",
            "focused_final_validation",
            "residual_gaps_declaration",
            "baseline_subject",
            "delivery_subject",
        ] {
            assert!(rendered.contains(field), "missing {field}: {rendered}");
        }
        for command in [
            "record-impact",
            "record-failing",
            "record-final",
            "record-gap",
            "bind-baseline",
            "bind-delivery",
        ] {
            assert!(
                rendered.contains(command),
                "missing recovery {command}: {rendered}"
            );
        }
    }
}

#[test]
fn retro_bind_retains_log_digests_and_real_baseline_and_requires_remaining_steps() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = init_repo(
        tmp.path(),
        "repo",
        Some("https://github.com/acme/widget.git"),
    );
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let out = tmp.path().join("evidence");
    let red = tmp.path().join("red.log");
    let green = tmp.path().join("green.log");
    fs::write(&red, "assertion failed: expected supported import\n").unwrap();
    fs::write(&green, "test result: ok\n").unwrap();
    fs::write(repo.join("implementation.txt"), "new behavior\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "implement behavior"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    // Initialization happens after implementation; the original baseline is explicit.
    assert_eq!(
        run(
            tmp.path(),
            &[
                "init",
                "--out",
                out.to_str().unwrap(),
                "--classification",
                "behavior-change",
                "--added-behavior",
                "retained log import"
            ]
        )
        .code,
        0
    );
    let args = [
        "retro-bind",
        "--out",
        out.to_str().unwrap(),
        "--project-path",
        repo.to_str().unwrap(),
        "--baseline",
        &base,
        "--red-log",
        red.to_str().unwrap(),
        "--green-log",
        green.to_str().unwrap(),
        "--red-command",
        "cargo test retained_import",
        "--green-command",
        "cargo test retained_import",
        "--red-exit-code",
        "101",
        "--green-exit-code",
        "0",
        "--expected-failure",
        "import unsupported",
        "--observed-failure",
        "assertion failed",
        "--reason",
        "Evidence was retained before record initialization",
        "--attest-pre-edit-red",
        "--format",
        "json",
    ];
    let imported = run(tmp.path(), &args);
    assert_eq!(
        imported.code,
        0,
        "{} {}",
        imported.stdout_text(),
        imported.stderr_text()
    );
    let imported_json = imported.stdout_json();
    let record = &imported_json["result"]["record"];
    assert_eq!(record["schema_version"], "test-first-evidence.record.v2");
    assert_eq!(record["subject"]["baseline"]["commit"], base);
    assert_eq!(record["subject"]["retro_binding"]["mode"], "retro-bound");
    assert_eq!(
        record["subject"]["retro_binding"]["pre_edit_red_attested"],
        true
    );
    assert_eq!(record["subject"]["retro_binding"]["bound_head"], head);
    let red_digest = format!(
        "sha256:{}",
        Sha256::digest(fs::read(&red).unwrap())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    assert_eq!(
        record["subject"]["retro_binding"]["red_log_digest"],
        red_digest
    );
    assert_eq!(record["failing_tests"][0]["exit_code"], 101);
    assert_eq!(record["final_validations"][0]["scope"], "focused");
    let verify_args = [
        "verify",
        "--out",
        out.to_str().unwrap(),
        "--project-path",
        repo.to_str().unwrap(),
        "--format",
        "json",
    ];
    let incomplete = run(tmp.path(), &verify_args);
    assert_eq!(incomplete.code, 1);
    let missing = incomplete.stdout_json()["error"]["details"]["missing"].clone();
    assert!(
        missing
            .as_array()
            .unwrap()
            .contains(&json!("test_impacts_or_none"))
    );
    assert!(
        missing
            .as_array()
            .unwrap()
            .contains(&json!("residual_gaps_declaration"))
    );
    for args in [
        vec![
            "record-impact",
            "--out",
            out.to_str().unwrap(),
            "--none",
            "--reason",
            "new fixture owner",
        ],
        vec!["record-gap", "--out", out.to_str().unwrap(), "--none"],
        vec![
            "bind-delivery",
            "--out",
            out.to_str().unwrap(),
            "--project-path",
            repo.to_str().unwrap(),
        ],
    ] {
        assert_eq!(run(tmp.path(), &args).code, 0);
    }
    assert_eq!(run(tmp.path(), &verify_args).code, 0);
    let before = fs::read(out.join("test-first-evidence.json")).unwrap();
    let duplicate = run(tmp.path(), &args);
    assert_eq!(duplicate.code, 65);
    assert_eq!(
        fs::read(out.join("test-first-evidence.json")).unwrap(),
        before
    );
    let mut corrupt: Value = serde_json::from_slice(&before).unwrap();
    corrupt["subject"]["retro_binding"]["red_log_digest"] = json!("invalid");
    fs::write(
        out.join("test-first-evidence.json"),
        serde_json::to_vec(&corrupt).unwrap(),
    )
    .unwrap();
    assert_eq!(run(tmp.path(), &verify_args).code, 1);
}

#[test]
fn retro_bind_rejects_invalid_evidence_without_partial_updates() {
    let tmp = tempfile::TempDir::new().unwrap();
    let repo = init_repo(tmp.path(), "repo", None);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    let out = tmp.path().join("evidence");
    let red = tmp.path().join("red.log");
    let green = tmp.path().join("green.log");
    fs::write(&red, "assertion failed\n").unwrap();
    fs::write(&green, "test passed\n").unwrap();
    assert_eq!(
        run(
            tmp.path(),
            &[
                "init",
                "--out",
                out.to_str().unwrap(),
                "--classification",
                "behavior-change",
                "--added-behavior",
                "import evidence"
            ]
        )
        .code,
        0
    );
    let args = vec![
        "retro-bind",
        "--out",
        out.to_str().unwrap(),
        "--project-path",
        repo.to_str().unwrap(),
        "--baseline",
        &base,
        "--red-log",
        red.to_str().unwrap(),
        "--green-log",
        green.to_str().unwrap(),
        "--red-command",
        "cargo test import",
        "--green-command",
        "cargo test import",
        "--red-exit-code",
        "101",
        "--green-exit-code",
        "0",
        "--expected-failure",
        "missing behavior",
        "--observed-failure",
        "assertion failed",
        "--reason",
        "retained logs predate initialization",
        "--attest-pre-edit-red",
        "--format",
        "json",
    ];
    let before = fs::read(out.join("test-first-evidence.json")).unwrap();
    for (flag, value, code) in [
        ("--red-exit-code", "0", "invalid-retained-evidence"),
        ("--green-exit-code", "1", "invalid-retained-evidence"),
        ("--reason", " ", "invalid-retained-evidence"),
        ("--expected-failure", " ", "invalid-retained-evidence"),
        ("--baseline", "HEAD", "invalid-retained-baseline"),
        (
            "--green-log",
            "absent-retained-log",
            "retained-log-unreadable",
        ),
    ] {
        let mut invalid = args.clone();
        let index = invalid.iter().position(|arg| *arg == flag).unwrap();
        invalid[index + 1] = value;
        let result = run(tmp.path(), &invalid);
        assert_eq!(result.code, 65, "{flag}: {}", result.stdout_text());
        assert_eq!(result.stdout_json()["error"]["code"], code);
        assert_eq!(
            fs::read(out.join("test-first-evidence.json")).unwrap(),
            before
        );
    }
    let without_attestation: Vec<_> = args
        .iter()
        .copied()
        .filter(|arg| *arg != "--attest-pre-edit-red")
        .collect();
    assert_eq!(run(tmp.path(), &without_attestation).code, 65);
    fs::write(&green, " \n\t").unwrap();
    let empty = run(tmp.path(), &args);
    assert_eq!(empty.stdout_json()["error"]["code"], "retained-log-empty");
    assert_eq!(
        fs::read(out.join("test-first-evidence.json")).unwrap(),
        before
    );
}

use std::fs;
use std::path::PathBuf;

use serde_json::Value;
use tempfile::TempDir;

mod common;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("split_prs")
        .join(name)
}

fn fixture_text(name: &str) -> String {
    fs::read_to_string(fixture_path(name)).expect("fixture exists")
}

fn tsv_rows(name: &str) -> Vec<Vec<String>> {
    fixture_text(name)
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.split('\t').map(|part| part.to_string()).collect())
        .collect()
}

#[test]
fn split_prs_deterministic_per_sprint_tsv_matches_fixture() {
    let dir = TempDir::new().expect("tempdir");
    common::write_file(&dir.path().join("plan.md"), &fixture_text("duck-plan.md"));

    let out = common::run_plan_tooling(
        dir.path(),
        &[
            "split-prs",
            "--file",
            "plan.md",
            "--scope",
            "sprint",
            "--sprint",
            "1",
            "--pr-grouping",
            "per-sprint",
            "--format",
            "tsv",
        ],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(out.stdout, fixture_text("per_sprint_expected.tsv"));
}

#[test]
fn split_prs_deterministic_group_json_matches_fixture() {
    let dir = TempDir::new().expect("tempdir");
    common::write_file(&dir.path().join("plan.md"), &fixture_text("duck-plan.md"));

    let out = common::run_plan_tooling(
        dir.path(),
        &[
            "split-prs",
            "--file",
            "plan.md",
            "--scope",
            "sprint",
            "--sprint",
            "2",
            "--pr-grouping",
            "group",
            "--pr-group",
            "S2T1=s2-isolated",
            "--pr-group",
            "S2T2=s2-shared",
            "--pr-group",
            "S2T3=s2-shared",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let actual: Value = serde_json::from_str(&out.stdout).expect("json");
    let mut expected: Value =
        serde_json::from_str(&fixture_text("group_expected.json")).expect("fixture json");

    expected["file"] = actual["file"].clone();
    assert_eq!(actual, expected);
}

#[test]
fn split_prs_error_group_requires_mapping() {
    let dir = TempDir::new().expect("tempdir");
    common::write_file(&dir.path().join("plan.md"), &fixture_text("duck-plan.md"));

    let out = common::run_plan_tooling(
        dir.path(),
        &[
            "split-prs",
            "--file",
            "plan.md",
            "--scope",
            "sprint",
            "--sprint",
            "2",
            "--pr-grouping",
            "group",
        ],
    );
    assert_eq!(out.code, 2);
    assert!(
        out.stderr
            .contains("--pr-grouping group requires at least one --pr-group"),
        "{}",
        out.stderr
    );
}

#[test]
fn split_prs_error_unknown_mapping_key() {
    let dir = TempDir::new().expect("tempdir");
    common::write_file(&dir.path().join("plan.md"), &fixture_text("duck-plan.md"));

    let out = common::run_plan_tooling(
        dir.path(),
        &[
            "split-prs",
            "--file",
            "plan.md",
            "--scope",
            "sprint",
            "--sprint",
            "2",
            "--pr-grouping",
            "group",
            "--pr-group",
            "S2T1=s2-isolated",
            "--pr-group",
            "S2T2=s2-shared",
            "--pr-group",
            "S2T9=s2-shared",
        ],
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("unknown task keys"), "{}", out.stderr);
}

#[test]
fn split_prs_auto_not_implemented() {
    let dir = TempDir::new().expect("tempdir");
    common::write_file(&dir.path().join("plan.md"), &fixture_text("duck-plan.md"));

    let out = common::run_plan_tooling(
        dir.path(),
        &[
            "split-prs",
            "--file",
            "plan.md",
            "--scope",
            "sprint",
            "--sprint",
            "1",
            "--pr-grouping",
            "per-sprint",
            "--strategy",
            "auto",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 1);
    assert!(out.stderr.contains("not implemented"), "{}", out.stderr);
    assert_eq!(out.stderr.matches("Complexity").count(), 1);
    assert_eq!(out.stderr.matches("Location").count(), 1);
    assert_eq!(out.stderr.matches("Dependencies").count(), 1);
}

#[test]
fn split_prs_fixture_tsv_header_is_stable() {
    for file in ["per_sprint_expected.tsv", "group_expected.tsv"] {
        let path = fixture_path(file);
        let text = fs::read_to_string(path).expect("fixture exists");
        let first = text.lines().next().unwrap_or_default();
        assert_eq!(
            first,
            "# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group"
        );
    }
}

#[test]
fn split_prs_non_regression_required_notes_keys() {
    for row in tsv_rows("per_sprint_expected.tsv") {
        assert_eq!(row.len(), 7);
        let notes = &row[5];
        assert!(notes.contains("sprint=S1"));
        assert!(notes.contains("plan-task:Task "));
        assert!(notes.contains("pr-grouping=per-sprint"));
        assert!(notes.contains("pr-group=s1"));
    }
}

#[test]
fn split_prs_non_regression_shared_anchor_rules() {
    for row in tsv_rows("group_expected.tsv") {
        assert_eq!(row.len(), 7);
        let task_id = &row[0];
        let notes = &row[5];
        assert!(notes.contains("pr-grouping=group"));
        if task_id == "S2T1" {
            assert!(!notes.contains("shared-pr-anchor="));
        } else {
            assert!(notes.contains("shared-pr-anchor=S2T2"));
        }
    }
}

#[test]
fn split_prs_fixture_json_contains_required_fields() {
    for file in ["per_sprint_expected.json", "group_expected.json"] {
        let path = fixture_path(file);
        let text = fs::read_to_string(path).expect("fixture exists");
        let value: Value = serde_json::from_str(&text).expect("valid json");

        assert!(value["file"].is_string() || value["file"].is_null());
        assert!(value["scope"].is_string());
        assert!(value["pr_grouping"].is_string());
        assert!(value["strategy"].is_string());
        let records = value["records"].as_array().expect("records array");
        assert!(!records.is_empty());

        for record in records {
            assert!(record["task_id"].is_string());
            assert!(record["summary"].is_string());
            assert!(record["branch"].is_string());
            assert!(record["worktree"].is_string());
            assert!(record["owner"].is_string());
            assert!(record["notes"].is_string());
            assert!(record["pr_group"].is_string());
        }
    }
}

#[test]
fn split_prs_error_matrix_doc_mentions_core_cases() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("docs")
        .join("specs")
        .join("split-prs-contract-v1.md");
    let text = fs::read_to_string(path).expect("spec exists");

    for token in [
        "unknown mapping key",
        "missing mapping",
        "--pr-grouping",
        "scope=sprint",
    ] {
        assert!(
            text.contains(token),
            "expected token in error matrix: {token}"
        );
    }
}

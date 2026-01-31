mod common;

use common::{run_plan_tooling, write_file};

#[test]
fn to_json_pretty_parses_and_includes_start_lines() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let plan_path = dir.path().join("plan.md");
    write_file(&plan_path, VALID_PLAN);

    let out = run_plan_tooling(dir.path(), &["to-json", "--file", "plan.md", "--pretty"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["title"], "Plan: Example");
    assert_eq!(v["file"], "plan.md");
    assert_eq!(v["sprints"][0]["number"], 1);
    assert_eq!(v["sprints"][0]["start_line"], 3);
    assert_eq!(v["sprints"][0]["tasks"][0]["id"], "Task 1.1");
    assert_eq!(v["sprints"][0]["tasks"][0]["start_line"], 9);
    assert_eq!(v["sprints"][0]["tasks"][1]["id"], "Task 1.2");
    assert_eq!(v["sprints"][0]["tasks"][1]["start_line"], 21);
}

#[test]
fn to_json_sprint_filter_returns_exact_sprint() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let plan_path = dir.path().join("plan.md");
    write_file(&plan_path, VALID_PLAN);

    let out = run_plan_tooling(
        dir.path(),
        &["to-json", "--file", "plan.md", "--sprint", "1"],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["sprints"].as_array().unwrap().len(), 1);

    let out = run_plan_tooling(
        dir.path(),
        &["to-json", "--file", "plan.md", "--sprint", "2"],
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("json");
    assert_eq!(v["sprints"].as_array().unwrap().len(), 0);
}

#[test]
fn to_json_invalid_sprint_is_usage_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");
    let plan_path = dir.path().join("plan.md");
    write_file(&plan_path, VALID_PLAN);

    let out = run_plan_tooling(
        dir.path(),
        &["to-json", "--file", "plan.md", "--sprint", "nope"],
    );
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("error: invalid --sprint"));
    assert!(out.stderr.contains("'nope'"));
}

#[test]
fn to_json_missing_file_is_parse_error() {
    let dir = tempfile::TempDir::new().expect("tempdir");

    let out = run_plan_tooling(dir.path(), &["to-json", "--file", "missing.md"]);
    assert_eq!(out.code, 1);
    assert!(out
        .stderr
        .contains("error: plan file not found: missing.md"));
}

const VALID_PLAN: &str = r#"# Plan: Example

## Sprint 1: First sprint
**Goal**: ...
**Demo/Validation**:
- Command(s): ...
- Verify: ...

### Task 1.1: Do thing
- **Location**:
  - `src/a.rs`
- **Description**: Do A
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - A works
- **Validation**:
  - cargo test -p plan-tooling

### Task 1.2: Do other
- **Location**:
  - `src/b.rs`
- **Description**: Do B
- **Dependencies**:
  - Task 1.1
- **Complexity**: 2
- **Acceptance criteria**:
  - B works
- **Validation**:
  - cargo test -p plan-tooling
"#;

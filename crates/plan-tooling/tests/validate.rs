mod common;

use common::{git, init_repo, run_plan_tooling, write_file};

#[test]
fn validate_ok_with_explicit_file() {
    let repo = init_repo();
    write_file(&repo.path().join("plan.md"), VALID_PLAN);

    let out = run_plan_tooling(repo.path(), &["validate", "--file", "plan.md"]);
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn validate_fails_with_errors() {
    let repo = init_repo();
    write_file(&repo.path().join("bad.md"), INVALID_PLAN);

    let out = run_plan_tooling(repo.path(), &["validate", "--file", "bad.md"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("error:"));
    assert!(out.stderr.contains("Location"));
}

#[test]
fn validate_default_discovers_tracked_docs_plans() {
    let repo = init_repo();

    let plan_path = repo.path().join("docs/plans/example-plan.md");
    write_file(&plan_path, VALID_PLAN);

    git(repo.path(), &["add", "docs/plans/example-plan.md"]);
    git(repo.path(), &["commit", "-m", "add plan", "-q"]);

    let out = run_plan_tooling(repo.path(), &["validate"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

const VALID_PLAN: &str = r#"# Plan: Example

## Sprint 1: First sprint

### Task 1.1: Do thing
- **Location**:
  - `src/a.rs`
- **Description**: Do A
- **Dependencies**:
  - none
- **Acceptance criteria**:
  - A works
- **Validation**:
  - cargo test -p plan-tooling
"#;

const INVALID_PLAN: &str = r#"# Plan: Bad

## Sprint 1: Bad sprint

### Task 1.1: Broken
- **Location**:
  - `/abs/path.rs`
- **Description**: TODO
- **Dependencies**:
  - Task 1.2
- **Acceptance criteria**:
  - <TBD>
- **Validation**:
  - TBD
"#;

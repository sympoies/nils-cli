mod common;

use common::{git, init_repo, run_plan_tooling, write_file};

use pretty_assertions::assert_eq;
use tempfile::TempDir;

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
fn validate_explicit_file_without_git_repo() {
    let dir = TempDir::new().expect("tempdir");
    write_file(&dir.path().join("plan.md"), VALID_PLAN);

    let out = run_plan_tooling(dir.path(), &["validate", "--file", "plan.md"]);
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

#[test]
fn validate_repo_relative_file_works_from_nested_dir() {
    let repo = init_repo();

    let plan_path = repo.path().join("docs/plans/example-plan.md");
    write_file(&plan_path, VALID_PLAN);

    let nested = repo.path().join("nested/dir");
    std::fs::create_dir_all(&nested).expect("create_dir_all");

    let out = run_plan_tooling(
        &nested,
        &["validate", "--file", "docs/plans/example-plan.md"],
    );
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
}

#[test]
fn validate_missing_dependencies_is_error() {
    let repo = init_repo();
    write_file(&repo.path().join("missing-deps.md"), MISSING_DEPS_PLAN);

    let out = run_plan_tooling(repo.path(), &["validate", "--file", "missing-deps.md"]);
    assert_eq!(out.code, 1);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("missing Dependencies"));
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

const MISSING_DEPS_PLAN: &str = r#"# Plan: Missing deps

## Sprint 1: First sprint

### Task 1.1: Do thing
- **Location**:
  - `src/a.rs`
- **Description**: Do A
- **Dependencies**:
- **Acceptance criteria**:
  - A works
- **Validation**:
  - cargo test -p plan-tooling
"#;

use std::fs;

use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

mod common;

const PLAN_PATH: &str = "docs/plans/plan-issue-rust-cli-full-delivery-plan.md";

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout should be valid JSON")
}

fn result_path(payload: &Value, key: &str) -> String {
    payload["payload"]["result"][key]
        .as_str()
        .unwrap_or_else(|| panic!("missing result path key: {key}"))
        .to_string()
}

#[test]
fn task_spec_generation_build_task_spec_writes_grouped_rows() {
    let tmp = TempDir::new().expect("temp dir");
    let out_path = tmp.path().join("sprint3.tsv");
    let out_path_s = out_path.to_string_lossy().to_string();

    let out = common::run_plan_issue(&[
        "--format",
        "json",
        "build-task-spec",
        "--plan",
        PLAN_PATH,
        "--sprint",
        "3",
        "--pr-grouping",
        "group",
        "--pr-group",
        "S3T1=s3-a",
        "--pr-group",
        "S3T2=s3-b",
        "--pr-group",
        "S3T3=s3-c",
        "--task-spec-out",
        &out_path_s,
    ]);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let payload = parse_json(&out.stdout);
    assert_eq!(payload["command"], "build-task-spec");
    assert_eq!(payload["status"], "ok");

    let rendered = fs::read_to_string(&out_path).expect("read task-spec");
    let mut lines = rendered.lines();
    assert_eq!(
        lines.next().unwrap_or_default(),
        "# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group"
    );

    let task_rows: Vec<&str> = rendered
        .lines()
        .filter(|line| line.starts_with("S3T"))
        .collect();
    assert_eq!(task_rows.len(), 3, "{rendered}");
    assert!(rendered.contains("\ts3-a"), "{rendered}");
    assert!(rendered.contains("\ts3-b"), "{rendered}");
    assert!(rendered.contains("\ts3-c"), "{rendered}");
}

#[test]
fn render_issue_body_start_plan_writes_issue_body_artifact() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("create agent home");

    let task_spec = tmp.path().join("plan.tsv");
    let issue_body = tmp.path().join("issue-body.md");
    let task_spec_s = task_spec.to_string_lossy().to_string();
    let issue_body_s = issue_body.to_string_lossy().to_string();
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-plan",
            "--plan",
            PLAN_PATH,
            "--pr-grouping",
            "per-sprint",
            "--task-spec-out",
            &task_spec_s,
            "--issue-body-out",
            &issue_body_s,
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let rendered = fs::read_to_string(&issue_body).expect("read issue body");
    assert!(rendered.contains("## Task Decomposition"), "{rendered}");
    assert!(
        rendered.contains("| S3T1 | Implement task-spec generation core using `plan-tooling` |")
    );
}

#[test]
fn render_issue_body_start_sprint_writes_start_comment_with_modes() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let task_spec = tmp.path().join("s3.tsv");
    let task_spec_s = task_spec.to_string_lossy().to_string();

    let out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-sprint",
            "--plan",
            PLAN_PATH,
            "--issue",
            "217",
            "--sprint",
            "3",
            "--task-spec-out",
            &task_spec_s,
            "--pr-grouping",
            "group",
            "--pr-group",
            "S3T1=s3-a",
            "--pr-group",
            "S3T2=s3-b",
            "--pr-group",
            "S3T3=s3-c",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let payload = parse_json(&out.stdout);
    let comment_path = result_path(&payload, "comment_path");
    let comment = fs::read_to_string(&comment_path).expect("read sprint comment");
    assert!(comment.contains("## Sprint 3 Start"), "{comment}");
    assert!(
        comment.contains("| Task | Summary | Execution Mode |"),
        "{comment}"
    );
    assert!(comment.contains("pr-isolated"), "{comment}");
}

#[test]
fn local_flow_plan_issue_local_dry_run_end_to_end_generates_artifacts() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let start_plan_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-plan",
            "--plan",
            PLAN_PATH,
            "--pr-grouping",
            "per-sprint",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(start_plan_out.code, 0, "stderr: {}", start_plan_out.stderr);
    let start_plan_json = parse_json(&start_plan_out.stdout);
    let issue_body_path = result_path(&start_plan_json, "issue_body_path");
    assert!(std::path::Path::new(&issue_body_path).is_file());

    let start_sprint_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-sprint",
            "--plan",
            PLAN_PATH,
            "--issue",
            "217",
            "--sprint",
            "3",
            "--pr-grouping",
            "per-sprint",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(
        start_sprint_out.code, 0,
        "stderr: {}",
        start_sprint_out.stderr
    );
    let start_sprint_json = parse_json(&start_sprint_out.stdout);
    let start_comment_path = result_path(&start_sprint_json, "comment_path");
    assert!(std::path::Path::new(&start_comment_path).is_file());

    let ready_sprint_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "ready-sprint",
            "--plan",
            PLAN_PATH,
            "--issue",
            "217",
            "--sprint",
            "3",
            "--pr-grouping",
            "per-sprint",
            "--summary",
            "Ready for review",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(
        ready_sprint_out.code, 0,
        "stderr: {}",
        ready_sprint_out.stderr
    );
    let ready_sprint_json = parse_json(&ready_sprint_out.stdout);
    let ready_comment_path = result_path(&ready_sprint_json, "comment_path");
    assert!(std::path::Path::new(&ready_comment_path).is_file());

    let accept_sprint_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "accept-sprint",
            "--plan",
            PLAN_PATH,
            "--issue",
            "217",
            "--sprint",
            "3",
            "--pr-grouping",
            "per-sprint",
            "--approved-comment-url",
            "https://github.com/graysurf/nils-cli/issues/217#issuecomment-123456789",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(
        accept_sprint_out.code, 0,
        "stderr: {}",
        accept_sprint_out.stderr
    );
    let accept_sprint_json = parse_json(&accept_sprint_out.stdout);
    let accepted_comment_path = result_path(&accept_sprint_json, "comment_path");
    let accepted_comment =
        fs::read_to_string(&accepted_comment_path).expect("read accepted comment");
    assert!(
        accepted_comment.contains("## Sprint 3 Accepted"),
        "{accepted_comment}"
    );
    assert!(
        accepted_comment
            .contains("https://github.com/graysurf/nils-cli/issues/217#issuecomment-123456789"),
        "{accepted_comment}"
    );
}

use std::collections::HashMap;
use std::fs;

use pretty_assertions::assert_eq;
use serde_json::Value;
use tempfile::TempDir;

mod common;

#[derive(Debug, Clone)]
struct IssueTaskRow {
    owner: String,
    branch: String,
    worktree: String,
    execution_mode: String,
    notes: String,
}

#[derive(Debug, Clone)]
struct SpecRow {
    owner: String,
    branch: String,
    worktree: String,
    notes: String,
    pr_group: String,
}

fn parse_json(stdout: &str) -> Value {
    serde_json::from_str(stdout).expect("stdout should be valid JSON")
}

fn result_path(payload: &Value, key: &str) -> String {
    payload["payload"]["result"][key]
        .as_str()
        .unwrap_or_else(|| panic!("missing result path key: {key}"))
        .to_string()
}

fn parse_markdown_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
        return Vec::new();
    }

    trimmed[1..trimmed.len() - 1]
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn parse_task_decomposition_rows(issue_body: &str) -> HashMap<String, IssueTaskRow> {
    let lines = issue_body.lines().collect::<Vec<_>>();
    let start = lines
        .iter()
        .position(|line| line.trim() == "## Task Decomposition")
        .map(|idx| idx + 1)
        .expect("task decomposition heading");

    let table_lines = lines
        .iter()
        .skip(start)
        .take_while(|line| !line.starts_with("## "))
        .copied()
        .filter(|line| line.trim().starts_with('|'))
        .collect::<Vec<_>>();
    assert!(table_lines.len() >= 3, "missing task table\n{issue_body}");

    let headers = parse_markdown_row(table_lines[0]);
    let task_idx = headers.iter().position(|h| h == "Task").expect("Task col");
    let owner_idx = headers
        .iter()
        .position(|h| h == "Owner")
        .expect("Owner col");
    let branch_idx = headers
        .iter()
        .position(|h| h == "Branch")
        .expect("Branch col");
    let worktree_idx = headers
        .iter()
        .position(|h| h == "Worktree")
        .expect("Worktree col");
    let mode_idx = headers
        .iter()
        .position(|h| h == "Execution Mode")
        .expect("Execution Mode col");
    let notes_idx = headers
        .iter()
        .position(|h| h == "Notes")
        .expect("Notes col");

    let mut rows = HashMap::new();
    for line in table_lines.iter().skip(2) {
        let cells = parse_markdown_row(line);
        if cells.len() != headers.len() {
            continue;
        }
        let task = cells[task_idx].trim();
        if task.is_empty() {
            continue;
        }

        rows.insert(
            task.to_string(),
            IssueTaskRow {
                owner: cells[owner_idx].clone(),
                branch: cells[branch_idx].clone(),
                worktree: cells[worktree_idx].clone(),
                execution_mode: cells[mode_idx].clone(),
                notes: cells[notes_idx].clone(),
            },
        );
    }

    rows
}

fn parse_task_spec_rows(tsv: &str) -> HashMap<String, SpecRow> {
    let mut rows = HashMap::new();

    for line in tsv.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let cols = line.split('\t').collect::<Vec<_>>();
        assert_eq!(cols.len(), 7, "unexpected task-spec row: {line}");
        rows.insert(
            cols[0].to_string(),
            SpecRow {
                branch: cols[2].to_string(),
                worktree: cols[3].to_string(),
                owner: cols[4].to_string(),
                notes: cols[5].to_string(),
                pr_group: cols[6].to_string(),
            },
        );
    }

    rows
}

fn note_value(notes: &str, key: &str) -> Option<String> {
    notes
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{key}=")).map(str::to_string))
}

fn parse_prompt_fields(prompt: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in prompt.lines() {
        if let Some(rest) = line.strip_prefix("- ")
            && let Some((key, value)) = rest.split_once(": ")
        {
            out.insert(key.to_string(), value.to_string());
        }
    }
    out
}

#[test]
fn start_plan_and_start_sprint_auto_single_lane_artifacts_can_drift_from_per_sprint_semantics() {
    let tmp = TempDir::new().expect("temp dir");
    let agent_home = tmp.path().join("agent-home");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let agent_home_s = agent_home.to_string_lossy().to_string();

    let plan_file = tmp.path().join("sprint3-auto-single-lane.md");
    let plan_file_s = plan_file.to_string_lossy().to_string();
    fs::write(
        &plan_file,
        r#"# Plan: Sprint 3 auto single lane delivery

## Sprint 3: Shared lane
- **PR grouping intent**: `group`.
- **Execution Profile**: `serial` (parallel width 1).

### Task 3.1: First lane task
- **Location**:
  - crates/plan-issue-cli/src/a.rs
- **Dependencies**:
  - none

### Task 3.2: Follow-up lane task
- **Location**:
  - crates/plan-issue-cli/src/b.rs
- **Dependencies**:
  - Task 3.1
"#,
    )
    .expect("write plan");

    let plan_task_spec = tmp.path().join("plan-task-spec.tsv");
    let plan_issue_body = tmp.path().join("plan-issue-body.md");
    let plan_task_spec_s = plan_task_spec.to_string_lossy().to_string();
    let plan_issue_body_s = plan_issue_body.to_string_lossy().to_string();

    let start_plan_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-plan",
            "--plan",
            &plan_file_s,
            "--pr-grouping",
            "group",
            "--strategy",
            "auto",
            "--task-spec-out",
            &plan_task_spec_s,
            "--issue-body-out",
            &plan_issue_body_s,
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(start_plan_out.code, 0, "stderr: {}", start_plan_out.stderr);

    let issue_body = fs::read_to_string(&plan_issue_body).expect("read issue body");
    let issue_rows = parse_task_decomposition_rows(&issue_body);
    let issue_s3t1 = issue_rows.get("S3T1").expect("S3T1 issue row");
    let issue_s3t2 = issue_rows.get("S3T2").expect("S3T2 issue row");
    // Characterization: start-plan issue body leaves execution metadata placeholders, while
    // later sprint artifacts are generated from recomputed task-spec rows.
    assert_eq!(issue_s3t1.execution_mode, "TBD");
    assert_eq!(issue_s3t2.execution_mode, "TBD");
    assert_eq!(issue_s3t1.owner, "TBD");
    assert_eq!(issue_s3t2.owner, "TBD");
    assert_eq!(issue_s3t1.branch, "TBD");
    assert_eq!(issue_s3t2.branch, "TBD");
    assert_eq!(issue_s3t1.worktree, "TBD");
    assert_eq!(issue_s3t2.worktree, "TBD");

    let sprint_task_spec = tmp.path().join("sprint3-task-spec.tsv");
    let sprint_task_spec_s = sprint_task_spec.to_string_lossy().to_string();
    let prompts_out = tmp.path().join("sprint3-prompts");
    let prompts_out_s = prompts_out.to_string_lossy().to_string();

    let start_sprint_out = common::run_plan_issue_local_with_env(
        &[
            "--format",
            "json",
            "--dry-run",
            "start-sprint",
            "--plan",
            &plan_file_s,
            "--issue",
            "217",
            "--sprint",
            "3",
            "--task-spec-out",
            &sprint_task_spec_s,
            "--subagent-prompts-out",
            &prompts_out_s,
            "--pr-grouping",
            "group",
            "--strategy",
            "auto",
            "--no-comment",
        ],
        &[("AGENT_HOME", &agent_home_s)],
    );
    assert_eq!(
        start_sprint_out.code, 0,
        "stderr: {}",
        start_sprint_out.stderr
    );

    let sprint_payload = parse_json(&start_sprint_out.stdout);
    let comment_path = result_path(&sprint_payload, "comment_path");
    let comment = fs::read_to_string(&comment_path).expect("read sprint comment");
    assert!(
        comment.contains("| S3T1 | First lane task | per-sprint |"),
        "{comment}"
    );
    assert!(
        comment.contains("| S3T2 | Follow-up lane task | per-sprint |"),
        "{comment}"
    );
    assert!(!comment.contains("pr-shared"), "{comment}");

    let spec_text = fs::read_to_string(&sprint_task_spec).expect("read sprint task-spec");
    let spec_rows = parse_task_spec_rows(&spec_text);
    let spec_s3t1 = spec_rows.get("S3T1").expect("S3T1 spec row");
    let spec_s3t2 = spec_rows.get("S3T2").expect("S3T2 spec row");
    assert_eq!(spec_s3t1.pr_group, spec_s3t2.pr_group);
    assert!(
        spec_s3t1.pr_group.starts_with("s3-auto-g"),
        "{}",
        spec_s3t1.pr_group
    );

    let anchor_task = note_value(&spec_s3t1.notes, "shared-pr-anchor")
        .or_else(|| note_value(&spec_s3t2.notes, "shared-pr-anchor"))
        .expect("shared-pr-anchor in auto single-lane notes");
    let (anchor_id, anchor_row, other_id, other_row) = if anchor_task == "S3T1" {
        ("S3T1", spec_s3t1, "S3T2", spec_s3t2)
    } else {
        ("S3T2", spec_s3t2, "S3T1", spec_s3t1)
    };

    assert_ne!(anchor_row.owner, other_row.owner);
    assert_ne!(anchor_row.branch, other_row.branch);
    assert_ne!(anchor_row.worktree, other_row.worktree);
    assert_ne!(anchor_row.notes, other_row.notes);

    let prompt_files = sprint_payload["payload"]["result"]["subagent_prompt_files"]
        .as_array()
        .expect("subagent prompt files");
    assert_eq!(prompt_files.len(), 2, "{}", start_sprint_out.stdout);
    let other_prompt_path = prompt_files
        .iter()
        .filter_map(|value| value.as_str())
        .find(|path| path.contains(&format!("{other_id}-subagent-prompt.md")))
        .expect("non-anchor prompt path");
    let other_prompt = fs::read_to_string(other_prompt_path).expect("read non-anchor prompt");
    let prompt_fields = parse_prompt_fields(&other_prompt);
    assert_eq!(
        prompt_fields.get("Task").map(String::as_str),
        Some(other_id),
        "{other_prompt}"
    );
    assert_eq!(
        prompt_fields.get("Owner").map(String::as_str),
        Some(other_row.owner.as_str()),
        "{other_prompt}"
    );
    assert_eq!(
        prompt_fields.get("Branch").map(String::as_str),
        Some(other_row.branch.as_str()),
        "{other_prompt}"
    );
    assert_eq!(
        prompt_fields.get("Worktree").map(String::as_str),
        Some(other_row.worktree.as_str()),
        "{other_prompt}"
    );
    assert_eq!(
        prompt_fields.get("Notes").map(String::as_str),
        Some(other_row.notes.as_str()),
        "{other_prompt}"
    );

    let issue_anchor = issue_rows.get(anchor_id).expect("anchor issue row");
    let issue_other = issue_rows.get(other_id).expect("non-anchor issue row");
    assert_eq!(issue_anchor.execution_mode, "TBD");
    assert_eq!(issue_other.execution_mode, "TBD");
    assert_eq!(issue_anchor.owner, "TBD");
    assert_eq!(issue_other.owner, "TBD");
    assert_eq!(issue_anchor.branch, "TBD");
    assert_eq!(issue_other.branch, "TBD");
    assert_eq!(issue_anchor.worktree, "TBD");
    assert_eq!(issue_other.worktree, "TBD");

    assert_ne!(other_row.owner, issue_other.owner);
    assert_ne!(other_row.branch, issue_other.branch);
    assert_ne!(other_row.worktree, issue_other.worktree);
    assert_eq!(other_row.notes, issue_other.notes);
}

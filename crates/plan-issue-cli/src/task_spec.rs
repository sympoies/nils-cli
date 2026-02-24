use std::path::{Path, PathBuf};
use std::process::Command;

use plan_tooling::parse::parse_plan_with_display;
use plan_tooling::split_prs::{
    SplitPlanOptions, SplitPrGrouping, SplitPrStrategy, SplitScope, build_split_plan_records,
    select_sprints_for_scope,
};

use crate::commands::{PrGroupMapping, PrGrouping, SplitStrategy};

pub const TASK_SPEC_HEADER: &str = "# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskSpecScope {
    Plan,
    Sprint(i32),
}

#[derive(Debug, Clone)]
pub struct TaskSpecBuildOptions {
    pub owner_prefix: String,
    pub branch_prefix: String,
    pub worktree_prefix: String,
    pub pr_grouping: PrGrouping,
    pub strategy: SplitStrategy,
    pub pr_group: Vec<PrGroupMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpecRow {
    pub task_id: String,
    pub summary: String,
    pub branch: String,
    pub worktree: String,
    pub owner: String,
    pub notes: String,
    pub pr_group: String,
    pub sprint: i32,
    pub grouping: PrGrouping,
}

#[derive(Debug, Clone)]
pub struct TaskSpecBuild {
    pub plan_title: String,
    pub display_plan_path: String,
    pub sprint_name: Option<String>,
    pub rows: Vec<TaskSpecRow>,
}

pub fn build_task_spec(
    plan_file: &Path,
    scope: TaskSpecScope,
    options: &TaskSpecBuildOptions,
) -> Result<TaskSpecBuild, String> {
    let display_path = plan_file.to_string_lossy().to_string();
    let resolved_plan_path = resolve_plan_file(plan_file);
    if !resolved_plan_path.is_file() {
        return Err(format!("plan file not found: {display_path}"));
    }

    let (plan, parse_errors) = parse_plan_with_display(&resolved_plan_path, &display_path)
        .map_err(|err| format!("{display_path}: {err}"))?;
    if !parse_errors.is_empty() {
        return Err(format!("{display_path}: {}", parse_errors.join(" | ")));
    }

    let split_scope = match scope {
        TaskSpecScope::Plan => SplitScope::Plan,
        TaskSpecScope::Sprint(sprint) => SplitScope::Sprint(sprint),
    };

    let selected_sprints = select_sprints_for_scope(&plan, split_scope)?;
    let sprint_name = match scope {
        TaskSpecScope::Plan => None,
        TaskSpecScope::Sprint(_) => selected_sprints.first().map(|sprint| sprint.name.clone()),
    };

    let split_options = SplitPlanOptions {
        pr_grouping: to_split_grouping(options.pr_grouping),
        strategy: to_split_strategy(options.strategy),
        pr_group_entries: options
            .pr_group
            .iter()
            .map(|entry| format!("{}={}", entry.task, entry.group))
            .collect(),
        owner_prefix: options.owner_prefix.clone(),
        branch_prefix: options.branch_prefix.clone(),
        worktree_prefix: options.worktree_prefix.clone(),
    };

    let rows = build_split_plan_records(&selected_sprints, &split_options)?
        .into_iter()
        .map(|record| TaskSpecRow {
            task_id: record.task_id,
            summary: record.summary,
            branch: record.branch,
            worktree: record.worktree,
            owner: record.owner,
            notes: record.notes,
            pr_group: record.pr_group,
            sprint: record.sprint,
            grouping: options.pr_grouping,
        })
        .collect();

    Ok(TaskSpecBuild {
        plan_title: plan.title,
        display_plan_path: display_path,
        sprint_name,
        rows,
    })
}

pub fn render_tsv(rows: &[TaskSpecRow]) -> String {
    let mut out = String::new();
    out.push_str(TASK_SPEC_HEADER);
    out.push('\n');
    for row in rows {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            row.task_id.replace('\t', " "),
            row.summary.replace('\t', " "),
            row.branch.replace('\t', " "),
            row.worktree.replace('\t', " "),
            row.owner.replace('\t', " "),
            row.notes.replace('\t', " "),
            row.pr_group.replace('\t', " "),
        ));
    }
    out
}

pub fn write_tsv(path: &Path, rows: &[TaskSpecRow]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create output directory {}: {err}",
                parent.display()
            )
        })?;
    }
    std::fs::write(path, render_tsv(rows))
        .map_err(|err| format!("failed to write task-spec {}: {err}", path.display()))
}

pub fn default_plan_task_spec_path(plan_file: &Path) -> PathBuf {
    let plan_stem = plan_file
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("plan")
        .to_string();

    agent_home()
        .join("out")
        .join("plan-issue-delivery-loop")
        .join(format!("{plan_stem}-plan-tasks.tsv"))
}

pub fn default_sprint_task_spec_path(plan_file: &Path, sprint: i32) -> PathBuf {
    let plan_stem = plan_file
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("plan")
        .to_string();

    agent_home()
        .join("out")
        .join("plan-issue-delivery-loop")
        .join(format!("{plan_stem}-sprint-{sprint}-tasks.tsv"))
}

pub fn agent_home() -> PathBuf {
    if let Ok(raw) = std::env::var("AGENT_HOME") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    detect_repo_root().join(".agents")
}

pub fn resolve_plan_file(plan_file: &Path) -> PathBuf {
    let repo_root = detect_repo_root();
    resolve_repo_relative(&repo_root, plan_file)
}

fn detect_repo_root() -> PathBuf {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output();
    if let Ok(out) = output
        && out.status.success()
    {
        let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !raw.is_empty() {
            return PathBuf::from(raw);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_repo_relative(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    repo_root.join(path)
}

fn to_split_grouping(grouping: PrGrouping) -> SplitPrGrouping {
    match grouping {
        PrGrouping::PerSprint => SplitPrGrouping::PerSprint,
        PrGrouping::Group => SplitPrGrouping::Group,
    }
}

fn to_split_strategy(strategy: SplitStrategy) -> SplitPrStrategy {
    match strategy {
        SplitStrategy::Deterministic => SplitPrStrategy::Deterministic,
        SplitStrategy::Auto => SplitPrStrategy::Auto,
    }
}

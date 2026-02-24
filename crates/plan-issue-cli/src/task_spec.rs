use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use plan_tooling::parse::{Plan, Sprint, parse_plan_with_display};

use crate::commands::{PrGroupMapping, PrGrouping};

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

#[derive(Debug, Clone)]
struct WorkingRecord {
    task_id: String,
    plan_task_id: String,
    sprint: i32,
    summary: String,
    branch: String,
    worktree: String,
    owner: String,
    notes_parts: Vec<String>,
    pr_group: String,
}

pub fn build_task_spec(
    plan_file: &Path,
    scope: TaskSpecScope,
    options: &TaskSpecBuildOptions,
) -> Result<TaskSpecBuild, String> {
    let repo_root = detect_repo_root();
    let display_path = plan_file.to_string_lossy().to_string();
    let resolved_plan_path = resolve_repo_relative(&repo_root, plan_file);
    if !resolved_plan_path.is_file() {
        return Err(format!("plan file not found: {display_path}"));
    }

    let (plan, parse_errors) = parse_plan_with_display(&resolved_plan_path, &display_path)
        .map_err(|err| format!("{display_path}: {err}"))?;
    if !parse_errors.is_empty() {
        return Err(format!("{display_path}: {}", parse_errors.join(" | ")));
    }

    let (selected_sprints, sprint_name) = select_sprints(&plan, scope)?;

    let mut records: Vec<WorkingRecord> = Vec::new();
    for sprint in selected_sprints {
        for (idx, task) in sprint.tasks.iter().enumerate() {
            let ordinal = idx + 1;
            let task_id = format!("S{}T{ordinal}", sprint.number);
            let plan_task_id = task.id.trim().to_string();
            let summary = normalize_spaces(if task.name.trim().is_empty() {
                if plan_task_id.is_empty() {
                    format!("sprint-{}-task-{ordinal}", sprint.number)
                } else {
                    plan_task_id.clone()
                }
            } else {
                task.name.trim().to_string()
            });

            let slug = normalize_token(&summary, &format!("task-{ordinal}"), 48);
            let branch_prefix = normalize_branch_prefix(&options.branch_prefix);
            let worktree_prefix = normalize_worktree_prefix(&options.worktree_prefix);
            let owner_prefix = normalize_owner_prefix(&options.owner_prefix);

            let deps: Vec<String> = task
                .dependencies
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty())
                .filter(|d| !is_placeholder(d))
                .collect();

            let validations: Vec<String> = task
                .validation
                .iter()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .filter(|v| !is_placeholder(v))
                .collect();

            let mut notes_parts = vec![
                format!("sprint=S{}", sprint.number),
                format!(
                    "plan-task:{}",
                    if plan_task_id.is_empty() {
                        task_id.clone()
                    } else {
                        plan_task_id.clone()
                    }
                ),
            ];
            if !deps.is_empty() {
                notes_parts.push(format!("deps={}", deps.join(",")));
            }
            if let Some(first) = validations.first() {
                notes_parts.push(format!("validate={first}"));
            }

            records.push(WorkingRecord {
                task_id,
                plan_task_id,
                sprint: sprint.number,
                summary,
                branch: format!("{branch_prefix}/s{}-t{ordinal}-{slug}", sprint.number),
                worktree: format!("{worktree_prefix}-s{}-t{ordinal}", sprint.number),
                owner: format!("{owner_prefix}-s{}-t{ordinal}", sprint.number),
                notes_parts,
                pr_group: String::new(),
            });
        }
    }

    if records.is_empty() {
        return Err(format!("{display_path}: selected scope has no tasks"));
    }

    apply_pr_groups(&mut records, options)?;

    let mut group_sizes: HashMap<String, usize> = HashMap::new();
    let mut group_anchor: HashMap<String, String> = HashMap::new();
    for rec in &records {
        let size = group_sizes.entry(rec.pr_group.clone()).or_insert(0);
        *size += 1;
        group_anchor
            .entry(rec.pr_group.clone())
            .or_insert_with(|| rec.task_id.clone());
    }

    let mut rows: Vec<TaskSpecRow> = Vec::new();
    for rec in records {
        let mut notes = rec.notes_parts.clone();
        notes.push(format!(
            "pr-grouping={}",
            grouping_label(options.pr_grouping)
        ));
        notes.push(format!("pr-group={}", rec.pr_group));
        if group_sizes.get(&rec.pr_group).copied().unwrap_or(0) > 1
            && let Some(anchor) = group_anchor.get(&rec.pr_group)
        {
            notes.push(format!("shared-pr-anchor={anchor}"));
        }

        rows.push(TaskSpecRow {
            task_id: rec.task_id,
            summary: rec.summary,
            branch: rec.branch,
            worktree: rec.worktree,
            owner: rec.owner,
            notes: notes.join("; "),
            pr_group: rec.pr_group,
            sprint: rec.sprint,
            grouping: options.pr_grouping,
        });
    }

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

fn apply_pr_groups(
    records: &mut [WorkingRecord],
    options: &TaskSpecBuildOptions,
) -> Result<(), String> {
    let mut group_assignments: HashMap<String, String> = HashMap::new();
    let mut assignment_sources: Vec<String> = Vec::new();
    for entry in &options.pr_group {
        let key = entry.task.trim();
        let group = normalize_token(entry.group.trim(), "", 48);
        if key.is_empty() || group.is_empty() {
            return Err("--pr-group must include both task key and group".to_string());
        }
        assignment_sources.push(key.to_string());
        group_assignments.insert(key.to_ascii_lowercase(), group);
    }

    if options.pr_grouping == PrGrouping::Group {
        let mut known: HashMap<String, bool> = HashMap::new();
        for rec in records.iter() {
            known.insert(rec.task_id.to_ascii_lowercase(), true);
            if !rec.plan_task_id.is_empty() {
                known.insert(rec.plan_task_id.to_ascii_lowercase(), true);
            }
        }

        let unknown: Vec<String> = assignment_sources
            .iter()
            .filter(|key| !known.contains_key(&key.to_ascii_lowercase()))
            .cloned()
            .collect();
        if !unknown.is_empty() {
            return Err(format!(
                "--pr-group references unknown task keys: {}",
                unknown
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }

    if options.pr_grouping == PrGrouping::Group {
        let mut missing: Vec<String> = Vec::new();
        for rec in records.iter_mut() {
            let mut found = String::new();
            for key in [&rec.task_id, &rec.plan_task_id] {
                if key.is_empty() {
                    continue;
                }
                if let Some(v) = group_assignments.get(&key.to_ascii_lowercase()) {
                    found = v.to_string();
                    break;
                }
            }
            if found.is_empty() {
                missing.push(rec.task_id.clone());
            } else {
                rec.pr_group = found;
            }
        }
        if !missing.is_empty() {
            return Err(format!(
                "--pr-grouping group requires mapping for every task; missing: {}",
                missing
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    } else {
        for rec in records.iter_mut() {
            rec.pr_group =
                normalize_token(&format!("s{}", rec.sprint), &format!("s{}", rec.sprint), 48);
        }
    }

    Ok(())
}

fn select_sprints(
    plan: &Plan,
    scope: TaskSpecScope,
) -> Result<(Vec<&Sprint>, Option<String>), String> {
    match scope {
        TaskSpecScope::Plan => {
            let selected: Vec<&Sprint> = plan
                .sprints
                .iter()
                .filter(|sprint| !sprint.tasks.is_empty())
                .collect();
            if selected.is_empty() {
                return Err("selected scope has no tasks".to_string());
            }
            Ok((selected, None))
        }
        TaskSpecScope::Sprint(want) => match plan.sprints.iter().find(|s| s.number == want) {
            Some(sprint) if !sprint.tasks.is_empty() => {
                Ok((vec![sprint], Some(sprint.name.clone())))
            }
            Some(_) => Err(format!("sprint {want} has no tasks")),
            None => Err(format!("sprint not found: {want}")),
        },
    }
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

fn normalize_branch_prefix(value: &str) -> &str {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() { "issue" } else { trimmed }
}

fn normalize_worktree_prefix(value: &str) -> &str {
    let trimmed = value.trim().trim_end_matches(['-', '_']);
    if trimmed.is_empty() { "issue" } else { trimmed }
}

fn normalize_owner_prefix(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::from("subagent");
    }
    if trimmed.to_ascii_lowercase().contains("subagent") {
        return trimmed.to_string();
    }
    format!("subagent-{trimmed}")
}

fn normalize_spaces(value: String) -> String {
    let joined = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        String::from("task")
    } else {
        joined
    }
}

fn normalize_token(value: &str, fallback: &str, max_len: usize) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }

    let normalized = out.trim_matches('-').to_string();
    let mut token = if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    };

    if token.len() > max_len {
        token.truncate(max_len);
        token = token.trim_matches('-').to_string();
    }

    token
}

fn is_placeholder(value: &str) -> bool {
    let token = value.trim().to_ascii_lowercase();
    if matches!(token.as_str(), "" | "-" | "none" | "n/a" | "na" | "...") {
        return true;
    }
    if token.starts_with('<') && token.ends_with('>') {
        return true;
    }
    token.contains("task ids")
}

fn grouping_label(grouping: PrGrouping) -> &'static str {
    match grouping {
        PrGrouping::PerSprint => "per-sprint",
        PrGrouping::Group => "group",
    }
}

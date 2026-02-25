use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use nils_common::{env as common_env, fs as common_fs, git as common_git};
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
    common_fs::write_text(path, &render_tsv(rows)).map_err(|err| match err {
        common_fs::WriteTextError::CreateParentDir { path, source } => {
            format!(
                "failed to create output directory {}: {source}",
                path.display()
            )
        }
        common_fs::WriteTextError::WriteFile { source, .. } => {
            format!("failed to write task-spec {}: {source}", path.display())
        }
    })
}

pub fn execution_mode_by_task(
    rows: &[TaskSpecRow],
    strategy: SplitStrategy,
) -> HashMap<String, String> {
    let mut sprint_group_set: HashMap<i32, BTreeSet<String>> = HashMap::new();
    let mut sprint_group_sizes: HashMap<(i32, String), usize> = HashMap::new();
    for row in rows {
        sprint_group_set
            .entry(row.sprint)
            .or_default()
            .insert(row.pr_group.clone());
        *sprint_group_sizes
            .entry((row.sprint, row.pr_group.clone()))
            .or_insert(0) += 1;
    }

    let mut out = HashMap::new();
    for row in rows {
        let sprint_group_count = sprint_group_set
            .get(&row.sprint)
            .map(BTreeSet::len)
            .unwrap_or(0);
        let group_size = sprint_group_sizes
            .get(&(row.sprint, row.pr_group.clone()))
            .copied()
            .unwrap_or(0);

        let mode = if row.grouping == PrGrouping::PerSprint {
            "per-sprint"
        } else if strategy == SplitStrategy::Auto && sprint_group_count == 1 && group_size > 1 {
            // Auto/group can converge to a single shared PR lane. Expose that as per-sprint so
            // downstream execution semantics match explicit per-sprint mode.
            "per-sprint"
        } else if group_size > 1 {
            "pr-shared"
        } else {
            "pr-isolated"
        };
        out.insert(row.task_id.clone(), mode.to_string());
    }

    out
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
    if let Some(agent_home) = common_env::env_non_empty("AGENT_HOME") {
        return PathBuf::from(agent_home);
    }
    detect_repo_root().join(".agents")
}

pub fn resolve_plan_file(plan_file: &Path) -> PathBuf {
    let repo_root = detect_repo_root();
    resolve_repo_relative(&repo_root, plan_file)
}

fn detect_repo_root() -> PathBuf {
    common_git::repo_root_or_cwd()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::too_many_arguments)]
    fn spec_row(
        task_id: &str,
        sprint: i32,
        pr_group: &str,
        grouping: PrGrouping,
        owner: &str,
        branch: &str,
        worktree: &str,
        notes: &str,
    ) -> TaskSpecRow {
        TaskSpecRow {
            task_id: task_id.to_string(),
            summary: format!("Summary for {task_id}"),
            branch: branch.to_string(),
            worktree: worktree.to_string(),
            owner: owner.to_string(),
            notes: notes.to_string(),
            pr_group: pr_group.to_string(),
            sprint,
            grouping,
        }
    }

    fn note_value(notes: &str, key: &str) -> Option<String> {
        notes
            .split(';')
            .map(str::trim)
            .find_map(|part| part.strip_prefix(&format!("{key}=")).map(str::to_string))
    }

    fn canonical_lane_anchor_task_id(
        rows: &[TaskSpecRow],
        sprint: i32,
        pr_group: &str,
    ) -> Option<String> {
        let lane_rows = rows
            .iter()
            .filter(|row| row.sprint == sprint && row.pr_group == pr_group)
            .collect::<Vec<_>>();
        if lane_rows.is_empty() {
            return None;
        }

        if let Some(anchor) = lane_rows
            .iter()
            .find_map(|row| note_value(&row.notes, "shared-pr-anchor"))
            .filter(|anchor| lane_rows.iter().any(|row| row.task_id == *anchor))
        {
            return Some(anchor);
        }

        lane_rows.iter().map(|row| row.task_id.clone()).min()
    }

    #[test]
    fn execution_mode_by_task_auto_single_lane_uses_per_sprint() {
        let rows = vec![
            spec_row(
                "S1T1",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t1",
                "issue/s1-t1",
                "wt-1",
                "sprint=S1; plan-task:Task 1.1; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
            spec_row(
                "S1T2",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t2",
                "issue/s1-t2",
                "wt-2",
                "sprint=S1; plan-task:Task 1.2; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
        ];

        let modes = execution_mode_by_task(&rows, SplitStrategy::Auto);
        assert_eq!(modes.get("S1T1").map(String::as_str), Some("per-sprint"));
        assert_eq!(modes.get("S1T2").map(String::as_str), Some("per-sprint"));
    }

    #[test]
    fn execution_mode_by_task_auto_multi_group_keeps_group_modes() {
        let rows = vec![
            spec_row(
                "S2T1",
                2,
                "s2-auto-g1",
                PrGrouping::Group,
                "subagent-s2-t1",
                "issue/s2-t1",
                "wt-1",
                "sprint=S2; plan-task:Task 2.1; pr-group=s2-auto-g1",
            ),
            spec_row(
                "S2T2",
                2,
                "s2-auto-g1",
                PrGrouping::Group,
                "subagent-s2-t2",
                "issue/s2-t2",
                "wt-2",
                "sprint=S2; plan-task:Task 2.2; pr-group=s2-auto-g1",
            ),
            spec_row(
                "S2T3",
                2,
                "s2-auto-g2",
                PrGrouping::Group,
                "subagent-s2-t3",
                "issue/s2-t3",
                "wt-3",
                "sprint=S2; plan-task:Task 2.3; pr-group=s2-auto-g2",
            ),
        ];

        let modes = execution_mode_by_task(&rows, SplitStrategy::Auto);
        assert_eq!(modes.get("S2T1").map(String::as_str), Some("pr-shared"));
        assert_eq!(modes.get("S2T2").map(String::as_str), Some("pr-shared"));
        assert_eq!(modes.get("S2T3").map(String::as_str), Some("pr-isolated"));
    }

    #[test]
    fn canonical_lane_anchor_prefers_shared_pr_anchor_note() {
        let rows = vec![
            spec_row(
                "S1T1",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t1",
                "issue/s1-t1",
                "wt-1",
                "sprint=S1; plan-task:Task 1.1; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
            spec_row(
                "S1T2",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t2",
                "issue/s1-t2",
                "wt-2",
                "sprint=S1; plan-task:Task 1.2; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
        ];

        assert_eq!(
            canonical_lane_anchor_task_id(&rows, 1, "s1-auto-g1"),
            Some("S1T2".to_string())
        );
    }

    #[test]
    fn canonical_lane_anchor_uses_deterministic_task_id_fallback_when_note_absent() {
        let rows = vec![
            spec_row(
                "S4T3",
                4,
                "s4-auto-g2",
                PrGrouping::Group,
                "subagent-s4-t3",
                "issue/s4-t3",
                "wt-3",
                "sprint=S4; plan-task:Task 4.3; pr-group=s4-auto-g2",
            ),
            spec_row(
                "S4T1",
                4,
                "s4-auto-g2",
                PrGrouping::Group,
                "subagent-s4-t1",
                "issue/s4-t1",
                "wt-1",
                "sprint=S4; plan-task:Task 4.1; pr-group=s4-auto-g2",
            ),
            spec_row(
                "S4T2",
                4,
                "s4-auto-g2",
                PrGrouping::Group,
                "subagent-s4-t2",
                "issue/s4-t2",
                "wt-2",
                "sprint=S4; plan-task:Task 4.2; pr-group=s4-auto-g2",
            ),
        ];

        assert_eq!(
            canonical_lane_anchor_task_id(&rows, 4, "s4-auto-g2"),
            Some("S4T1".to_string())
        );
    }

    #[test]
    #[ignore = "S2 lane canonicalization should rewrite single-lane metadata to anchor values"]
    fn auto_single_lane_canonicalization_expectations_document_anchor_invariants() {
        let rows = vec![
            spec_row(
                "S1T1",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t1",
                "issue/s1-t1",
                "wt-1",
                "sprint=S1; plan-task:Task 1.1; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
            spec_row(
                "S1T2",
                1,
                "s1-auto-g1",
                PrGrouping::Group,
                "subagent-s1-t2",
                "issue/s1-t2",
                "wt-2",
                "sprint=S1; plan-task:Task 1.2; pr-group=s1-auto-g1; shared-pr-anchor=S1T2",
            ),
        ];
        let modes = execution_mode_by_task(&rows, SplitStrategy::Auto);
        assert_eq!(modes.get("S1T1").map(String::as_str), Some("per-sprint"));
        assert_eq!(modes.get("S1T2").map(String::as_str), Some("per-sprint"));

        let anchor_task =
            canonical_lane_anchor_task_id(&rows, 1, "s1-auto-g1").expect("anchor task");
        let anchor_row = rows
            .iter()
            .find(|row| row.task_id == anchor_task)
            .expect("anchor row");
        let anchor_owner = anchor_row.owner.clone();
        let anchor_branch = anchor_row.branch.clone();
        let anchor_worktree = anchor_row.worktree.clone();
        let anchor_notes = anchor_row.notes.clone();

        for row in rows
            .iter()
            .filter(|row| row.sprint == 1 && row.pr_group == "s1-auto-g1")
        {
            assert_eq!(
                row.owner, anchor_owner,
                "task {} owner should match anchor",
                row.task_id
            );
            assert_eq!(
                row.branch, anchor_branch,
                "task {} branch should match anchor",
                row.task_id
            );
            assert_eq!(
                row.worktree, anchor_worktree,
                "task {} worktree should match anchor",
                row.task_id
            );
            assert_eq!(
                row.notes, anchor_notes,
                "task {} notes should match anchor",
                row.task_id
            );
        }
    }
}

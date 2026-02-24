use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRow {
    pub task: String,
    pub summary: String,
    pub owner: String,
    pub branch: String,
    pub worktree: String,
    pub execution_mode: String,
    pub pr: String,
    pub status: String,
    pub notes: String,
    pub line_index: usize,
}

#[derive(Debug, Clone)]
pub struct TaskTable {
    lines: Vec<String>,
    rows: Vec<TaskRow>,
    trailing_newline: bool,
}

impl TaskTable {
    pub fn rows(&self) -> &[TaskRow] {
        &self.rows
    }

    pub fn rows_mut(&mut self) -> &mut [TaskRow] {
        &mut self.rows
    }

    pub fn sprint_row_indexes(&self, sprint: i32) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(idx, row)| (row_sprint(row) == Some(sprint)).then_some(idx))
            .collect()
    }

    pub fn render(&self) -> String {
        let mut lines = self.lines.clone();
        for row in &self.rows {
            lines[row.line_index] = format_markdown_row(row);
        }

        let mut rendered = lines.join("\n");
        if self.trailing_newline {
            rendered.push('\n');
        }
        rendered
    }
}

pub fn parse_task_table(body: &str) -> Result<TaskTable, String> {
    let trailing_newline = body.ends_with('\n');
    let lines: Vec<String> = body.lines().map(ToString::to_string).collect();

    let section_idx = lines
        .iter()
        .position(|line| line.trim() == "## Task Decomposition")
        .ok_or_else(|| "issue body missing `## Task Decomposition` section".to_string())?;

    let mut header_idx = None;
    for (idx, line) in lines.iter().enumerate().skip(section_idx + 1) {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            break;
        }
        if trimmed.starts_with('|') {
            let cells = split_table_cells(trimmed);
            if normalize_header_cells(&cells) {
                header_idx = Some(idx);
                break;
            }
        }
    }

    let header_idx = header_idx.ok_or_else(|| {
        "task decomposition table header not found or does not match expected columns".to_string()
    })?;

    let separator_idx = header_idx + 1;
    if separator_idx >= lines.len() || !lines[separator_idx].trim().starts_with("| ---") {
        return Err("task decomposition table separator row is missing".to_string());
    }

    let mut rows = Vec::new();
    for (idx, line) in lines.iter().enumerate().skip(separator_idx + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("## ") || !trimmed.starts_with('|') {
            break;
        }

        let cells = split_table_cells(trimmed);
        if cells.len() != 9 {
            return Err(format!(
                "task decomposition row has {} columns (expected 9): {}",
                cells.len(),
                trimmed
            ));
        }

        rows.push(TaskRow {
            task: cells[0].clone(),
            summary: cells[1].clone(),
            owner: cells[2].clone(),
            branch: cells[3].clone(),
            worktree: cells[4].clone(),
            execution_mode: cells[5].clone(),
            pr: cells[6].clone(),
            status: cells[7].clone(),
            notes: cells[8].clone(),
            line_index: idx,
        });
    }

    if rows.is_empty() {
        return Err("task decomposition table has no task rows".to_string());
    }

    Ok(TaskTable {
        lines,
        rows,
        trailing_newline,
    })
}

pub fn validate_rows(rows: &[TaskRow]) -> Vec<String> {
    let mut errors = Vec::new();

    let mut per_task_branches: HashMap<String, String> = HashMap::new();
    let mut per_task_worktrees: HashMap<String, String> = HashMap::new();

    for row in rows {
        let status = row.status.trim().to_ascii_lowercase();
        if !matches!(
            status.as_str(),
            "planned" | "in-progress" | "blocked" | "done"
        ) {
            errors.push(format!(
                "{}: invalid Status `{}`",
                row.task,
                row.status.trim()
            ));
        }

        let execution_mode = row.execution_mode.trim().to_ascii_lowercase();
        if !matches!(
            execution_mode.as_str(),
            "per-task" | "per-sprint" | "pr-isolated" | "pr-shared" | "tbd"
        ) {
            errors.push(format!(
                "{}: invalid Execution Mode `{}`",
                row.task,
                row.execution_mode.trim()
            ));
        }

        if matches!(status.as_str(), "in-progress" | "done") {
            for (label, value) in [
                ("Owner", row.owner.as_str()),
                ("Branch", row.branch.as_str()),
                ("Worktree", row.worktree.as_str()),
                ("Execution Mode", row.execution_mode.as_str()),
                ("PR", row.pr.as_str()),
            ] {
                if is_placeholder(value) {
                    errors.push(format!(
                        "{}: Status `{}` requires non-placeholder {}",
                        row.task, status, label
                    ));
                }
            }
        }

        if !matches!(status.as_str(), "planned" | "blocked") {
            let owner = row.owner.trim().to_ascii_lowercase();
            if !owner.contains("subagent") {
                errors.push(format!(
                    "{}: Owner must include `subagent` for status `{}`",
                    row.task, status
                ));
            }
            if owner.contains("main-agent") {
                errors.push(format!(
                    "{}: Owner cannot reference main-agent for status `{}`",
                    row.task, status
                ));
            }
        }

        if execution_mode == "per-task" {
            if !is_placeholder(&row.branch) {
                let key = row.branch.trim().to_ascii_lowercase();
                if let Some(prev_task) = per_task_branches.insert(key.clone(), row.task.clone()) {
                    errors.push(format!(
                        "{}: per-task Branch `{}` duplicates task {}",
                        row.task,
                        row.branch.trim(),
                        prev_task
                    ));
                }
            }

            if !is_placeholder(&row.worktree) {
                let key = row.worktree.trim().to_ascii_lowercase();
                if let Some(prev_task) = per_task_worktrees.insert(key.clone(), row.task.clone()) {
                    errors.push(format!(
                        "{}: per-task Worktree `{}` duplicates task {}",
                        row.task,
                        row.worktree.trim(),
                        prev_task
                    ));
                }
            }
        }
    }

    errors
}

pub fn row_sprint(row: &TaskRow) -> Option<i32> {
    for token in row.notes.split(';').map(str::trim) {
        if let Some(value) = token.strip_prefix("sprint=S")
            && let Ok(number) = value.trim().parse::<i32>()
        {
            return Some(number);
        }
    }

    parse_sprint_from_task_id(&row.task)
}

pub fn parse_pr_number(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if is_placeholder(trimmed) {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix('#') {
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        return digits.parse::<u64>().ok();
    }

    if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
        return trimmed.parse::<u64>().ok();
    }

    if let Some((_, tail)) = trimmed.rsplit_once("/pull/") {
        let digits: String = tail.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        return digits.parse::<u64>().ok();
    }

    None
}

pub fn normalize_pr_display(value: &str) -> String {
    parse_pr_number(value)
        .map(|pr| format!("#{pr}"))
        .unwrap_or_else(|| value.trim().to_string())
}

pub fn is_placeholder(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return true;
    }

    if matches!(normalized.as_str(), "-" | "none" | "null" | "n/a" | "?") {
        return true;
    }

    normalized.starts_with("tbd")
}

fn normalize_header_cells(cells: &[String]) -> bool {
    let expected = [
        "task",
        "summary",
        "owner",
        "branch",
        "worktree",
        "execution mode",
        "pr",
        "status",
        "notes",
    ];

    if cells.len() != expected.len() {
        return false;
    }

    cells
        .iter()
        .zip(expected)
        .all(|(cell, expected)| cell.trim().eq_ignore_ascii_case(expected))
}

fn split_table_cells(line: &str) -> Vec<String> {
    let mut cells: Vec<String> = line
        .trim()
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect();

    if cells.first().is_some_and(|cell| cell.is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|cell| cell.is_empty()) {
        cells.pop();
    }

    cells
}

fn parse_sprint_from_task_id(task: &str) -> Option<i32> {
    let normalized = task.trim();
    if !normalized.starts_with('S') {
        return None;
    }

    let rest = &normalized[1..];
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }

    if !rest[digits.len()..].starts_with('T') {
        return None;
    }

    digits.parse::<i32>().ok()
}

fn sanitize_table_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ").replace('|', "/")
}

fn format_markdown_row(row: &TaskRow) -> String {
    format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        sanitize_table_value(&row.task),
        sanitize_table_value(&row.summary),
        sanitize_table_value(&row.owner),
        sanitize_table_value(&row.branch),
        sanitize_table_value(&row.worktree),
        sanitize_table_value(&row.execution_mode),
        sanitize_table_value(&row.pr),
        sanitize_table_value(&row.status),
        sanitize_table_value(&row.notes),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        TaskRow, is_placeholder, normalize_pr_display, parse_pr_number, parse_task_table,
        row_sprint, validate_rows,
    };

    #[test]
    fn parse_task_table_extracts_rows() {
        let body = "## Task Decomposition\n\n| Task | Summary | Owner | Branch | Worktree | Execution Mode | PR | Status | Notes |\n| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n| S4T1 | A | subagent | issue/s4 | issue-s4 | per-sprint | #1 | done | sprint=S4 |\n";
        let table = parse_task_table(body).expect("parse table");
        assert_eq!(table.rows().len(), 1);
        assert_eq!(table.rows()[0].task, "S4T1");
    }

    #[test]
    fn row_sprint_prefers_notes_then_task_id() {
        let row = TaskRow {
            task: "S2T1".to_string(),
            summary: String::new(),
            owner: String::new(),
            branch: String::new(),
            worktree: String::new(),
            execution_mode: String::new(),
            pr: String::new(),
            status: String::new(),
            notes: "x=1; sprint=S9".to_string(),
            line_index: 0,
        };
        assert_eq!(row_sprint(&row), Some(9));
    }

    #[test]
    fn placeholder_and_pr_normalization_cover_common_cases() {
        assert!(is_placeholder("TBD (per-sprint)"));
        assert_eq!(parse_pr_number("#221"), Some(221));
        assert_eq!(
            parse_pr_number("https://github.com/graysurf/nils-cli/pull/221"),
            Some(221)
        );
        assert_eq!(
            normalize_pr_display("https://github.com/x/y/pull/17"),
            "#17"
        );
    }

    #[test]
    fn validate_rows_flags_non_subagent_owner_for_done_rows() {
        let rows = [TaskRow {
            task: "S4T1".to_string(),
            summary: "x".to_string(),
            owner: "main-agent".to_string(),
            branch: "issue/s4".to_string(),
            worktree: "issue-s4".to_string(),
            execution_mode: "per-sprint".to_string(),
            pr: "#1".to_string(),
            status: "done".to_string(),
            notes: "sprint=S4".to_string(),
            line_index: 0,
        }];
        let errs = validate_rows(&rows);
        assert!(!errs.is_empty());
    }
}

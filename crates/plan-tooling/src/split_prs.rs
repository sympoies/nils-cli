use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::parse::{Plan, Sprint, parse_plan_with_display};

const USAGE: &str = r#"Usage:
  plan-tooling split-prs --file <plan.md> --pr-grouping <per-sprint|group> [options]

Purpose:
  Build deterministic task-to-PR split records from a Plan Format v1 file.

Required:
  --file <path>                    Plan file to parse
  --pr-grouping <mode>             per-sprint | group

Options:
  --scope <plan|sprint>            Scope to split (default: sprint)
  --sprint <n>                     Sprint number when --scope sprint
  --pr-group <task=group>          Explicit mapping; repeatable (group mode only)
  --strategy <deterministic|auto>  Split strategy (default: deterministic)
  --owner-prefix <text>            Owner prefix (default: subagent)
  --branch-prefix <text>           Branch prefix (default: issue)
  --worktree-prefix <text>         Worktree prefix (default: issue__)
  --format <json|tsv>              Output format (default: json)
  -h, --help                       Show help

Exit:
  0: success
  1: runtime or validation error
  2: usage error
"#;

#[derive(Debug, Clone)]
struct Record {
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

#[derive(Debug, Serialize)]
struct Output {
    file: String,
    scope: String,
    sprint: Option<i32>,
    pr_grouping: String,
    strategy: String,
    records: Vec<OutputRecord>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct OutputRecord {
    task_id: String,
    summary: String,
    branch: String,
    worktree: String,
    owner: String,
    notes: String,
    pr_group: String,
}

pub fn run(args: &[String]) -> i32 {
    let mut file: Option<String> = None;
    let mut scope = String::from("sprint");
    let mut sprint: Option<String> = None;
    let mut pr_grouping: Option<String> = None;
    let mut pr_group_entries: Vec<String> = Vec::new();
    let mut strategy = String::from("deterministic");
    let mut owner_prefix = String::from("subagent");
    let mut branch_prefix = String::from("issue");
    let mut worktree_prefix = String::from("issue__");
    let mut format = String::from("json");

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --file");
                };
                if v.is_empty() {
                    return die("missing value for --file");
                }
                file = Some(v.to_string());
                i += 2;
            }
            "--scope" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --scope");
                };
                if v.is_empty() {
                    return die("missing value for --scope");
                }
                scope = v.to_string();
                i += 2;
            }
            "--sprint" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --sprint");
                };
                if v.is_empty() {
                    return die("missing value for --sprint");
                }
                sprint = Some(v.to_string());
                i += 2;
            }
            "--pr-grouping" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --pr-grouping");
                };
                if v.is_empty() {
                    return die("missing value for --pr-grouping");
                }
                pr_grouping = Some(v.to_string());
                i += 2;
            }
            "--pr-group" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --pr-group");
                };
                if v.is_empty() {
                    return die("missing value for --pr-group");
                }
                pr_group_entries.push(v.to_string());
                i += 2;
            }
            "--strategy" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --strategy");
                };
                if v.is_empty() {
                    return die("missing value for --strategy");
                }
                strategy = v.to_string();
                i += 2;
            }
            "--owner-prefix" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --owner-prefix");
                };
                if v.is_empty() {
                    return die("missing value for --owner-prefix");
                }
                owner_prefix = v.to_string();
                i += 2;
            }
            "--branch-prefix" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --branch-prefix");
                };
                if v.is_empty() {
                    return die("missing value for --branch-prefix");
                }
                branch_prefix = v.to_string();
                i += 2;
            }
            "--worktree-prefix" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --worktree-prefix");
                };
                if v.is_empty() {
                    return die("missing value for --worktree-prefix");
                }
                worktree_prefix = v.to_string();
                i += 2;
            }
            "--format" => {
                let Some(v) = args.get(i + 1) else {
                    return die("missing value for --format");
                };
                if v.is_empty() {
                    return die("missing value for --format");
                }
                format = v.to_string();
                i += 2;
            }
            "-h" | "--help" => {
                print_usage();
                return 0;
            }
            other => {
                return die(&format!("unknown argument: {other}"));
            }
        }
    }

    let Some(file_arg) = file else {
        print_usage();
        return 2;
    };
    let Some(mut pr_grouping) = pr_grouping else {
        print_usage();
        return 2;
    };

    if pr_grouping == "per-spring" {
        pr_grouping = String::from("per-sprint");
    }
    if scope != "plan" && scope != "sprint" {
        return die(&format!(
            "invalid --scope (expected plan|sprint): {}",
            crate::repr::py_repr(&scope)
        ));
    }
    if pr_grouping != "per-sprint" && pr_grouping != "group" {
        return die(&format!(
            "invalid --pr-grouping (expected per-sprint|group): {}",
            crate::repr::py_repr(&pr_grouping)
        ));
    }
    if strategy != "deterministic" && strategy != "auto" {
        return die(&format!(
            "invalid --strategy (expected deterministic|auto): {}",
            crate::repr::py_repr(&strategy)
        ));
    }
    if format != "json" && format != "tsv" {
        return die(&format!(
            "invalid --format (expected json|tsv): {}",
            crate::repr::py_repr(&format)
        ));
    }

    let sprint_num = if scope == "sprint" {
        let Some(raw) = sprint.as_deref() else {
            return die("--sprint is required when --scope sprint");
        };
        match raw.parse::<i32>() {
            Ok(v) if v > 0 => Some(v),
            _ => {
                eprintln!(
                    "error: invalid --sprint (expected positive int): {}",
                    crate::repr::py_repr(raw)
                );
                return 2;
            }
        }
    } else {
        None
    };

    // Sprint 1 contract freeze for future auto behavior:
    // - scoring inputs: Complexity, dependency layers, and Location overlap
    // - optional `--pr-group` entries in group mode act as pinned assignments
    // - deterministic tie-break keys: Task N.M, then SxTy, then lexical summary
    // Runtime intentionally remains disabled until the auto assignment engine lands.
    if strategy == "auto" {
        eprintln!(
            "error: split-prs strategy 'auto' is not implemented yet (planned factors: Complexity, Location, Dependencies)"
        );
        return 1;
    }

    if pr_grouping == "group" && pr_group_entries.is_empty() {
        return die(
            "--pr-grouping group requires at least one --pr-group <task-or-plan-id>=<group> entry",
        );
    }
    if pr_grouping != "group" && !pr_group_entries.is_empty() {
        return die("--pr-group can only be used when --pr-grouping group");
    }

    let repo_root = crate::repo_root::detect();
    let display_path = file_arg.clone();
    let read_path = resolve_repo_relative(&repo_root, Path::new(&file_arg));
    if !read_path.is_file() {
        eprintln!("error: plan file not found: {display_path}");
        return 1;
    }

    let plan: Plan;
    let parse_errors: Vec<String>;
    match parse_plan_with_display(&read_path, &display_path) {
        Ok((p, errs)) => {
            plan = p;
            parse_errors = errs;
        }
        Err(err) => {
            eprintln!("error: {display_path}: {err}");
            return 1;
        }
    }
    if !parse_errors.is_empty() {
        for err in parse_errors {
            eprintln!("error: {display_path}: error: {err}");
        }
        return 1;
    }

    let selected_sprints: Vec<&Sprint> = if scope == "plan" {
        plan.sprints
            .iter()
            .filter(|s| !s.tasks.is_empty())
            .collect()
    } else {
        let Some(want) = sprint_num else {
            return die("internal error: missing sprint number");
        };
        match plan.sprints.iter().find(|s| s.number == want) {
            Some(sprint) if !sprint.tasks.is_empty() => vec![sprint],
            Some(_) => {
                eprintln!("error: {display_path}: sprint {want} has no tasks");
                return 1;
            }
            None => {
                eprintln!("error: {display_path}: sprint not found: {want}");
                return 1;
            }
        }
    };

    if selected_sprints.is_empty() {
        eprintln!("error: {display_path}: selected scope has no tasks");
        return 1;
    }

    let mut records: Vec<Record> = Vec::new();
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

            let branch_prefix_norm = branch_prefix.trim().trim_end_matches('/');
            let branch_prefix_norm = if branch_prefix_norm.is_empty() {
                "issue"
            } else {
                branch_prefix_norm
            };

            let worktree_prefix_norm = worktree_prefix.trim().trim_end_matches(['-', '_']);
            let worktree_prefix_norm = if worktree_prefix_norm.is_empty() {
                "issue"
            } else {
                worktree_prefix_norm
            };

            let owner_prefix_trim = owner_prefix.trim();
            let owner_prefix_norm = if owner_prefix_trim.is_empty() {
                String::from("subagent")
            } else if owner_prefix_trim.to_ascii_lowercase().contains("subagent") {
                owner_prefix_trim.to_string()
            } else {
                format!("subagent-{owner_prefix_trim}")
            };

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

            records.push(Record {
                task_id,
                plan_task_id,
                sprint: sprint.number,
                summary,
                branch: format!("{branch_prefix_norm}/s{}-t{ordinal}-{slug}", sprint.number),
                worktree: format!("{worktree_prefix_norm}-s{}-t{ordinal}", sprint.number),
                owner: format!("{owner_prefix_norm}-s{}-t{ordinal}", sprint.number),
                notes_parts,
                pr_group: String::new(),
            });
        }
    }

    if records.is_empty() {
        eprintln!("error: {display_path}: selected scope has no tasks");
        return 1;
    }

    let mut group_assignments: HashMap<String, String> = HashMap::new();
    let mut assignment_sources: Vec<String> = Vec::new();
    for entry in &pr_group_entries {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((raw_key, raw_group)) = trimmed.split_once('=') else {
            eprintln!("error: --pr-group must use <task-or-plan-id>=<group> format");
            return 1;
        };
        let key = raw_key.trim();
        let group = normalize_token(raw_group.trim(), "", 48);
        if key.is_empty() || group.is_empty() {
            eprintln!("error: --pr-group must include both task key and group");
            return 1;
        }
        assignment_sources.push(key.to_string());
        group_assignments.insert(key.to_ascii_lowercase(), group);
    }

    if pr_grouping == "group" {
        let mut known: HashMap<String, bool> = HashMap::new();
        for rec in &records {
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
            eprintln!(
                "error: --pr-group references unknown task keys: {}",
                unknown
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return 1;
        }
    }

    if pr_grouping == "group" {
        let mut missing: Vec<String> = Vec::new();
        for rec in &mut records {
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
            eprintln!(
                "error: --pr-grouping group requires explicit mapping for every task; missing: {}",
                missing
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            return 1;
        }
    } else {
        for rec in &mut records {
            rec.pr_group =
                normalize_token(&format!("s{}", rec.sprint), &format!("s{}", rec.sprint), 48);
        }
    }

    // Anchor selection is deterministic because records are emitted in stable sprint/task order.
    let mut group_sizes: HashMap<String, usize> = HashMap::new();
    let mut group_anchor: HashMap<String, String> = HashMap::new();
    for rec in &records {
        let size = group_sizes.entry(rec.pr_group.clone()).or_insert(0);
        *size += 1;
        group_anchor
            .entry(rec.pr_group.clone())
            .or_insert_with(|| rec.task_id.clone());
    }

    let mut out_records: Vec<OutputRecord> = Vec::new();
    for rec in &records {
        let mut notes = rec.notes_parts.clone();
        notes.push(format!("pr-grouping={pr_grouping}"));
        notes.push(format!("pr-group={}", rec.pr_group));
        if group_sizes.get(&rec.pr_group).copied().unwrap_or(0) > 1
            && let Some(anchor) = group_anchor.get(&rec.pr_group)
        {
            notes.push(format!("shared-pr-anchor={anchor}"));
        }
        out_records.push(OutputRecord {
            task_id: rec.task_id.clone(),
            summary: rec.summary.clone(),
            branch: rec.branch.clone(),
            worktree: rec.worktree.clone(),
            owner: rec.owner.clone(),
            notes: notes.join("; "),
            pr_group: rec.pr_group.clone(),
        });
    }

    if format == "tsv" {
        print_tsv(&out_records);
        return 0;
    }

    let output = Output {
        file: path_to_posix(&maybe_relativize(&read_path, &repo_root)),
        scope: scope.clone(),
        sprint: sprint_num,
        pr_grouping,
        strategy,
        records: out_records,
    };
    match serde_json::to_string(&output) {
        Ok(json) => {
            println!("{json}");
            0
        }
        Err(err) => {
            eprintln!("error: failed to encode JSON: {err}");
            1
        }
    }
}

fn print_tsv(records: &[OutputRecord]) {
    println!("# task_id\tsummary\tbranch\tworktree\towner\tnotes\tpr_group");
    for rec in records {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}",
            rec.task_id.replace('\t', " "),
            rec.summary.replace('\t', " "),
            rec.branch.replace('\t', " "),
            rec.worktree.replace('\t', " "),
            rec.owner.replace('\t', " "),
            rec.notes.replace('\t', " "),
            rec.pr_group.replace('\t', " "),
        );
    }
}

fn print_usage() {
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
}

fn die(msg: &str) -> i32 {
    eprintln!("split-prs: {msg}");
    2
}

fn resolve_repo_relative(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    repo_root.join(path)
}

fn maybe_relativize(path: &Path, repo_root: &Path) -> PathBuf {
    let Ok(path_abs) = path.canonicalize() else {
        return path.to_path_buf();
    };
    let Ok(root_abs) = repo_root.canonicalize() else {
        return path_abs;
    };
    match path_abs.strip_prefix(&root_abs) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => path_abs,
    }
}

fn path_to_posix(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
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
    let mut final_token = if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    };
    if final_token.len() > max_len {
        final_token.truncate(max_len);
        final_token = final_token.trim_matches('-').to_string();
    }
    final_token
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

#[cfg(test)]
mod tests {
    use super::{is_placeholder, normalize_token};
    use pretty_assertions::assert_eq;

    #[test]
    fn normalize_token_collapses_non_alnum_and_limits_length() {
        assert_eq!(
            normalize_token("Sprint 2 :: Shared Pair", "fallback", 20),
            "sprint-2-shared-pair"
        );
        assert_eq!(normalize_token("!!!!", "fallback-value", 8), "fallback");
    }

    #[test]
    fn placeholder_rules_cover_common_plan_values() {
        assert!(is_placeholder("none"));
        assert!(is_placeholder("<task ids>"));
        assert!(is_placeholder("Task IDs here"));
        assert!(!is_placeholder("Task 1.1"));
    }
}

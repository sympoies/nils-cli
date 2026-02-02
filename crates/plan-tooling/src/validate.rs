use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

use crate::parse::{parse_plan_with_display, Plan, Task};

const USAGE: &str = r#"Usage:
  validate_plans.sh [--file <path>]...

Purpose:
  Lint plan markdown files under docs/plans/ against Plan Format v1.

Options:
  --file <path>  Validate a specific plan file (may be repeated)
  -h, --help     Show help

Defaults:
  With no --file args, validates tracked `docs/plans/*-plan.md` files.

Exit:
  0: all validated files are compliant
  1: validation errors found
  2: usage error
"#;

fn print_usage() {
    let _ = std::io::stderr().write_all(USAGE.as_bytes());
}

fn die(msg: &str) -> i32 {
    eprintln!("validate_plans: {msg}");
    2
}

pub fn run(args: &[String]) -> i32 {
    let mut files: Vec<String> = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                if args.get(i + 1).is_none() {
                    return die("--file requires a path");
                }
                files.push(args[i + 1].to_string());
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

    let repo_root = crate::repo_root::detect();

    let discovered = if files.is_empty() {
        discover_default_plan_files(&repo_root)
    } else {
        files
    };

    if discovered.is_empty() {
        return 0;
    }

    let progress = Progress::new(
        discovered.len() as u64,
        ProgressOptions::default().with_finish(ProgressFinish::Clear),
    );

    let mut errors: Vec<String> = Vec::new();
    for (idx, display_path) in discovered.into_iter().enumerate() {
        progress.set_message(display_path.clone());

        let read_path = resolve_repo_relative(&repo_root, Path::new(&display_path));
        if !read_path.is_file() {
            errors.push(format!("{display_path}: file not found"));
            progress.set_position((idx + 1) as u64);
            continue;
        }
        errors.extend(validate_plan(&display_path, &read_path));

        progress.set_position((idx + 1) as u64);
    }

    progress.finish_and_clear();

    if errors.is_empty() {
        return 0;
    }

    for err in errors {
        eprintln!("error: {err}");
    }
    1
}

fn discover_default_plan_files(repo_root: &Path) -> Vec<String> {
    let mut files = git_ls_files(repo_root, "docs/plans/*-plan.md");
    if files.is_empty() {
        files = find_plan_files(repo_root);
    }
    files
}

fn git_ls_files(repo_root: &Path, pattern: &str) -> Vec<String> {
    let output = Command::new("git")
        .args(["ls-files", "--", pattern])
        .current_dir(repo_root)
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut files: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    files.sort();
    files
}

fn find_plan_files(repo_root: &Path) -> Vec<String> {
    let dir = repo_root.join("docs/plans");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with("-plan.md") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(repo_root) {
            out.push(
                rel.to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/"),
            );
        } else {
            out.push(path.to_string_lossy().to_string());
        }
    }
    out.sort();
    out
}

fn resolve_repo_relative(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    repo_root.join(path)
}

fn validate_plan(display_path: &str, read_path: &Path) -> Vec<String> {
    let plan: Plan;
    let parse_errors: Vec<String>;
    match parse_plan_with_display(read_path, display_path) {
        Ok((p, errs)) => {
            plan = p;
            parse_errors = errs;
        }
        Err(err) => {
            return vec![format!("{display_path}: failed to parse plan: {err}")];
        }
    }

    if !parse_errors.is_empty() {
        return parse_errors
            .into_iter()
            .map(|e| format!("{display_path}: error: {e}"))
            .collect();
    }

    if plan.sprints.is_empty() {
        return vec![format!(
            "{display_path}: missing sprints (expected '## Sprint N: ...' headings)"
        )];
    }

    let mut tasks: Vec<&Task> = Vec::new();
    for sprint in &plan.sprints {
        tasks.extend(sprint.tasks.iter());
    }
    if tasks.is_empty() {
        return vec![format!(
            "{display_path}: no tasks found (expected '### Task N.M: ...' headings)"
        )];
    }

    let all_task_ids: HashSet<String> = tasks.iter().map(|t| t.id.trim().to_string()).collect();

    let mut errs: Vec<String> = Vec::new();
    for task in tasks {
        errs.extend(validate_task(display_path, task, &all_task_ids));
    }
    errs
}

fn validate_task(plan_path: &str, task: &Task, all_task_ids: &HashSet<String>) -> Vec<String> {
    let mut errs: Vec<String> = Vec::new();

    let task_id = task.id.trim();
    let prefix = if task_id.is_empty() {
        format!("{plan_path}:<unknown task>")
    } else {
        format!("{plan_path}:{task_id}")
    };

    if task_id.is_empty() || !is_task_id(task_id) {
        errs.push(format!("{prefix}: invalid or missing task id"));
    }

    if !is_non_empty_list(&task.location) {
        errs.push(format!(
            "{prefix}: missing Location (must be a non-empty list)"
        ));
    } else {
        for loc in &task.location {
            if loc.trim().is_empty() {
                continue;
            }
            if loc.starts_with('/') {
                errs.push(format!(
                    "{prefix}: Location must be repo-relative (no leading '/'): {}",
                    crate::repr::py_repr(loc)
                ));
            }
            if loc.ends_with('/') {
                errs.push(format!(
                    "{prefix}: Location must be a file path (not a directory): {}",
                    crate::repr::py_repr(loc)
                ));
            }
            if ["*", "?", "{", "}"].iter().any(|ch| loc.contains(ch)) {
                errs.push(format!(
                    "{prefix}: Location must not use globs/braces: {}",
                    crate::repr::py_repr(loc)
                ));
            }
            if has_placeholder(loc) {
                errs.push(format!(
                    "{prefix}: Location contains placeholder: {}",
                    crate::repr::py_repr(loc)
                ));
            }
        }
    }

    match task.description.as_deref() {
        None => errs.push(format!("{prefix}: missing Description")),
        Some(desc) => {
            if desc.trim().is_empty() {
                errs.push(format!("{prefix}: missing Description"));
            } else if has_placeholder(desc) {
                errs.push(format!(
                    "{prefix}: Description contains placeholder: {}",
                    crate::repr::py_repr(desc)
                ));
            }
        }
    }

    match task.dependencies.as_ref() {
        None => errs.push(format!(
            "{prefix}: missing Dependencies (use 'none' or list task IDs)"
        )),
        Some(deps) => {
            for dep in deps {
                let d = dep.trim();
                if d.is_empty() {
                    continue;
                }
                if !is_task_id(d) {
                    errs.push(format!(
                        "{prefix}: invalid dependency (expected 'Task N.M'): {}",
                        crate::repr::py_repr(dep)
                    ));
                } else if !all_task_ids.contains(d) {
                    errs.push(format!(
                        "{prefix}: unknown dependency (not found in plan): {}",
                        crate::repr::py_repr(d)
                    ));
                }
            }
        }
    }

    if let Some(c) = task.complexity {
        if !(1..=10).contains(&c) {
            errs.push(format!("{prefix}: Complexity out of range (1-10): {c}"));
        }
    }

    if !is_non_empty_list(&task.acceptance_criteria) {
        errs.push(format!(
            "{prefix}: missing Acceptance criteria (must be a non-empty list)"
        ));
    } else {
        for item in &task.acceptance_criteria {
            if has_placeholder(item) {
                errs.push(format!(
                    "{prefix}: Acceptance criteria contains placeholder: {}",
                    crate::repr::py_repr(item)
                ));
            }
        }
    }

    if !is_non_empty_list(&task.validation) {
        errs.push(format!(
            "{prefix}: missing Validation (must be a non-empty list)"
        ));
    } else {
        for cmd in &task.validation {
            if has_placeholder(cmd) {
                errs.push(format!(
                    "{prefix}: Validation contains placeholder: {}",
                    crate::repr::py_repr(cmd)
                ));
            }
        }
    }

    errs
}

fn has_placeholder(value: &str) -> bool {
    if contains_angle_placeholder(value) {
        return true;
    }

    contains_word_case_insensitive(value, "TBD") || contains_word_case_insensitive(value, "TODO")
}

fn contains_angle_placeholder(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            let start = i + 1;
            if start < bytes.len() {
                if let Some(end) = bytes[start..].iter().position(|b| *b == b'>') {
                    if end >= 1 {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

fn contains_word_case_insensitive(haystack: &str, needle: &str) -> bool {
    let h = haystack.to_ascii_uppercase();
    let n = needle.to_ascii_uppercase();
    let hb = h.as_bytes();
    let nb = n.as_bytes();
    if nb.is_empty() || hb.len() < nb.len() {
        return false;
    }

    for i in 0..=(hb.len() - nb.len()) {
        if &hb[i..i + nb.len()] != nb {
            continue;
        }
        let left_ok = i == 0 || !is_word_byte(hb[i - 1]);
        let right_ok = i + nb.len() == hb.len() || !is_word_byte(hb[i + nb.len()]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
}

fn is_non_empty_list(items: &[String]) -> bool {
    items.iter().any(|x| !x.trim().is_empty())
}

fn is_task_id(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("Task ") else {
        return false;
    };
    let Some((a, b)) = rest.split_once('.') else {
        return false;
    };
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.chars().all(|c| c.is_ascii_digit()) && b.chars().all(|c| c.is_ascii_digit())
}

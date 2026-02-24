use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
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
    complexity: i32,
    location_paths: Vec<String>,
    dependency_keys: Vec<String>,
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

    // Deterministic group mode requires full explicit mappings.
    // Auto group mode can derive missing assignments from topology/conflict signals.
    if pr_grouping == "group" && strategy == "deterministic" && pr_group_entries.is_empty() {
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
            let location_paths: Vec<String> = task
                .location
                .iter()
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .filter(|p| !is_placeholder(p))
                .collect();
            let complexity = match task.complexity {
                Some(value) if value > 0 => value,
                _ => 5,
            };

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
                complexity,
                location_paths,
                dependency_keys: deps.clone(),
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

    if pr_grouping == "group" && !assignment_sources.is_empty() {
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
            rec.pr_group.clear();
            for key in [&rec.task_id, &rec.plan_task_id] {
                if key.is_empty() {
                    continue;
                }
                if let Some(v) = group_assignments.get(&key.to_ascii_lowercase()) {
                    rec.pr_group = v.to_string();
                    break;
                }
            }
            if rec.pr_group.is_empty() {
                missing.push(rec.task_id.clone());
            }
        }
        if strategy == "deterministic" {
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
        } else if !missing.is_empty() {
            assign_auto_groups(&mut records);
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

#[derive(Debug)]
struct AutoMergeCandidate {
    i: usize,
    j: usize,
    score_key: i64,
    key_a: String,
    key_b: String,
}

fn assign_auto_groups(records: &mut [Record]) {
    let mut sprint_to_indices: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (idx, rec) in records.iter().enumerate() {
        if rec.pr_group.is_empty() {
            sprint_to_indices.entry(rec.sprint).or_default().push(idx);
        }
    }

    for (sprint, indices) in sprint_to_indices {
        let assignments = auto_groups_for_sprint(records, sprint, &indices);
        for (idx, group) in assignments {
            if let Some(rec) = records.get_mut(idx)
                && rec.pr_group.is_empty()
            {
                rec.pr_group = group;
            }
        }
    }
}

fn auto_groups_for_sprint(
    records: &[Record],
    sprint: i32,
    indices: &[usize],
) -> BTreeMap<usize, String> {
    let mut lookup: HashMap<String, usize> = HashMap::new();
    for idx in indices {
        let rec = &records[*idx];
        lookup.insert(rec.task_id.to_ascii_lowercase(), *idx);
        if !rec.plan_task_id.is_empty() {
            lookup.insert(rec.plan_task_id.to_ascii_lowercase(), *idx);
        }
    }

    let mut deps: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    let mut paths: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for idx in indices {
        let rec = &records[*idx];
        let mut resolved_deps: BTreeSet<usize> = BTreeSet::new();
        for dep in &rec.dependency_keys {
            let dep_key = dep.trim().to_ascii_lowercase();
            if dep_key.is_empty() {
                continue;
            }
            if let Some(dep_idx) = lookup.get(&dep_key)
                && dep_idx != idx
            {
                resolved_deps.insert(*dep_idx);
            }
        }
        deps.insert(*idx, resolved_deps);

        let normalized_paths: BTreeSet<String> = rec
            .location_paths
            .iter()
            .map(|path| normalize_location_path(path))
            .filter(|path| !path.is_empty())
            .collect();
        paths.insert(*idx, normalized_paths);
    }

    let batch_by_idx = compute_batch_index(records, indices, &deps);
    let mut parent: HashMap<usize, usize> = indices.iter().copied().map(|idx| (idx, idx)).collect();

    let mut by_batch: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for idx in indices {
        let batch = batch_by_idx.get(idx).copied().unwrap_or(0);
        by_batch.entry(batch).or_default().push(*idx);
    }

    for members in by_batch.values_mut() {
        members.sort_by_key(|idx| task_sort_key(records, *idx));

        let mut path_to_members: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for idx in members {
            for path in paths.get(idx).into_iter().flatten() {
                path_to_members.entry(path.clone()).or_default().push(*idx);
            }
        }
        for overlap_members in path_to_members.values() {
            if overlap_members.len() < 2 {
                continue;
            }
            let first = overlap_members[0];
            for other in overlap_members.iter().skip(1) {
                uf_union(&mut parent, first, *other);
            }
        }
    }

    let mut grouped: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for idx in indices {
        let root = uf_find(&mut parent, *idx);
        grouped.entry(root).or_default().insert(*idx);
    }
    let mut groups: Vec<BTreeSet<usize>> = grouped.into_values().collect();

    loop {
        let mut candidates: Vec<AutoMergeCandidate> = Vec::new();
        for i in 0..groups.len() {
            for j in (i + 1)..groups.len() {
                let merged_complexity =
                    group_complexity(records, &groups[i]) + group_complexity(records, &groups[j]);
                if merged_complexity > 20 {
                    continue;
                }

                let dep_cross = dependency_cross_edges(&deps, &groups[i], &groups[j]);
                let overlap_paths = overlap_path_count(&paths, &groups[i], &groups[j]);
                let min_group_size = groups[i].len().min(groups[j].len()).max(1) as f64;
                let dep_affinity = ((dep_cross as f64) / min_group_size).min(1.0);
                let ovl_affinity = ((overlap_paths as f64) / 2.0).min(1.0);
                let size_fit = (1.0 - ((merged_complexity as f64 - 12.0).abs() / 12.0)).max(0.0);
                let span = group_span(&batch_by_idx, &groups[i], &groups[j]);
                let serial_penalty = ((span as f64 - 1.0).max(0.0)) / 3.0;
                let oversize_penalty = ((merged_complexity as f64 - 20.0).max(0.0)) / 20.0;

                let score = (0.45 * dep_affinity) + (0.35 * ovl_affinity) + (0.20 * size_fit)
                    - (0.25 * serial_penalty)
                    - (0.45 * oversize_penalty);
                if score < 0.30 {
                    continue;
                }

                let mut key_a = group_min_task_key(records, &groups[i]);
                let mut key_b = group_min_task_key(records, &groups[j]);
                if key_b < key_a {
                    std::mem::swap(&mut key_a, &mut key_b);
                }
                candidates.push(AutoMergeCandidate {
                    i,
                    j,
                    score_key: (score * 1_000_000.0).round() as i64,
                    key_a,
                    key_b,
                });
            }
        }

        if candidates.is_empty() {
            break;
        }

        candidates.sort_by(|a, b| {
            b.score_key
                .cmp(&a.score_key)
                .then_with(|| a.key_a.cmp(&b.key_a))
                .then_with(|| a.key_b.cmp(&b.key_b))
                .then_with(|| a.i.cmp(&b.i))
                .then_with(|| a.j.cmp(&b.j))
        });
        let chosen = &candidates[0];

        let mut merged = groups[chosen.i].clone();
        merged.extend(groups[chosen.j].iter().copied());
        groups[chosen.i] = merged;
        groups.remove(chosen.j);
    }

    groups.sort_by(|a, b| {
        group_min_batch(&batch_by_idx, a)
            .cmp(&group_min_batch(&batch_by_idx, b))
            .then_with(|| group_min_task_key(records, a).cmp(&group_min_task_key(records, b)))
    });

    let mut out: BTreeMap<usize, String> = BTreeMap::new();
    for (idx, group) in groups.iter().enumerate() {
        let fallback = format!("s{sprint}-auto-g{}", idx + 1);
        let group_key = normalize_token(&fallback, &fallback, 48);
        for member in group {
            out.insert(*member, group_key.clone());
        }
    }
    out
}

fn compute_batch_index(
    records: &[Record],
    indices: &[usize],
    deps: &BTreeMap<usize, BTreeSet<usize>>,
) -> BTreeMap<usize, usize> {
    let mut in_deg: HashMap<usize, usize> = indices.iter().copied().map(|idx| (idx, 0)).collect();
    let mut reverse: HashMap<usize, BTreeSet<usize>> = indices
        .iter()
        .copied()
        .map(|idx| (idx, BTreeSet::new()))
        .collect();

    for idx in indices {
        for dep in deps.get(idx).cloned().unwrap_or_default() {
            if !in_deg.contains_key(&dep) {
                continue;
            }
            if let Some(value) = in_deg.get_mut(idx) {
                *value += 1;
            }
            if let Some(children) = reverse.get_mut(&dep) {
                children.insert(*idx);
            }
        }
    }

    let mut remaining: BTreeSet<usize> = indices.iter().copied().collect();
    let mut batch_by_idx: BTreeMap<usize, usize> = BTreeMap::new();
    let mut layer = 0usize;
    let mut ready: VecDeque<usize> = {
        let mut start: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|idx| in_deg.get(idx).copied().unwrap_or(0) == 0)
            .collect();
        start.sort_by_key(|idx| task_sort_key(records, *idx));
        start.into_iter().collect()
    };

    while !remaining.is_empty() {
        let mut batch_members: Vec<usize> = ready.drain(..).collect();
        batch_members.sort_by_key(|idx| task_sort_key(records, *idx));

        if batch_members.is_empty() {
            let mut cycle_members: Vec<usize> = remaining.iter().copied().collect();
            cycle_members.sort_by_key(|idx| task_sort_key(records, *idx));
            for idx in cycle_members {
                remaining.remove(&idx);
                batch_by_idx.insert(idx, layer);
            }
            break;
        }

        for idx in &batch_members {
            remaining.remove(idx);
            batch_by_idx.insert(*idx, layer);
        }

        let mut next: Vec<usize> = Vec::new();
        for idx in batch_members {
            for child in reverse.get(&idx).cloned().unwrap_or_default() {
                if let Some(value) = in_deg.get_mut(&child) {
                    *value = value.saturating_sub(1);
                    if *value == 0 && remaining.contains(&child) {
                        next.push(child);
                    }
                }
            }
        }
        next.sort_by_key(|idx| task_sort_key(records, *idx));
        next.dedup();
        ready.extend(next);
        layer += 1;
    }

    for idx in indices {
        batch_by_idx.entry(*idx).or_insert(0);
    }
    batch_by_idx
}

fn task_sort_key(records: &[Record], idx: usize) -> (String, String) {
    let rec = &records[idx];
    let primary = if rec.plan_task_id.trim().is_empty() {
        rec.task_id.to_ascii_lowercase()
    } else {
        rec.plan_task_id.to_ascii_lowercase()
    };
    (primary, rec.task_id.to_ascii_lowercase())
}

fn normalize_location_path(path: &str) -> String {
    path.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn group_complexity(records: &[Record], group: &BTreeSet<usize>) -> i32 {
    group
        .iter()
        .map(|idx| records[*idx].complexity.max(1))
        .sum::<i32>()
}

fn group_min_task_key(records: &[Record], group: &BTreeSet<usize>) -> String {
    group
        .iter()
        .map(|idx| task_sort_key(records, *idx).0)
        .min()
        .unwrap_or_default()
}

fn group_min_batch(batch_by_idx: &BTreeMap<usize, usize>, group: &BTreeSet<usize>) -> usize {
    group
        .iter()
        .filter_map(|idx| batch_by_idx.get(idx).copied())
        .min()
        .unwrap_or(0)
}

fn group_span(
    batch_by_idx: &BTreeMap<usize, usize>,
    left: &BTreeSet<usize>,
    right: &BTreeSet<usize>,
) -> usize {
    let mut min_batch = usize::MAX;
    let mut max_batch = 0usize;
    for idx in left.union(right) {
        let batch = batch_by_idx.get(idx).copied().unwrap_or(0);
        min_batch = min_batch.min(batch);
        max_batch = max_batch.max(batch);
    }
    if min_batch == usize::MAX {
        0
    } else {
        max_batch.saturating_sub(min_batch)
    }
}

fn dependency_cross_edges(
    deps: &BTreeMap<usize, BTreeSet<usize>>,
    left: &BTreeSet<usize>,
    right: &BTreeSet<usize>,
) -> usize {
    let mut count = 0usize;
    for src in left {
        if let Some(edges) = deps.get(src) {
            count += edges.iter().filter(|dep| right.contains(dep)).count();
        }
    }
    for src in right {
        if let Some(edges) = deps.get(src) {
            count += edges.iter().filter(|dep| left.contains(dep)).count();
        }
    }
    count
}

fn overlap_path_count(
    paths: &BTreeMap<usize, BTreeSet<String>>,
    left: &BTreeSet<usize>,
    right: &BTreeSet<usize>,
) -> usize {
    let mut left_paths: BTreeSet<String> = BTreeSet::new();
    let mut right_paths: BTreeSet<String> = BTreeSet::new();
    for idx in left {
        for path in paths.get(idx).into_iter().flatten() {
            left_paths.insert(path.clone());
        }
    }
    for idx in right {
        for path in paths.get(idx).into_iter().flatten() {
            right_paths.insert(path.clone());
        }
    }
    left_paths.intersection(&right_paths).count()
}

fn uf_find(parent: &mut HashMap<usize, usize>, node: usize) -> usize {
    let parent_node = parent.get(&node).copied().unwrap_or(node);
    if parent_node == node {
        return node;
    }
    let root = uf_find(parent, parent_node);
    parent.insert(node, root);
    root
}

fn uf_union(parent: &mut HashMap<usize, usize>, left: usize, right: usize) {
    let left_root = uf_find(parent, left);
    let right_root = uf_find(parent, right);
    if left_root == right_root {
        return;
    }
    if left_root < right_root {
        parent.insert(right_root, left_root);
    } else {
        parent.insert(left_root, right_root);
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

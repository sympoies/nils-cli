use serde::Serialize;
use std::path::Path;

pub mod to_json;

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub title: String,
    pub file: String,
    pub sprints: Vec<Sprint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sprint {
    pub number: i32,
    pub name: String,
    pub start_line: u32,
    pub tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    pub name: String,
    pub sprint: i32,
    pub start_line: u32,
    pub location: Vec<String>,
    pub description: Option<String>,
    pub dependencies: Option<Vec<String>>,
    pub complexity: Option<i32>,
    pub acceptance_criteria: Vec<String>,
    pub validation: Vec<String>,
}

pub fn parse_plan_with_display(
    path: &Path,
    display_path: &str,
) -> anyhow::Result<(Plan, Vec<String>)> {
    let raw = std::fs::read(path)?;
    let raw_text = String::from_utf8_lossy(&raw);
    let raw_lines: Vec<String> = raw_text.lines().map(|l| l.to_string()).collect();

    let mut plan_title = String::new();
    for line in &raw_lines {
        if let Some(rest) = line.strip_prefix("# ") {
            plan_title = rest.trim().to_string();
            break;
        }
    }

    let mut errors: Vec<String> = Vec::new();

    let mut sprints: Vec<Sprint> = Vec::new();
    let mut current_sprint: Option<Sprint> = None;
    let mut current_task: Option<Task> = None;

    fn finish_task(
        current_task: &mut Option<Task>,
        current_sprint: &mut Option<Sprint>,
        errors: &mut Vec<String>,
        display_path: &str,
    ) {
        let Some(task) = current_task.take() else {
            return;
        };
        let Some(sprint) = current_sprint.as_mut() else {
            errors.push(format!(
                "{display_path}:{}: task outside of any sprint: {}",
                task.start_line, task.id
            ));
            return;
        };
        sprint.tasks.push(task);
    }

    fn finish_sprint(current_sprint: &mut Option<Sprint>, sprints: &mut Vec<Sprint>) {
        if let Some(s) = current_sprint.take() {
            sprints.push(s);
        }
    }

    let mut i: usize = 0;
    while i < raw_lines.len() {
        let line = raw_lines[i].as_str();

        if let Some((number, name)) = parse_sprint_heading(line) {
            finish_task(
                &mut current_task,
                &mut current_sprint,
                &mut errors,
                display_path,
            );
            finish_sprint(&mut current_sprint, &mut sprints);
            current_sprint = Some(Sprint {
                number,
                name,
                start_line: (i + 1) as u32,
                tasks: Vec::new(),
            });
            i += 1;
            continue;
        }

        if let Some((sprint_num, seq_num, name)) = parse_task_heading(line) {
            finish_task(
                &mut current_task,
                &mut current_sprint,
                &mut errors,
                display_path,
            );
            current_task = Some(Task {
                id: normalize_task_id(sprint_num, seq_num),
                name,
                sprint: sprint_num,
                start_line: (i + 1) as u32,
                location: Vec::new(),
                description: None,
                dependencies: None,
                complexity: None,
                acceptance_criteria: Vec::new(),
                validation: Vec::new(),
            });
            i += 1;
            continue;
        }

        if current_task.is_none() {
            i += 1;
            continue;
        }

        let Some((base_indent, field, rest)) = parse_field_line(line) else {
            i += 1;
            continue;
        };

        match field.as_str() {
            "Description" => {
                let v = rest.unwrap_or_default();
                if let Some(task) = current_task.as_mut() {
                    task.description = Some(v);
                }
                i += 1;
            }
            "Complexity" => {
                let v = rest.unwrap_or_default();
                if !v.trim().is_empty() {
                    match v.trim().parse::<i32>() {
                        Ok(n) => {
                            if let Some(task) = current_task.as_mut() {
                                task.complexity = Some(n);
                            }
                        }
                        Err(_) => {
                            errors.push(format!(
                                "{display_path}:{}: invalid Complexity (expected int): {}",
                                i + 1,
                                crate::repr::py_repr(v.trim())
                            ));
                        }
                    }
                }
                i += 1;
            }
            "Location" | "Dependencies" | "Acceptance criteria" | "Validation" => {
                let (items, next_idx) = if let Some(r) = rest.clone() {
                    if !r.trim().is_empty() {
                        (vec![strip_inline_code(&r)], i + 1)
                    } else {
                        parse_list_block(&raw_lines, i + 1, base_indent)
                    }
                } else {
                    parse_list_block(&raw_lines, i + 1, base_indent)
                };

                if let Some(task) = current_task.as_mut() {
                    let cleaned: Vec<String> =
                        items.into_iter().filter(|x| !x.trim().is_empty()).collect();
                    match field.as_str() {
                        "Location" => task.location.extend(cleaned),
                        "Dependencies" => task.dependencies = Some(cleaned),
                        "Acceptance criteria" => task.acceptance_criteria.extend(cleaned),
                        "Validation" => task.validation.extend(cleaned),
                        _ => {}
                    }
                }

                i = next_idx;
            }
            _ => {
                i += 1;
            }
        }
    }

    finish_task(
        &mut current_task,
        &mut current_sprint,
        &mut errors,
        display_path,
    );
    finish_sprint(&mut current_sprint, &mut sprints);

    for sprint in &mut sprints {
        for task in &mut sprint.tasks {
            let Some(deps) = task.dependencies.clone() else {
                continue;
            };

            let mut normalized: Vec<String> = Vec::new();
            for d in deps {
                let trimmed = d.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if trimmed.eq_ignore_ascii_case("none") {
                    continue;
                }
                for part in trimmed.split(',') {
                    let p = part.trim();
                    if !p.is_empty() {
                        normalized.push(p.to_string());
                    }
                }
            }
            task.dependencies = Some(normalized);
        }
    }

    Ok((
        Plan {
            title: plan_title,
            file: display_path.to_string(),
            sprints,
        },
        errors,
    ))
}

fn normalize_task_id(sprint: i32, seq: i32) -> String {
    format!("Task {sprint}.{seq}")
}

fn parse_sprint_heading(line: &str) -> Option<(i32, String)> {
    let rest = line.strip_prefix("## Sprint ")?;
    let (num_part, name_part) = rest.split_once(':')?;
    if num_part.is_empty() || !num_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let number = num_part.parse::<i32>().ok()?;
    let name = name_part.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((number, name))
}

fn parse_task_heading(line: &str) -> Option<(i32, i32, String)> {
    let rest = line.strip_prefix("### Task ")?;
    let (id_part, name_part) = rest.split_once(':')?;
    let (sprint_part, seq_part) = id_part.split_once('.')?;
    if sprint_part.is_empty() || !sprint_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if seq_part.is_empty() || !seq_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let sprint_num = sprint_part.parse::<i32>().ok()?;
    let seq_num = seq_part.parse::<i32>().ok()?;
    let name = name_part.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some((sprint_num, seq_num, name))
}

fn parse_field_line(line: &str) -> Option<(usize, String, Option<String>)> {
    let base_indent = line.chars().take_while(|c| *c == ' ').count();
    let trimmed = line.trim_start_matches(' ');
    let after_dash = trimmed.strip_prefix('-')?;
    let after_space = after_dash.trim_start();
    let after_star = after_space.strip_prefix("**")?;
    let (field, rest) = after_star.split_once("**:")?;
    let field = field.to_string();
    match field.as_str() {
        "Location"
        | "Description"
        | "Dependencies"
        | "Complexity"
        | "Acceptance criteria"
        | "Validation" => Some((base_indent, field, Some(rest.trim().to_string()))),
        _ => None,
    }
}

fn strip_inline_code(text: &str) -> String {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('`') && t.ends_with('`') {
        return t[1..t.len() - 1].trim().to_string();
    }
    t.to_string()
}

fn parse_list_block(
    lines: &[String],
    start_idx: usize,
    base_indent: usize,
) -> (Vec<String>, usize) {
    let mut items: Vec<String> = Vec::new();
    let mut i = start_idx;
    while i < lines.len() {
        let raw = lines[i].as_str();
        if raw.trim().is_empty() {
            i += 1;
            continue;
        }

        let indent = raw.chars().take_while(|c| *c == ' ').count();
        let trimmed = raw.trim_start_matches(' ');
        if !trimmed.starts_with('-') {
            break;
        }
        let after_dash = &trimmed[1..];
        if after_dash.is_empty() || !after_dash.chars().next().unwrap_or('x').is_whitespace() {
            break;
        }
        if indent <= base_indent {
            break;
        }
        let text = after_dash.trim_start().trim_end();
        items.push(strip_inline_code(text));
        i += 1;
    }

    (items, i)
}

use crate::{fzf, util};
use std::path::PathBuf;

pub fn run(args: &[String]) -> i32 {
    let default_query = if args.is_empty() {
        std::env::var("BUFFER").unwrap_or_default()
    } else {
        util::join_args(args)
    };

    let histfile = match std::env::var("HISTFILE") {
        Ok(v) if !v.trim().is_empty() => PathBuf::from(v),
        _ => {
            eprintln!("❌ HISTFILE is not set.");
            return 1;
        }
    };

    let data = match std::fs::read(&histfile) {
        Ok(v) => v,
        Err(_) => return 0,
    };

    let content = String::from_utf8_lossy(&data);
    let mut entries = Vec::new();
    for (idx, raw_line) in content.lines().enumerate() {
        let Some(entry) = parse_history_line(raw_line, idx + 1) else {
            continue;
        };
        entries.push(entry);
    }
    entries.reverse();

    let input = entries
        .into_iter()
        .map(|e| format!("{} | {:>4} | {}", e.epoch, e.line_no, e.cmd))
        .collect::<Vec<_>>()
        .join("\n");

    let args_vec: Vec<String> = vec![
        "--ansi".to_string(),
        "--reverse".to_string(),
        "--height=50%".to_string(),
        "--query".to_string(),
        default_query,
        "--preview-window=right:50%:wrap".to_string(),
        "--preview".to_string(),
        "printf \"%s\\n\" {}".to_string(),
        "--expect".to_string(),
        "enter".to_string(),
    ];

    let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
    let (code, _key, rest) = match fzf::run_expect(&format!("{input}\n"), &args_ref, &[]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    if code != 0 {
        return 0;
    }

    let Some(selected) = rest.first() else {
        return 0;
    };

    let cmd = extract_cmd_from_selected_line(selected);
    let cmd = strip_icon_prefix(cmd.trim());
    if cmd.is_empty() {
        return 0;
    }

    println!("{cmd}");
    0
}

struct HistoryEntry {
    epoch: String,
    line_no: usize,
    cmd: String,
}

fn parse_history_line(line: &str, line_no: usize) -> Option<HistoryEntry> {
    if !line.starts_with(':') {
        return None;
    }

    let (meta, cmd) = line.split_once(';')?;
    let cmd = cmd.to_string();

    if cmd.trim().is_empty() {
        return None;
    }
    if cmd
        .chars()
        .all(|c| c.is_ascii_control() || c.is_ascii_punctuation() || c.is_whitespace())
    {
        return None;
    }
    if cmd.chars().any(|c| c.is_control()) {
        return None;
    }
    if cmd.contains('\u{FFFD}') {
        return None;
    }

    let epoch = meta
        .split(':')
        .nth(1)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    Some(HistoryEntry {
        epoch,
        line_no,
        cmd,
    })
}

fn extract_cmd_from_selected_line(selected: &str) -> &str {
    let Some(first) = selected.find('|') else {
        return selected;
    };
    let Some(second_rel) = selected[first + 1..].find('|') else {
        return selected;
    };
    let second = first + 1 + second_rel;
    selected[second + 1..].trim()
}

fn strip_icon_prefix(input: &str) -> &str {
    let trimmed = input.trim_start();
    for icon in ["🖥️", "🧪", "🐧", "🐳", "🛠️"] {
        if let Some(rest) = trimmed.strip_prefix(icon) {
            return rest.trim_start();
        }
    }
    trimmed
}

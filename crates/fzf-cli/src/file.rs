use crate::{fzf, open, util};
use std::path::Path;
use walkdir::WalkDir;

pub fn run(args: &[String]) -> i32 {
    let (open_with, query_parts) = match open::parse_open_with_flags(args) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let max_depth = util::env_or_default("FZF_FILE_MAX_DEPTH", "10")
        .parse::<usize>()
        .unwrap_or(10);

    let files = list_files(max_depth);
    let input = if files.is_empty() {
        String::new()
    } else {
        format!("{}\n", files.join("\n"))
    };

    let query = util::join_args(&query_parts);
    let fzf_args = [
        "--ansi",
        "--query",
        &query,
        "--preview",
        "bat --color=always --style=numbers --line-range :100 {}",
    ];

    let (code, lines) = match fzf::run_lines(&input, &fzf_args, &[]) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    if code != 0 {
        return 0;
    }
    let Some(selected) = lines.first() else {
        return 0;
    };

    open::open_file(open_with, Path::new(selected), false)
}

fn list_files(max_depth: usize) -> Vec<String> {
    let walker = WalkDir::new(".")
        .follow_links(true)
        .max_depth(max_depth.saturating_add(1))
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");

    let mut out = Vec::new();
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        let display = path.strip_prefix(".").unwrap_or(path).to_string_lossy();
        let display = display.trim_start_matches('/');
        if display.is_empty() {
            continue;
        }
        out.push(display.to_string());
    }
    out.sort();
    out
}

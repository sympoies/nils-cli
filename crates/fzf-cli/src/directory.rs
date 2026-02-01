use crate::{fzf, open, util};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub fn run(args: &[String]) -> i32 {
    let (open_with, query_parts) = match open::parse_open_with_flags(args) {
        Ok(v) => v,
        Err(code) => return code,
    };

    let mut dir_query = util::join_args(&query_parts);
    let max_depth = util::env_or_default("FZF_FILE_MAX_DEPTH", "10")
        .parse::<usize>()
        .unwrap_or(10);

    loop {
        let dirs = list_dirs();
        let input = if dirs.is_empty() {
            String::new()
        } else {
            format!("{}\n", dirs.join("\n"))
        };

        let args_vec: Vec<String> = vec![
            "--ansi".to_string(),
            "--prompt".to_string(),
            "📁 Directory > ".to_string(),
            "--preview".to_string(),
            "command -v eza >/dev/null && eza -alhT --level=2 --color=always {} || ls -la {}"
                .to_string(),
            "--print-query".to_string(),
            "--query".to_string(),
            dir_query.clone(),
        ];

        let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
        let (code, query, selected) = match fzf::run_print_query(&input, &args_ref, &[]) {
            Ok(v) => v,
            Err(err) => {
                eprintln!("{err:#}");
                return 1;
            }
        };

        if code != 0 {
            return 1;
        }

        dir_query = query.unwrap_or_default();
        let Some(dir) = selected.filter(|s| !s.is_empty()) else {
            return 1;
        };

        let dir_path = PathBuf::from(&dir);
        let dir_path = canonicalize_or_fallback(&dir_path);

        loop {
            let files = list_files_in_dir(&dir_path, max_depth);
            let file_input = if files.is_empty() {
                String::new()
            } else {
                format!("{}\n", files.join("\n"))
            };

            let args_vec: Vec<String> = vec![
                "--ansi".to_string(),
                "--prompt".to_string(),
                format!(
                    "📄 Files in {} > ",
                    dir_path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("dir")
                ),
                "--header".to_string(),
                "enter/ctrl-f: open (exit)    ctrl-d: cd (exit)    esc: back".to_string(),
                "--preview-window".to_string(),
                util::env_or_default("FZF_PREVIEW_WINDOW", "right:50%:wrap"),
                "--preview".to_string(),
                "if command -v bat >/dev/null; then bat --color=always --style=numbers --line-range :200 -- \"$FZF_DIRECTORY_ROOT\"/{}; else sed -n \"1,200p\" \"$FZF_DIRECTORY_ROOT\"/{}; fi"
                    .to_string(),
                "--expect".to_string(),
                "enter,ctrl-f,ctrl-d".to_string(),
            ];

            let args_ref: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();
            let env_root = dir_path.to_string_lossy();
            let (code, key, rest) = match fzf::run_expect(
                &file_input,
                &args_ref,
                &[("FZF_DIRECTORY_ROOT", env_root.as_ref())],
            ) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{err:#}");
                    return 1;
                }
            };

            if code != 0 {
                break;
            }

            let key = key.unwrap_or_default();
            let selected_file = rest.first().cloned().unwrap_or_default();

            match key.as_str() {
                "ctrl-d" => {
                    let escaped = util::shell_escape_single_quotes(env_root.as_ref());
                    println!("cd {escaped}");
                    return 0;
                }
                "enter" | "ctrl-f" => {
                    if selected_file.is_empty() {
                        continue;
                    }
                    let full_path = dir_path.join(&selected_file);
                    let _ = open::open_file(open_with, Path::new(&full_path), false);
                    return 0;
                }
                _ => {}
            }
        }
    }
}

fn list_dirs() -> Vec<String> {
    let spinner = Progress::spinner(
        ProgressOptions::default()
            .with_prefix("index ")
            .with_finish(ProgressFinish::Clear),
    );
    spinner.set_message("directories");
    spinner.tick();

    let mut out: Vec<String> = Vec::new();

    let walker = WalkDir::new(".")
        .follow_links(true)
        .max_depth(25)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");

    let mut scanned: usize = 0;
    for entry in walker.flatten() {
        scanned = scanned.saturating_add(1);
        if scanned % 128 == 0 {
            spinner.tick();
        }

        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        if path == Path::new(".") {
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
    spinner.finish_and_clear();
    out
}

fn list_files_in_dir(dir: &Path, max_depth: usize) -> Vec<String> {
    let spinner = Progress::spinner(
        ProgressOptions::default()
            .with_prefix("index ")
            .with_finish(ProgressFinish::Clear),
    );
    spinner.set_message("files");
    spinner.tick();

    let mut out: Vec<String> = Vec::new();
    let walker = WalkDir::new(dir)
        .follow_links(true)
        .max_depth(max_depth.saturating_add(1))
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git");

    let mut scanned: usize = 0;
    for entry in walker.flatten() {
        scanned = scanned.saturating_add(1);
        if scanned % 128 == 0 {
            spinner.tick();
        }

        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.components().any(|c| c.as_os_str() == ".git") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(dir) {
            let display = rel.to_string_lossy();
            if !display.is_empty() {
                out.push(display.to_string());
            }
        }
    }

    out.sort();
    spinner.finish_and_clear();
    out
}

fn canonicalize_or_fallback(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

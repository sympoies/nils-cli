use crate::change::parse_name_status_lines;
use crate::print::{emit_file, HeadFallback, PrintSource};
use crate::progress::ProgressRunner;
use crate::tree::{tree_support, TREE_MISSING_WARNING, TREE_UNSUPPORTED_WARNING};
use anyhow::Result;
use std::collections::BTreeSet;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum PrintMode {
    Worktree,
    Index,
}

pub fn render_with_type(
    lines: &[String],
    no_color: bool,
    print_mode: PrintMode,
    print: bool,
    progress_opt_in: bool,
) -> Result<Vec<String>> {
    if lines.is_empty() {
        println!("⚠️  No matching files");
        return Ok(Vec::new());
    }

    println!();
    println!("📄 Changed files:");

    let mut files: Vec<String> = Vec::new();

    for entry in parse_name_status_lines(lines) {
        let display_path = entry.display_path();
        let file_path = entry.file_path();

        files.push(file_path);

        let color = kind_color(&entry.kind, no_color);
        let reset = color_reset(no_color);
        println!(
            "  {color}➔ [{}] {display}{reset}",
            entry.kind,
            display = display_path
        );
    }

    render_tree(&files, no_color)?;

    if print {
        println!();
        println!("📦 Printing file contents:");

        let progress = ProgressRunner::new(files.len() as u64, progress_opt_in);

        for file in &files {
            match print_mode {
                PrintMode::Index => {
                    progress.run(file, || -> Result<()> {
                        emit_file(PrintSource::Index, file, HeadFallback::DeletedInIndex)?;
                        println!();
                        Ok(())
                    })?;
                }
                PrintMode::Worktree => {
                    progress.run(file, || -> Result<()> {
                        emit_file(PrintSource::Worktree, file, HeadFallback::FromHead)?;
                        println!();
                        Ok(())
                    })?;
                }
            }
        }

        progress.finish();
    }

    Ok(files)
}

pub fn print_all_files(
    files: &[String],
    staged_lines: &[String],
    unstaged_lines: &[String],
    progress_opt_in: bool,
) -> Result<()> {
    println!();
    println!("📦 Printing file contents:");

    let staged_paths = collect_paths(staged_lines);
    let unstaged_paths = collect_paths(unstaged_lines);

    let total_ops = files
        .iter()
        .map(|file| {
            let staged = staged_paths.contains(file) as u64;
            let unstaged = unstaged_paths.contains(file) as u64;
            let ops = staged + unstaged;
            if ops == 0 {
                1
            } else {
                ops
            }
        })
        .sum::<u64>();

    let progress = ProgressRunner::new(total_ops, progress_opt_in);

    for file in files {
        let mut printed = false;

        if staged_paths.contains(file) {
            progress.run(format!("{file} (index)"), || -> Result<()> {
                emit_file(PrintSource::Index, file, HeadFallback::DeletedInIndex)?;
                println!();
                Ok(())
            })?;
            printed = true;
        }

        if unstaged_paths.contains(file) {
            progress.run(format!("{file} (working tree)"), || -> Result<()> {
                emit_file(PrintSource::Worktree, file, HeadFallback::FromHead)?;
                println!();
                Ok(())
            })?;
            printed = true;
        }

        if !printed {
            progress.run(file, || -> Result<()> {
                emit_file(PrintSource::Worktree, file, HeadFallback::FromHead)?;
                println!();
                Ok(())
            })?;
        }
    }

    progress.finish();

    Ok(())
}

fn collect_paths(lines: &[String]) -> BTreeSet<String> {
    parse_name_status_lines(lines)
        .into_iter()
        .map(|entry| entry.file_path())
        .collect()
}

fn kind_color(kind: &str, no_color: bool) -> &'static str {
    if no_color {
        return "";
    }
    match kind {
        "A" => "\x1b[38;5;66m",
        "M" => "\x1b[38;5;110m",
        "D" => "\x1b[38;5;95m",
        "U" => "\x1b[38;5;110m",
        "-" => "\x1b[0m",
        _ => "\x1b[38;5;110m",
    }
}

fn color_reset(no_color: bool) -> &'static str {
    if no_color {
        ""
    } else {
        "\x1b[0m"
    }
}

fn render_tree(files: &[String], no_color: bool) -> Result<()> {
    if files.is_empty() {
        println!("⚠️ No files to render as tree");
        return Ok(());
    }

    println!();
    println!("📂 Directory tree:");

    let support = tree_support();
    if !support.is_installed || !support.supports_fromfile {
        if let Some(warning) = support.warning {
            println!("{warning}");
        } else if !support.is_installed {
            println!("{TREE_MISSING_WARNING}");
        } else {
            println!("{TREE_UNSUPPORTED_WARNING}");
        }
        return Ok(());
    }

    let mut tree_args = vec!["--fromfile"];
    if !no_color {
        tree_args.push("-C");
    }

    let tree_input = expand_tree_paths(files);
    let mut cmd = Command::new("tree");
    cmd.args(&tree_args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());

    let mut child = cmd.spawn()?;
    {
        let stdin = child.stdin.as_mut().expect("tree stdin");
        use std::io::Write;
        for line in tree_input {
            writeln!(stdin, "{line}")?;
        }
    }

    let output = child.wait_with_output()?;
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if no_color {
        text = strip_ansi(&text);
    }
    print!("{text}");
    Ok(())
}

pub fn kind_color_for_commit(kind: &str, no_color: bool) -> &'static str {
    kind_color(kind, no_color)
}

pub fn color_reset_for_commit(no_color: bool) -> &'static str {
    color_reset(no_color)
}

pub fn render_tree_for_commit(files: &[String], no_color: bool) -> Result<()> {
    render_tree(files, no_color)
}

fn expand_tree_paths(files: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for file in files {
        let parts: Vec<&str> = file.split('/').collect();
        if parts.is_empty() {
            continue;
        }
        for i in 1..=parts.len() {
            let path = parts[..i].join("/");
            if !path.is_empty() {
                set.insert(path);
            }
        }
    }
    set.into_iter().collect()
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some('[')) {
            chars.next();
            for c in chars.by_ref() {
                if c == 'm' {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

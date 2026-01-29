use crate::print::{print_file_content, print_file_content_index};
use anyhow::Result;
use std::collections::BTreeSet;
use std::process::Command;

#[derive(Debug, Clone, Copy)]
pub enum PrintMode {
    Worktree,
    Index,
}

#[derive(Debug, Clone)]
struct ChangeLine {
    kind: String,
    src: String,
    dest: Option<String>,
}

pub fn render_with_type(
    lines: &[String],
    no_color: bool,
    print_mode: PrintMode,
    print: bool,
) -> Result<Vec<String>> {
    if lines.is_empty() {
        println!("⚠️  No matching files");
        return Ok(Vec::new());
    }

    println!();
    println!("📄 Changed files:");

    let mut files: Vec<String> = Vec::new();

    for entry in parse_lines(lines) {
        let display_path = if is_rename_or_copy(&entry.kind) {
            match entry.dest.as_ref() {
                Some(dest) => format!("{} -> {}", entry.src, dest),
                None => entry.src.clone(),
            }
        } else {
            entry.src.clone()
        };

        let file_path = if is_rename_or_copy(&entry.kind) {
            entry.dest.clone().unwrap_or_else(|| entry.src.clone())
        } else {
            entry.src.clone()
        };

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
        for file in &files {
            match print_mode {
                PrintMode::Index => print_file_content_index(file)?,
                PrintMode::Worktree => print_file_content(file)?,
            }
            println!();
        }
    }

    Ok(files)
}

pub fn print_all_files(
    files: &[String],
    staged_lines: &[String],
    unstaged_lines: &[String],
) -> Result<()> {
    println!();
    println!("📦 Printing file contents:");

    let staged_paths = collect_paths(staged_lines);
    let unstaged_paths = collect_paths(unstaged_lines);

    for file in files {
        let mut printed = false;

        if staged_paths.contains(file) {
            print_file_content_index(file)?;
            printed = true;
            println!();
        }

        if unstaged_paths.contains(file) {
            print_file_content(file)?;
            printed = true;
            println!();
        }

        if !printed {
            print_file_content(file)?;
            println!();
        }
    }

    Ok(())
}

fn collect_paths(lines: &[String]) -> BTreeSet<String> {
    parse_lines(lines)
        .into_iter()
        .map(|entry| {
            if is_rename_or_copy(&entry.kind) {
                entry.dest.unwrap_or(entry.src)
            } else {
                entry.src
            }
        })
        .collect()
}

fn parse_lines(lines: &[String]) -> Vec<ChangeLine> {
    let mut entries = Vec::new();
    for line in lines {
        let mut parts = line.split('\t');
        let kind = match parts.next() {
            Some(k) => k.to_string(),
            None => continue,
        };
        let src = match parts.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let dest = parts.next().map(|s| s.to_string());
        entries.push(ChangeLine { kind, src, dest });
    }
    entries
}

fn is_rename_or_copy(kind: &str) -> bool {
    kind.starts_with('R') || kind.starts_with('C')
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

    if Command::new("tree").arg("--version").output().is_err() {
        println!("⚠️  tree is not installed. Install it to see the directory tree.");
        return Ok(());
    }

    let support = Command::new("tree")
        .arg("--fromfile")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    if support.map(|s| !s.success()).unwrap_or(true) {
        println!(
            "⚠️  tree does not support --fromfile. Please upgrade tree to enable directory tree output."
        );
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

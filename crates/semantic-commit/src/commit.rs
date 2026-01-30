use crate::{codex, git};
use std::fs::File;
use std::io::{BufRead, BufReader, IsTerminal, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub fn run(args: &[String]) -> i32 {
    let mut message: Option<String> = None;
    let mut message_file: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage_stdout();
                return 0;
            }
            "--message" => {
                let value = match args.get(i + 1) {
                    Some(value) => value.clone(),
                    None => {
                        eprintln!("error: --message requires a value");
                        print_usage_stderr();
                        return 1;
                    }
                };
                message = Some(value);
                i += 2;
            }
            "--message-file" => {
                let value = match args.get(i + 1) {
                    Some(value) => value.clone(),
                    None => {
                        eprintln!("error: --message-file requires a path");
                        print_usage_stderr();
                        return 1;
                    }
                };
                message_file = Some(value);
                i += 2;
            }
            other => {
                eprintln!("error: unknown argument: {other}");
                print_usage_stderr();
                return 1;
            }
        }
    }

    if message.is_some() && message_file.is_some() {
        eprintln!("error: use only one of --message or --message-file");
        return 1;
    }

    if !git::is_inside_work_tree() {
        eprintln!("error: must run inside a git work tree");
        return 1;
    }

    match git::has_staged_changes() {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("error: no staged changes (stage files with git add first)");
            return 2;
        }
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    let message_contents = match (message, message_file) {
        (Some(text), None) => text,
        (None, Some(path)) => match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => {
                eprintln!("error: message file not found: {path}");
                return 1;
            }
        },
        (None, None) => {
            if std::io::stdin().is_terminal() {
                eprintln!(
                    "error: no commit message provided (use stdin, --message, or --message-file)"
                );
                print_usage_stderr();
                return 1;
            }

            let mut buf = String::new();
            if let Err(err) = std::io::stdin().read_to_string(&mut buf) {
                eprintln!("{err:#}");
                return 1;
            }
            buf
        }
        (Some(_), Some(_)) => unreachable!("validated above"),
    };

    if message_contents.is_empty() {
        eprintln!("error: commit message is empty");
        return 1;
    }

    let tmpfile = match tempfile::NamedTempFile::new() {
        Ok(file) => file,
        Err(_) => {
            eprintln!("error: failed to create temp file for commit message");
            return 1;
        }
    };

    if let Err(err) = write_message_file(tmpfile.path(), &message_contents) {
        eprintln!("{err:#}");
        return 1;
    }

    if let Err(code) = validate_commit_message(tmpfile.path()) {
        return code;
    }

    let status = Command::new("git")
        .args(["commit", "-F"])
        .arg(tmpfile.path())
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(status) if status.success() => {}
        Ok(status) => {
            let rc = status.code().unwrap_or(1);
            eprintln!("error: git commit failed (exit code: {rc})");
            return rc;
        }
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    }

    print_summary()
}

fn print_summary() -> i32 {
    let git_scope = codex::resolve_command("git-scope");
    match git_scope {
        None => {
            eprintln!("warning: git-scope not found; falling back to git show --stat");
            let _ = run_git_show_stat();
            0
        }
        Some(tool) => {
            let status = Command::new(tool)
                .args(["commit", "HEAD", "--no-color"])
                .env("GIT_PAGER", "cat")
                .env("PAGER", "cat")
                .stdin(Stdio::null())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status();

            match status {
                Ok(status) if status.success() => 0,
                _ => {
                    eprintln!("warning: git-scope commit failed; falling back to git show --stat");
                    run_git_show_stat()
                }
            }
        }
    }
}

fn run_git_show_stat() -> i32 {
    let status = Command::new("git")
        .args(["show", "--no-color", "--stat", "HEAD"])
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    match status {
        Ok(status) if status.success() => 0,
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    }
}

fn write_message_file(path: &Path, contents: &str) -> anyhow::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(contents.as_bytes())?;
    Ok(())
}

fn validate_commit_message(path: &Path) -> Result<(), i32> {
    let file = File::open(path).map_err(|_| {
        eprintln!("error: commit message validation failed");
        1
    })?;

    let reader = BufReader::new(file);
    let mut lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(line) => lines.push(line),
            Err(_) => {
                return fail_validation("commit message validation failed");
            }
        }
    }

    if lines.is_empty() {
        return fail_validation("commit message is empty");
    }

    let header = &lines[0];
    if header.is_empty() {
        return fail_validation("commit header is empty");
    }
    if header.chars().count() > 100 {
        return fail_validation("commit header exceeds 100 characters (max 100)");
    }
    if !is_valid_header(header) {
        return fail_validation(
            "invalid header format (expected 'type(scope): subject' or 'type: subject' with lowercase type)",
        );
    }

    let body_exists = lines.iter().skip(1).any(|line| !line.is_empty());
    if body_exists {
        if lines.get(1).is_some_and(|line| !line.is_empty()) {
            return fail_validation("commit body must be separated from header by a blank line");
        }

        for (idx, line) in lines.iter().enumerate().skip(2) {
            let line_no = idx + 1;
            if line.is_empty() {
                return fail_validation(&format!(
                    "commit body line {line_no} is empty; body lines must start with '- ' followed by uppercase letter"
                ));
            }
            if line.chars().count() > 100 {
                return fail_validation(&format!(
                    "commit body line {line_no} exceeds 100 characters (max 100)"
                ));
            }
            if !line.starts_with("- ")
                || line
                    .chars()
                    .nth(2)
                    .map(|c| !c.is_ascii_uppercase())
                    .unwrap_or(true)
            {
                return fail_validation(&format!(
                    "commit body line {line_no} must start with '- ' followed by uppercase letter"
                ));
            }
        }
    }

    Ok(())
}

fn fail_validation(message: &str) -> Result<(), i32> {
    eprintln!("error: {message}");
    Err(1)
}

fn is_valid_header(header: &str) -> bool {
    // Regex parity: ^[a-z][a-z0-9-]*(\([a-z0-9._-]+\))?: .+$
    let Some((prefix, subject)) = header.split_once(": ") else {
        return false;
    };
    if subject.is_empty() {
        return false;
    }

    let (typ, scope) = if let Some((t, rest)) = prefix.split_once('(') {
        let Some(scope_end) = rest.strip_suffix(')') else {
            return false;
        };
        (t, Some(scope_end))
    } else {
        (prefix, None)
    };

    let mut chars = typ.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        return false;
    }

    if let Some(scope) = scope {
        if scope.is_empty() {
            return false;
        }
        if !scope.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
        }) {
            return false;
        }
    }

    true
}

fn print_usage_stdout() {
    print_usage(false);
}

fn print_usage_stderr() {
    print_usage(true);
}

fn print_usage(stderr: bool) {
    let out: &mut dyn std::io::Write = if stderr {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };

    let _ = writeln!(out, "Usage:");
    let _ = writeln!(
        out,
        "  semantic-commit commit [--message <text> | --message-file <path>]"
    );
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Reads a prepared commit message (prefer stdin for multi-line messages), runs:"
    );
    let _ = writeln!(out, "  git commit -F <temp-file>");
    let _ = writeln!(out, "Then prints:");
    let _ = writeln!(out, "  git-scope commit HEAD --no-color");
    let _ = writeln!(out);
    let _ = writeln!(out, "Examples:");
    let _ = writeln!(out, "  cat <<'MSG' | semantic-commit commit");
    let _ = writeln!(out, "  feat(core): add thing");
    let _ = writeln!(out);
    let _ = writeln!(out, "  - Add thing");
    let _ = writeln!(out, "  MSG");
    let _ = writeln!(out);
    let _ = writeln!(out, "  semantic-commit commit --message-file ./message.txt");
}

use crate::{codex, git};
use std::process::{Command, Stdio};

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        print_usage_stdout();
        return 0;
    }

    if let Some(arg) = args.first() {
        eprintln!("error: unknown argument: {arg}");
        print_usage_stderr();
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

    if let Some(tool) = codex::resolve_command("git-commit-context-json") {
        let status = Command::new(tool)
            .args(["--stdout", "--bundle"])
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .status();

        match status {
            Ok(status) if status.success() => return 0,
            Ok(_) => eprintln!("warning: git-commit-context-json failed; falling back"),
            Err(_) => eprintln!("warning: git-commit-context-json failed; falling back"),
        }
    }

    eprintln!("warning: printing fallback staged diff only");
    let mut child = match Command::new("git")
        .args(["diff", "--staged", "--no-color"])
        .env("GIT_PAGER", "cat")
        .env("PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            eprintln!("{err:#}");
            return 1;
        }
    };

    match child.wait() {
        Ok(status) if status.success() => 0,
        Ok(status) => status.code().unwrap_or(1),
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    }
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

    let _ = writeln!(out, "Usage: semantic-commit staged-context");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "Print staged change context for commit message generation."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "Prefers:");
    let _ = writeln!(out, "  git-commit-context-json --stdout --bundle");
}

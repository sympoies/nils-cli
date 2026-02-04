use std::ffi::OsString;
use std::io::{self, Write};

use crate::commit;
use crate::{branch, ci, reset, utils};

#[derive(Debug, Clone, Copy)]
enum Group {
    Utils,
    Reset,
    Commit,
    Branch,
    Ci,
}

impl Group {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "utils" => Some(Self::Utils),
            "reset" => Some(Self::Reset),
            "commit" => Some(Self::Commit),
            "branch" => Some(Self::Branch),
            "ci" => Some(Self::Ci),
            _ => None,
        }
    }
}

pub fn dispatch(args: Vec<OsString>) -> i32 {
    let args: Vec<String> = args
        .into_iter()
        .map(|v| v.to_string_lossy().to_string())
        .collect();

    if args.is_empty() {
        print_top_level_usage(&mut io::stdout());
        return 0;
    }

    let group_raw = &args[0];
    if is_help_token(group_raw) {
        print_top_level_usage(&mut io::stdout());
        return 0;
    }

    let cmd_raw = args.get(1);
    if cmd_raw.is_none() || cmd_raw.is_some_and(|v| is_help_token(v)) {
        return print_group_usage(group_raw);
    }

    let cmd_raw = cmd_raw.expect("cmd present");
    match Group::parse(group_raw) {
        Some(Group::Utils) => match utils::dispatch(cmd_raw, &args[2..]) {
            Some(code) => code,
            None => {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                2
            }
        },
        Some(Group::Reset) => match reset::dispatch(cmd_raw, &args[2..]) {
            Some(code) => code,
            None => {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                2
            }
        },
        Some(Group::Commit) => {
            let known = [
                "context",
                "context-json",
                "context_json",
                "contextjson",
                "json",
                "to-stash",
                "stash",
            ];
            if !known.contains(&cmd_raw.as_str()) {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                return 2;
            }
            commit::dispatch(cmd_raw, &args[2..])
        }
        Some(Group::Branch) => match branch::dispatch(cmd_raw, &args[2..]) {
            Some(code) => code,
            None => {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                2
            }
        },
        Some(Group::Ci) => match ci::dispatch(cmd_raw, &args[2..]) {
            Some(code) => code,
            None => {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                2
            }
        },
        None => {
            eprintln!("Unknown group: {group_raw}");
            print_top_level_usage(&mut io::stdout());
            2
        }
    }
}

fn is_help_token(raw: &str) -> bool {
    matches!(raw, "-h" | "--help" | "help")
}

fn print_group_usage(group_raw: &str) -> i32 {
    let mut out = io::stdout();

    match Group::parse(group_raw) {
        Some(Group::Utils) => {
            writeln!(out, "Usage: git-cli utils <command> [args]").ok();
            writeln!(out, "  zip | copy-staged | root | commit-hash").ok();
            0
        }
        Some(Group::Reset) => {
            writeln!(out, "Usage: git-cli reset <command> [args]").ok();
            writeln!(
                out,
                "  soft | mixed | hard | undo | back-head | back-checkout | remote"
            )
            .ok();
            0
        }
        Some(Group::Commit) => {
            writeln!(out, "Usage: git-cli commit <command> [args]").ok();
            writeln!(out, "  context | context-json | to-stash").ok();
            0
        }
        Some(Group::Branch) => {
            writeln!(out, "Usage: git-cli branch <command> [args]").ok();
            writeln!(out, "  cleanup").ok();
            0
        }
        Some(Group::Ci) => {
            writeln!(out, "Usage: git-cli ci <command> [args]").ok();
            writeln!(out, "  pick").ok();
            0
        }
        None => {
            eprintln!("Unknown group: {group_raw}");
            print_top_level_usage(&mut out);
            2
        }
    }
}

fn print_top_level_usage(out: &mut dyn Write) {
    writeln!(out, "Usage:").ok();
    writeln!(out, "  git-cli <group> <command> [args]").ok();
    writeln!(out).ok();
    writeln!(out, "Groups:").ok();
    writeln!(out, "  utils    zip | copy-staged | root | commit-hash").ok();
    writeln!(
        out,
        "  reset    soft | mixed | hard | undo | back-head | back-checkout | remote"
    )
    .ok();
    writeln!(out, "  commit   context | context-json | to-stash").ok();
    writeln!(out, "  branch   cleanup").ok();
    writeln!(out, "  ci       pick").ok();
    writeln!(out).ok();
    writeln!(out, "Help:").ok();
    writeln!(out, "  git-cli help").ok();
    writeln!(out, "  git-cli <group> help").ok();
    writeln!(out).ok();
    writeln!(out, "Examples:").ok();
    writeln!(out, "  git-cli utils zip").ok();
    writeln!(out, "  git-cli reset hard 3").ok();
}

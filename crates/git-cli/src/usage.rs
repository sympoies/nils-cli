use std::ffi::OsString;
use std::io::{self, Write};

use crate::commit;
use crate::{branch, ci, completion, open, reset, utils};

#[derive(Debug, Clone, Copy)]
enum Group {
    Utils,
    Reset,
    Commit,
    Branch,
    Ci,
    Open,
    Completion,
}

impl Group {
    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "utils" => Some(Self::Utils),
            "reset" => Some(Self::Reset),
            "commit" => Some(Self::Commit),
            "branch" => Some(Self::Branch),
            "ci" => Some(Self::Ci),
            "open" => Some(Self::Open),
            "completion" => Some(Self::Completion),
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
    if is_version_token(group_raw) {
        print_version(&mut io::stdout());
        return 0;
    }
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
        Some(Group::Open) => match open::dispatch(cmd_raw, &args[2..]) {
            Some(code) => code,
            None => {
                eprintln!("Unknown {group_raw} command: {cmd_raw}");
                let _ = print_group_usage(group_raw);
                2
            }
        },
        Some(Group::Completion) => completion::dispatch(cmd_raw, &args[2..]),
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

fn is_version_token(raw: &str) -> bool {
    matches!(raw, "-V" | "--version")
}

fn print_version(out: &mut dyn Write) {
    writeln!(out, "git-cli {}", env!("CARGO_PKG_VERSION")).ok();
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
        Some(Group::Open) => {
            writeln!(out, "Usage: git-cli open <command> [args]").ok();
            writeln!(
                out,
                "  repo | branch | default-branch | commit | compare | pr | pulls | issues | actions | releases | tags | commits | file | blame"
            )
            .ok();
            0
        }
        Some(Group::Completion) => {
            writeln!(out, "Usage: git-cli completion <shell>").ok();
            writeln!(out, "  bash | zsh").ok();
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
    writeln!(
        out,
        "  open     repo | branch | default-branch | commit | compare | pr | pulls | issues | actions | releases | tags | commits | file | blame"
    )
    .ok();
    writeln!(out, "  completion  bash | zsh").ok();
    writeln!(out).ok();
    writeln!(out, "Help:").ok();
    writeln!(out, "  git-cli help").ok();
    writeln!(out, "  git-cli <group> help").ok();
    writeln!(out).ok();
    writeln!(out, "Examples:").ok();
    writeln!(out, "  git-cli utils zip").ok();
    writeln!(out, "  git-cli reset hard 3").ok();
}

#[cfg(test)]
mod tests {
    use super::{
        Group, dispatch, is_help_token, is_version_token, print_group_usage, print_top_level_usage,
    };
    use std::ffi::OsString;

    fn to_args(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn group_parse_recognizes_known_groups() {
        assert!(matches!(Group::parse("utils"), Some(Group::Utils)));
        assert!(matches!(Group::parse("reset"), Some(Group::Reset)));
        assert!(matches!(Group::parse("commit"), Some(Group::Commit)));
        assert!(matches!(Group::parse("branch"), Some(Group::Branch)));
        assert!(matches!(Group::parse("ci"), Some(Group::Ci)));
        assert!(matches!(Group::parse("open"), Some(Group::Open)));
        assert!(matches!(
            Group::parse("completion"),
            Some(Group::Completion)
        ));
        assert!(Group::parse("unknown").is_none());
    }

    #[test]
    fn help_token_detection_matches_cli_aliases() {
        assert!(is_help_token("-h"));
        assert!(is_help_token("--help"));
        assert!(is_help_token("help"));
        assert!(!is_help_token("HELP"));
    }

    #[test]
    fn version_token_detection_matches_cli_aliases() {
        assert!(is_version_token("-V"));
        assert!(is_version_token("--version"));
        assert!(!is_version_token("-v"));
    }

    #[test]
    fn dispatch_returns_two_for_unknown_group_or_command() {
        assert_eq!(dispatch(to_args(&["unknown", "cmd"])), 2);
        assert_eq!(dispatch(to_args(&["reset", "unknown"])), 2);
        assert_eq!(dispatch(to_args(&["branch", "unknown"])), 2);
        assert_eq!(dispatch(to_args(&["ci", "unknown"])), 2);
        assert_eq!(dispatch(to_args(&["open", "unknown"])), 2);
        assert_eq!(dispatch(to_args(&["completion", "fish"])), 1);
    }

    #[test]
    fn commit_group_unknown_command_is_rejected_before_runtime() {
        assert_eq!(dispatch(to_args(&["commit", "unknown"])), 2);
    }

    #[test]
    fn dispatch_version_flag_returns_zero() {
        assert_eq!(dispatch(to_args(&["-V"])), 0);
        assert_eq!(dispatch(to_args(&["--version"])), 0);
    }

    #[test]
    fn print_group_usage_supports_each_group_and_unknown() {
        assert_eq!(print_group_usage("utils"), 0);
        assert_eq!(print_group_usage("reset"), 0);
        assert_eq!(print_group_usage("commit"), 0);
        assert_eq!(print_group_usage("branch"), 0);
        assert_eq!(print_group_usage("ci"), 0);
        assert_eq!(print_group_usage("open"), 0);
        assert_eq!(print_group_usage("completion"), 0);
        assert_eq!(print_group_usage("unknown"), 2);
    }

    #[test]
    fn print_top_level_usage_includes_required_sections() {
        let mut out = Vec::<u8>::new();
        print_top_level_usage(&mut out);
        let text = String::from_utf8(out).expect("utf8");

        assert!(text.contains("Usage:"));
        assert!(text.contains("Groups:"));
        assert!(text.contains("Examples:"));
        assert!(text.contains("git-cli reset hard 3"));
        assert!(text.contains("completion  bash | zsh"));
    }
}

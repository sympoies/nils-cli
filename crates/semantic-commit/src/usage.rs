use crate::{commit, staged_context};

pub fn dispatch(args: &[String]) -> i32 {
    if args.len() <= 1 {
        print_help_stdout();
        return 0;
    }

    let subcommand = args[1].as_str();
    if is_help(subcommand) {
        print_help_stdout();
        return 0;
    }

    match subcommand {
        "staged-context" => staged_context::run(&args[2..]),
        "commit" => commit::run(&args[2..]),
        "help" => {
            print_help_stdout();
            0
        }
        other => {
            eprintln!("error: unknown argument: {other}");
            print_help_stderr();
            1
        }
    }
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "-h" | "--help")
}

pub fn print_help_stdout() {
    print_help(false);
}

pub fn print_help_stderr() {
    print_help(true);
}

fn print_help(stderr: bool) {
    let out: &mut dyn std::io::Write = if stderr {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };

    let _ = writeln!(out, "Usage: semantic-commit <command> [args]");
    let _ = writeln!(out);
    let _ = writeln!(out, "Commands:");
    let _ = writeln!(
        out,
        "  {:<16}  Print staged change context for commit message generation",
        "staged-context"
    );
    let _ = writeln!(
        out,
        "  {:<16}  Commit staged changes with a prepared commit message",
        "commit"
    );
    let _ = writeln!(out, "  {:<16}  Display help message", "help");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_help_when_no_args() {
        let code = dispatch(&["semantic-commit".to_string()]);
        assert_eq!(code, 0);
    }

    #[test]
    fn dispatch_help_flag_is_zero() {
        let code = dispatch(&["semantic-commit".to_string(), "--help".to_string()]);
        assert_eq!(code, 0);
    }

    #[test]
    fn dispatch_unknown_command_exits_one() {
        let code = dispatch(&["semantic-commit".to_string(), "nope".to_string()]);
        assert_eq!(code, 1);
    }
}

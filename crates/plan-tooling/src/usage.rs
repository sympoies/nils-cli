use crate::{batches, scaffold, validate};

pub fn dispatch(args: &[String]) -> i32 {
    if args.len() <= 1 {
        print_help_stdout();
        return 0;
    }

    let subcommand = args[1].as_str();
    if is_help(subcommand) || subcommand == "help" {
        print_help_stdout();
        return 0;
    }

    match subcommand {
        "to-json" => crate::parse::to_json::run(&args[2..]),
        "validate" => validate::run(&args[2..]),
        "batches" => batches::run(&args[2..]),
        "scaffold" => scaffold::run(&args[2..]),
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

    let _ = writeln!(out, "Usage: plan-tooling <command> [args]");
    let _ = writeln!(out);
    let _ = writeln!(out, "Commands:");
    let _ = writeln!(
        out,
        "  {:<10}  Parse a plan markdown file into a stable JSON schema",
        "to-json"
    );
    let _ = writeln!(out, "  {:<10}  Lint plan markdown files", "validate");
    let _ = writeln!(
        out,
        "  {:<10}  Compute dependency layers (parallel batches) for a sprint",
        "batches"
    );
    let _ = writeln!(out, "  {:<10}  Create a new plan from template", "scaffold");
    let _ = writeln!(out, "  {:<10}  Display help message", "help");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_help_when_no_args() {
        let code = dispatch(&["plan-tooling".to_string()]);
        assert_eq!(code, 0);
    }

    #[test]
    fn dispatch_help_flag_is_zero() {
        let code = dispatch(&["plan-tooling".to_string(), "-h".to_string()]);
        assert_eq!(code, 0);
    }

    #[test]
    fn dispatch_unknown_command_exits_one() {
        let code = dispatch(&["plan-tooling".to_string(), "nope".to_string()]);
        assert_eq!(code, 1);
    }
}

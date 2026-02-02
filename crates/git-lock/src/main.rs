use clap::{Parser, Subcommand};

mod copy;
mod delete;
mod diff;
mod fs;
mod git;
mod list;
mod lock;
mod lock_view;
mod messages;
mod prompt;
mod store;
mod tag;
mod unlock;

#[derive(Parser)]
#[command(
    name = "git-lock",
    disable_help_flag = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    Lock {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Unlock {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    List {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Copy {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Delete {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Diff {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Tag {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    Help,
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 && is_help(&args[1]) {
        messages::print_help();
        return 0;
    }

    if !git::is_git_repo() {
        println!("{}", messages::NOT_GIT_REPO);
        return 1;
    }

    if args.len() <= 1 {
        messages::print_help();
        return 0;
    }

    if !is_known_command(&args[1]) {
        println!("{}", messages::unknown_command(&args[1]));
        println!("{}", messages::UNKNOWN_COMMAND_HINT);
        return 1;
    }

    let cli = Cli::parse_from(&args);

    let result = match cli.command.unwrap_or(Command::Help) {
        Command::Lock { args } => lock::run(&args),
        Command::Unlock { args } => unlock::run(&args),
        Command::List { args } => list::run(&args),
        Command::Copy { args } => copy::run(&args),
        Command::Delete { args } => delete::run(&args),
        Command::Diff { args } => diff::run(&args),
        Command::Tag { args } => tag::run(&args),
        Command::Help => {
            messages::print_help();
            Ok(0)
        }
    };

    match result {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{err:#}");
            1
        }
    }
}

fn is_help(arg: &str) -> bool {
    matches!(arg, "help" | "--help" | "-h")
}

fn is_known_command(arg: &str) -> bool {
    matches!(
        arg,
        "lock" | "unlock" | "list" | "copy" | "delete" | "diff" | "tag" | "help"
    )
}

#[cfg(test)]
mod tests {
    use super::{is_help, is_known_command};

    #[test]
    fn is_help_matches_expected_flags() {
        assert!(is_help("help"));
        assert!(is_help("--help"));
        assert!(is_help("-h"));
        assert!(!is_help("lock"));
    }

    #[test]
    fn is_known_command_accepts_known() {
        assert!(is_known_command("lock"));
        assert!(is_known_command("unlock"));
        assert!(is_known_command("list"));
        assert!(is_known_command("copy"));
        assert!(is_known_command("delete"));
        assert!(is_known_command("diff"));
        assert!(is_known_command("tag"));
        assert!(is_known_command("help"));
        assert!(!is_known_command("nope"));
    }
}

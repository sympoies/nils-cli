use clap::{Parser, Subcommand};

mod copy;
mod delete;
mod diff;
mod fs;
mod git;
mod list;
mod lock;
mod prompt;
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
    if !git::is_git_repo() {
        println!("❗ Not a Git repository. Run this command inside a Git project.");
        return 1;
    }

    let args: Vec<String> = std::env::args().collect();
    if args.len() <= 1 || is_help(&args[1]) {
        print_help();
        return 0;
    }

    if !is_known_command(&args[1]) {
        println!("❗ Unknown command: '{}'", args[1]);
        println!("Run 'git-lock help' for usage.");
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
            print_help();
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

fn print_help() {
    println!("Usage: git-lock <command> [args]");
    println!();
    println!("Commands:");
    println!(
        "  {:<16}  Save commit hash to lock",
        "lock [label] [note] [commit]"
    );
    println!("  {:<16}  Reset to a saved commit", "unlock [label]");
    println!("  {:<16}  Show all locks for repo", "list");
    println!("  {:<16}  Duplicate a lock label", "copy <from> <to>");
    println!("  {:<16}  Remove a lock", "delete [label]");
    println!(
        "  {:<16}  Compare commits between two locks",
        "diff <l1> <l2> [--no-color]"
    );
    println!(
        "  {:<16}  Create git tag from a lock",
        "tag <label> <tag> [-m msg]"
    );
    println!();
}

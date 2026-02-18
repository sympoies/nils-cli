use clap::{Arg, Command};
use clap_complete::{Generator, Shell, generate};
use std::io;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum CompletionShell {
    Bash,
    Zsh,
}

pub fn maybe_handle_completion_export(args: &[String]) -> Option<i32> {
    if args.first().map(String::as_str) != Some("completion") {
        return None;
    }

    match args.get(1).map(String::as_str) {
        None => {
            eprintln!("usage: git-summary completion <bash|zsh>");
            Some(1)
        }
        Some("bash") if args.len() == 2 => Some(run(CompletionShell::Bash)),
        Some("zsh") if args.len() == 2 => Some(run(CompletionShell::Zsh)),
        Some(shell) if args.len() == 2 => {
            eprintln!("git-summary: error: unsupported completion shell '{shell}'");
            eprintln!("usage: git-summary completion <bash|zsh>");
            Some(1)
        }
        _ => {
            eprintln!("git-summary: error: expected `git-summary completion <bash|zsh>`");
            Some(1)
        }
    }
}

fn run(shell: CompletionShell) -> i32 {
    match shell {
        CompletionShell::Bash => generate_script(Shell::Bash),
        CompletionShell::Zsh => generate_script(Shell::Zsh),
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = build_completion_command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

fn build_completion_command() -> Command {
    Command::new("git-summary")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Git history summary CLI")
        .disable_help_subcommand(true)
        .arg(
            Arg::new("from")
                .value_name("from")
                .help("Custom range start date (YYYY-MM-DD)")
                .required(false),
        )
        .arg(
            Arg::new("to")
                .value_name("to")
                .help("Custom range end date (YYYY-MM-DD)")
                .required(false),
        )
        .subcommand(Command::new("all").about("Entire history"))
        .subcommand(Command::new("today").about("Today only"))
        .subcommand(Command::new("yesterday").about("Yesterday only"))
        .subcommand(Command::new("this-month").about("1st to today"))
        .subcommand(Command::new("last-month").about("1st to end of last month"))
        .subcommand(Command::new("this-week").about("This Mon-Sun"))
        .subcommand(Command::new("last-week").about("Last Mon-Sun"))
        .subcommand(Command::new("help").about("Display help message for git-summary"))
        .subcommand(
            Command::new("completion")
                .about("Export shell completion script")
                .arg(
                    Arg::new("shell")
                        .value_name("shell")
                        .value_parser(["bash", "zsh"])
                        .required(true),
                ),
        )
}

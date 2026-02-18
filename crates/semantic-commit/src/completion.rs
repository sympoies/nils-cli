use clap::{Arg, ArgAction, Command, ValueHint};
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            eprintln!("usage: semantic-commit completion <bash|zsh>");
            1
        }
        Some("bash") if args.len() == 1 => generate_script(Shell::Bash),
        Some("zsh") if args.len() == 1 => generate_script(Shell::Zsh),
        Some(shell) if args.len() == 1 => {
            eprintln!("semantic-commit: error: unsupported completion shell '{shell}'");
            eprintln!("usage: semantic-commit completion <bash|zsh>");
            1
        }
        _ => {
            eprintln!("semantic-commit: error: expected `semantic-commit completion <bash|zsh>`");
            1
        }
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = build_completion_command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

fn build_completion_command() -> Command {
    Command::new("semantic-commit")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Commit workflow helper with semantic commit validation")
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("staged-context")
                .about("Print staged change context for commit message generation")
                .arg(
                    Arg::new("format")
                        .long("format")
                        .value_name("bundle|json|patch")
                        .value_parser(["bundle", "json", "patch"]),
                )
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
                .arg(
                    Arg::new("repo")
                        .long("repo")
                        .value_name("path")
                        .value_hint(ValueHint::DirPath),
                ),
        )
        .subcommand(
            Command::new("commit")
                .about("Commit staged changes with a prepared commit message")
                .arg(
                    Arg::new("message")
                        .short('m')
                        .long("message")
                        .value_name("text"),
                )
                .arg(
                    Arg::new("message-file")
                        .short('F')
                        .long("message-file")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(
                    Arg::new("message-out")
                        .long("message-out")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(
                    Arg::new("summary")
                        .long("summary")
                        .value_name("git-scope|git-show|none")
                        .value_parser(["git-scope", "git-show", "none"]),
                )
                .arg(
                    Arg::new("no-summary")
                        .long("no-summary")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("repo")
                        .long("repo")
                        .value_name("path")
                        .value_hint(ValueHint::DirPath),
                )
                .arg(
                    Arg::new("automation")
                        .long("automation")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("non-interactive")
                        .long("non-interactive")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("validate-only")
                        .long("validate-only")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("no-progress")
                        .long("no-progress")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("quiet").long("quiet").action(ArgAction::SetTrue)),
        )
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
        .subcommand(Command::new("help").about("Display help message"))
}

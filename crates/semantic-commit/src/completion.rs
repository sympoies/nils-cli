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
                        .help("Output format for staged context")
                        .value_name("bundle|json|patch")
                        .value_parser(["bundle", "json", "patch"]),
                )
                .arg(
                    Arg::new("json")
                        .long("json")
                        .help("Alias for --format json")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("repo")
                        .long("repo")
                        .help("Repository path override")
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
                        .help("Inline commit message")
                        .value_name("text"),
                )
                .arg(
                    Arg::new("message-file")
                        .short('F')
                        .long("message-file")
                        .help("Read commit message from file")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(
                    Arg::new("message-out")
                        .long("message-out")
                        .help("Write final commit message to file")
                        .value_name("path")
                        .value_hint(ValueHint::FilePath),
                )
                .arg(
                    Arg::new("summary")
                        .long("summary")
                        .help("Summary provider")
                        .value_name("git-scope|git-show|none")
                        .value_parser(["git-scope", "git-show", "none"]),
                )
                .arg(
                    Arg::new("no-summary")
                        .long("no-summary")
                        .help("Disable summary section")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("repo")
                        .long("repo")
                        .help("Repository path override")
                        .value_name("path")
                        .value_hint(ValueHint::DirPath),
                )
                .arg(
                    Arg::new("automation")
                        .long("automation")
                        .help("Disable interactive prompts and stdin input")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("non-interactive")
                        .long("non-interactive")
                        .help("Fail instead of prompting for input")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("validate-only")
                        .long("validate-only")
                        .help("Validate message and exit without committing")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Print commit plan without running git commit")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("no-progress")
                        .long("no-progress")
                        .help("Disable progress UI")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("quiet")
                        .long("quiet")
                        .help("Reduce non-error output")
                        .action(ArgAction::SetTrue),
                ),
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

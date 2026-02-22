use clap::{Arg, ArgAction, Command};
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            eprintln!("usage: fzf-cli completion <bash|zsh>");
            1
        }
        Some("bash") if args.len() == 1 => generate_script(Shell::Bash),
        Some("zsh") if args.len() == 1 => generate_script(Shell::Zsh),
        Some(shell) if args.len() == 1 => {
            eprintln!("fzf-cli: error: unsupported completion shell '{shell}'");
            eprintln!("usage: fzf-cli completion <bash|zsh>");
            1
        }
        _ => {
            eprintln!("fzf-cli: error: expected `fzf-cli completion <bash|zsh>`");
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

pub(crate) fn print_subcommand_help(subcommand: &str) -> bool {
    let mut command = build_completion_command();
    let Some(subcommand) = command.find_subcommand_mut(subcommand) else {
        return false;
    };
    if subcommand.print_help().is_err() {
        return false;
    }
    println!();
    true
}

fn query_arg() -> Arg {
    Arg::new("query")
        .value_name("query")
        .num_args(0..)
        .allow_hyphen_values(true)
}

pub(crate) fn build_completion_command() -> Command {
    Command::new("fzf-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Fuzzy workflow helper CLI")
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("file")
                .about("Search and preview text files")
                .arg(
                    Arg::new("vi")
                        .long("vi")
                        .help("Open selection with vi")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("vscode")
                        .long("vscode")
                        .help("Open selection with VS Code")
                        .action(ArgAction::SetTrue),
                )
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("directory")
                .about("Search directories and cd into selection")
                .arg(
                    Arg::new("vi")
                        .long("vi")
                        .help("Open selection with vi")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("vscode")
                        .long("vscode")
                        .help("Open selection with VS Code")
                        .action(ArgAction::SetTrue),
                )
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("git-status")
                .about("Interactive git status viewer")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("git-commit")
                .about("Browse commits and open changed files in editor")
                .arg(
                    Arg::new("snapshot")
                        .long("snapshot")
                        .help("Preview file snapshots from selected commit")
                        .action(ArgAction::SetTrue),
                )
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("git-checkout")
                .about("Pick and checkout a previous commit")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("git-branch")
                .about("Browse and checkout branches interactively")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("git-tag")
                .about("Browse and checkout tags interactively")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("process")
                .about("Browse and kill running processes")
                .arg(
                    Arg::new("kill")
                        .short('k')
                        .long("kill")
                        .help("Kill selected process")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .short('9')
                        .long("force")
                        .help("Use SIGKILL when killing")
                        .action(ArgAction::SetTrue),
                )
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("port")
                .about("Browse listening ports and owners")
                .arg(
                    Arg::new("kill")
                        .short('k')
                        .long("kill")
                        .help("Kill owner process for selected port")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .short('9')
                        .long("force")
                        .help("Use SIGKILL when killing")
                        .action(ArgAction::SetTrue),
                )
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("history")
                .about("Search and execute command history")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("env")
                .about("Browse environment variables")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("alias")
                .about("Browse shell aliases")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("function")
                .about("Browse defined shell functions")
                .arg(query_arg()),
        )
        .subcommand(
            Command::new("def")
                .about("Browse all definitions (env, alias, functions)")
                .arg(query_arg()),
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
        .subcommand(Command::new("help").about("Display help message for fzf-cli"))
}

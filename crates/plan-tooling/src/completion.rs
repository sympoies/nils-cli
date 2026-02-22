use clap::{Arg, ArgAction, Command};
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            eprintln!("usage: plan-tooling completion <bash|zsh>");
            1
        }
        Some("bash") if args.len() == 1 => generate_script(Shell::Bash),
        Some("zsh") if args.len() == 1 => generate_script(Shell::Zsh),
        Some(shell) if args.len() == 1 => {
            eprintln!("plan-tooling: error: unsupported completion shell '{shell}'");
            eprintln!("usage: plan-tooling completion <bash|zsh>");
            1
        }
        _ => {
            eprintln!("plan-tooling: error: expected `plan-tooling completion <bash|zsh>`");
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
    Command::new("plan-tooling")
        .version(env!("CARGO_PKG_VERSION"))
        .about("Plan tooling CLI")
        .disable_help_subcommand(true)
        .subcommand(
            Command::new("to-json")
                .about("Parse a plan markdown file into a stable JSON schema")
                .arg(
                    Arg::new("file")
                        .long("file")
                        .help("Plan markdown file path")
                        .value_name("path")
                        .required(false),
                )
                .arg(
                    Arg::new("sprint")
                        .long("sprint")
                        .help("Sprint number to parse")
                        .value_name("n")
                        .required(false),
                )
                .arg(
                    Arg::new("pretty")
                        .long("pretty")
                        .help("Pretty-print JSON output")
                        .action(ArgAction::SetTrue),
                ),
        )
        .subcommand(
            Command::new("validate")
                .about("Lint plan markdown files")
                .arg(
                    Arg::new("file")
                        .long("file")
                        .help("Plan markdown file path (repeatable)")
                        .value_name("path")
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .help("Validation output format")
                        .value_name("fmt")
                        .value_parser(["text", "json"]),
                ),
        )
        .subcommand(
            Command::new("batches")
                .about("Compute dependency layers (parallel batches) for a sprint")
                .arg(
                    Arg::new("file")
                        .long("file")
                        .help("Plan markdown file path")
                        .value_name("path")
                        .required(false),
                )
                .arg(
                    Arg::new("sprint")
                        .long("sprint")
                        .help("Sprint number to analyze")
                        .value_name("n")
                        .required(false),
                )
                .arg(
                    Arg::new("format")
                        .long("format")
                        .help("Batch output format")
                        .value_name("fmt")
                        .value_parser(["json", "text"]),
                ),
        )
        .subcommand(
            Command::new("scaffold")
                .about("Create a new plan from template")
                .arg(
                    Arg::new("slug")
                        .long("slug")
                        .help("Plan slug used for output naming")
                        .value_name("slug"),
                )
                .arg(
                    Arg::new("file")
                        .long("file")
                        .help("Explicit output plan path")
                        .value_name("path"),
                )
                .arg(
                    Arg::new("title")
                        .long("title")
                        .help("Plan title override")
                        .value_name("text"),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Overwrite existing output file")
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

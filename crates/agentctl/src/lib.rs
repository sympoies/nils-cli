pub mod cli;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};

pub fn run() -> i32 {
    run_from(std::env::args())
}

pub fn run_from<I, T>(argv: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = match cli::Cli::try_parse_from(argv) {
        Ok(cli) => cli,
        Err(err) => {
            let code = match err.kind() {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => 0,
                _ => 64,
            };
            let _ = err.print();
            return code;
        }
    };

    match cli.command {
        Some(cli::Command::Provider) => print_group_help("provider"),
        Some(cli::Command::Diag) => print_group_help("diag"),
        Some(cli::Command::Debug) => print_group_help("debug"),
        Some(cli::Command::Workflow) => print_group_help("workflow"),
        Some(cli::Command::Automation) => print_group_help("automation"),
        None => print_root_help(),
    }
}

fn print_root_help() -> i32 {
    let mut cmd = cli::Cli::command();
    if cmd.print_help().is_ok() {
        println!();
        return 0;
    }
    1
}

fn print_group_help(name: &str) -> i32 {
    let mut cmd = cli::Cli::command();
    if let Some(subcommand) = cmd.find_subcommand_mut(name) {
        if subcommand.print_help().is_ok() {
            println!();
            return 0;
        }
    }
    1
}

use clap::{CommandFactory, ValueEnum};
use clap_complete::{Generator, Shell, generate};

use crate::cli::Cli;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
}

pub fn run(shell: CompletionShell) -> i32 {
    let mut command = Cli::command();
    let bin_name = command.get_name().to_string();

    match shell {
        CompletionShell::Bash => print_completion(Shell::Bash, &mut command, &bin_name),
        CompletionShell::Zsh => print_completion(Shell::Zsh, &mut command, &bin_name),
    }

    0
}

fn print_completion<G: Generator>(generator: G, command: &mut clap::Command, bin_name: &str) {
    generate(generator, command, bin_name, &mut std::io::stdout());
}

use std::io;

use clap::CommandFactory;
use clap_complete::{Generator, Shell, generate};

use crate::BinaryFlavor;
use crate::cli::Cli;
use crate::commands::completion::CompletionShell;

pub fn run(binary: BinaryFlavor, shell: CompletionShell) -> i32 {
    let mut command = Cli::command().name(binary.binary_name());
    let bin_name = binary.binary_name();

    match shell {
        CompletionShell::Bash => print_completion(Shell::Bash, &mut command, bin_name),
        CompletionShell::Zsh => print_completion(Shell::Zsh, &mut command, bin_name),
    }

    0
}

fn print_completion<G: Generator>(generator: G, command: &mut clap::Command, bin_name: &str) {
    generate(generator, command, bin_name, &mut io::stdout());
}

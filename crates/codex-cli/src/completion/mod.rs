use clap::CommandFactory;
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn run(shell: crate::cli::CompletionShell) -> i32 {
    match shell {
        crate::cli::CompletionShell::Bash => generate_script(Shell::Bash),
        crate::cli::CompletionShell::Zsh => generate_script(Shell::Zsh),
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = crate::cli::Cli::command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

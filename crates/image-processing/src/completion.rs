use clap::CommandFactory;
use clap_complete::{Generator, Shell, generate};
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompletionShell {
    Bash,
    Zsh,
}

pub fn run(shell: CompletionShell) -> i32 {
    match shell {
        CompletionShell::Bash => generate_script(Shell::Bash),
        CompletionShell::Zsh => generate_script(Shell::Zsh),
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = crate::cli::Cli::command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

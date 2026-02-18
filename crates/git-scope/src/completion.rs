use clap::CommandFactory;
use clap_complete::{Generator, Shell, generate};
use std::io;

pub fn run(shell: crate::CompletionShell) -> i32 {
    match shell {
        crate::CompletionShell::Bash => generate_script(Shell::Bash),
        crate::CompletionShell::Zsh => generate_script(Shell::Zsh),
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = crate::Cli::command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut io::stdout());
    0
}

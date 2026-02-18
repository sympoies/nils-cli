use clap::CommandFactory;
use clap_complete::{Generator, Shell, generate};

use crate::cli::Cli;

pub fn run(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        None => {
            eprintln!("usage: screen-record completion <bash|zsh>");
            1
        }
        Some("bash") if args.len() == 1 => generate_script(Shell::Bash),
        Some("zsh") if args.len() == 1 => generate_script(Shell::Zsh),
        Some(shell) if args.len() == 1 => {
            eprintln!("screen-record: error: unsupported completion shell '{shell}'");
            eprintln!("usage: screen-record completion <bash|zsh>");
            1
        }
        _ => {
            eprintln!("screen-record: error: expected `screen-record completion <bash|zsh>`");
            1
        }
    }
}

fn generate_script<G: Generator>(generator: G) -> i32 {
    let mut command = Cli::command();
    let bin_name = command.get_name().to_string();
    generate(generator, &mut command, bin_name, &mut std::io::stdout());
    0
}

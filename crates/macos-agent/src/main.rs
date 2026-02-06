mod cli;
mod error;
mod preflight;
mod run;

use clap::{error::ErrorKind, Parser};

use crate::cli::Cli;

fn main() {
    std::process::exit(run_cli());
}

fn run_cli() -> i32 {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let is_info = matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            let code = if is_info { 0 } else { err.exit_code() };
            let _ = err.print();
            return code;
        }
    };

    match run::run(cli) {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("{err}");
            i32::from(err.exit_code())
        }
    }
}

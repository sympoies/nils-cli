use std::process::ExitCode;

use clap::Parser;
use screen_record::cli::Cli;
use screen_record::run::run;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(err.exit_code())
        }
    }
}

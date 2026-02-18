use std::process::ExitCode;

use clap::Parser;
use screen_record::cli::Cli;
use screen_record::run::run;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("completion") {
        let code = screen_record::completion::run(&args[2..]);
        return if code == 0 {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        };
    }

    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(err.exit_code())
        }
    }
}

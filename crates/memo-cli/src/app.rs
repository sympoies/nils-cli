use std::ffi::OsString;

use clap::{Parser, error::ErrorKind};

use crate::cli::{Cli, MemoCommand, OutputMode};
use crate::errors::AppError;

pub fn run() -> i32 {
    run_with_args(std::env::args_os())
}

pub fn run_with_args<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(err) => {
            let kind = err.kind();
            match kind {
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                    print!("{err}");
                    return 0;
                }
                _ => {
                    eprint!("{err}");
                    return 64;
                }
            }
        }
    };

    if let MemoCommand::Completion(args) = cli.command {
        return crate::completion::run(args.shell);
    }

    let output_mode = match cli.resolve_output_mode() {
        Ok(mode) => mode,
        Err(err) => {
            eprintln!("{}", err.message());
            return err.exit_code();
        }
    };

    match crate::commands::run(&cli, output_mode) {
        Ok(()) => 0,
        Err(err) => report_error(&cli, output_mode, &err),
    }
}

fn report_error(cli: &Cli, output_mode: OutputMode, err: &AppError) -> i32 {
    if output_mode.is_json()
        && let Err(output_err) =
            crate::output::emit_json_error(cli.schema_version(), cli.command_id(), err)
    {
        eprintln!("{}", output_err.message());
    }

    if !output_mode.is_json() {
        eprintln!("{}", err.message());
    }

    err.exit_code()
}

use std::process;

fn main() {
    let exit_code = git_cli::usage::dispatch(std::env::args_os().skip(1).collect());
    process::exit(exit_code);
}

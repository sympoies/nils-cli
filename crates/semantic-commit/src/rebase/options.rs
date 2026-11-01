use clap::{Arg, ArgAction, ArgGroup, Command, ValueHint};
use std::path::PathBuf;

pub(super) struct Options {
    pub repo: Option<PathBuf>,
    pub upstream: Option<String>,
}

pub(crate) fn clap_command() -> Command {
    Command::new("rebase")
        .about("Replay a non-default managed branch with authenticated signing")
        .arg(Arg::new("upstream").value_name("upstream"))
        .arg(
            Arg::new("continue")
                .long("continue")
                .help("Continue the current replay with authenticated signing")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("repo")
                .long("repo")
                .help("Path to the managed linked worktree")
                .value_name("path")
                .value_hint(ValueHint::DirPath),
        )
        .group(
            ArgGroup::new("action")
                .args(["upstream", "continue"])
                .required(true),
        )
}

pub(super) fn parse(args: &[String]) -> Result<Options, i32> {
    let matches = clap_command()
        .bin_name("semantic-commit rebase")
        .try_get_matches_from(std::iter::once("rebase").chain(args.iter().map(String::as_str)))
        .map_err(|error| {
            let code = if error.use_stderr() { 64 } else { 0 };
            let _ = error.print();
            code
        })?;
    Ok(Options {
        repo: matches.get_one::<String>("repo").map(PathBuf::from),
        upstream: matches.get_one::<String>("upstream").cloned(),
    })
}

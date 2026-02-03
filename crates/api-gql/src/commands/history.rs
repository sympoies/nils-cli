use std::io::Write;
use std::path::{Path, PathBuf};

use api_testing_core::cli_history::{resolve_history_file, run_history_command};
use api_testing_core::cli_util::trim_non_empty;
use api_testing_core::config;

use crate::cli::HistoryArgs;

pub(crate) fn cmd_history(
    args: &HistoryArgs,
    invocation_dir: &Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);

    let history_file = match resolve_history_file(
        invocation_dir,
        config_dir.as_deref(),
        args.file.as_deref(),
        "GQL_HISTORY_FILE",
        |cwd, config_dir| {
            config::resolve_gql_setup_dir_for_history(cwd, invocation_dir, config_dir)
        },
        ".gql_history",
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    run_history_command(&history_file, args.tail, args.command_only, stdout, stderr)
}

use std::io::Write;
use std::path::{Path, PathBuf};

use api_testing_core::{config, history};

use crate::cli::HistoryArgs;
use crate::util::trim_non_empty;

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

    let setup_dir = match config::resolve_gql_setup_dir_for_history(
        invocation_dir,
        invocation_dir,
        config_dir.as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    let file_override = args.file.as_deref().and_then(trim_non_empty).or_else(|| {
        std::env::var("GQL_HISTORY_FILE")
            .ok()
            .and_then(|s| trim_non_empty(&s))
    });
    let file_override = file_override.as_deref().map(Path::new);
    let setup = api_testing_core::config::ResolvedSetup::graphql(setup_dir, file_override);
    let history_file = &setup.history_file;

    if !history_file.is_file() {
        let _ = writeln!(stderr, "History file not found: {}", history_file.display());
        return 1;
    }

    let records = match history::read_records(history_file) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };
    if records.is_empty() {
        return 3;
    }

    let n = args.tail.unwrap_or(1).max(1) as usize;
    let start = records.len().saturating_sub(n);
    for record in &records[start..] {
        if args.command_only && record.starts_with('#') {
            let trimmed = record
                .split_once('\n')
                .map(|(_first, rest)| rest)
                .unwrap_or_default();
            let _ = stdout.write_all(trimmed.as_bytes());
            if trimmed.is_empty() {
                let _ = stdout.write_all(b"\n\n");
            }
        } else {
            let _ = stdout.write_all(record.as_bytes());
        }
    }

    0
}

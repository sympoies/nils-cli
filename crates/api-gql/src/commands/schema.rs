use std::io::Write;
use std::path::PathBuf;

use api_testing_core::cli_util::trim_non_empty;
use api_testing_core::config;

use crate::cli::SchemaArgs;

pub(crate) fn cmd_schema(
    args: &SchemaArgs,
    invocation_dir: &std::path::Path,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
    let config_dir = args
        .config_dir
        .as_deref()
        .and_then(trim_non_empty)
        .map(PathBuf::from);

    let setup_dir = match config::resolve_gql_setup_dir_for_schema(
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

    let schema_path = match api_testing_core::graphql::schema_file::resolve_schema_path(
        &setup_dir,
        args.file.as_deref().and_then(trim_non_empty).as_deref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            let _ = writeln!(stderr, "{err}");
            return 1;
        }
    };

    if args.cat {
        match std::fs::read_to_string(&schema_path) {
            Ok(v) => {
                let _ = writeln!(stdout, "{v}");
            }
            Err(_) => {
                let _ = writeln!(
                    stderr,
                    "error: failed to read schema file: {}",
                    schema_path.display()
                );
                return 1;
            }
        }
    } else {
        let _ = writeln!(stdout, "{}", schema_path.display());
    }

    0
}

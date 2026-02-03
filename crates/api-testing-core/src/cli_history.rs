use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{cli_util, history, Result};

pub fn resolve_history_file<F>(
    cwd: &Path,
    config_dir: Option<&Path>,
    file_override_arg: Option<&str>,
    env_override_var: &str,
    resolve_setup_dir: F,
    default_filename: &str,
) -> Result<PathBuf>
where
    F: FnOnce(&Path, Option<&Path>) -> Result<PathBuf>,
{
    let setup_dir = resolve_setup_dir(cwd, config_dir)?;
    let file_override = file_override_arg
        .and_then(cli_util::trim_non_empty)
        .or_else(|| {
            std::env::var(env_override_var)
                .ok()
                .and_then(|s| cli_util::trim_non_empty(&s))
        });
    let file_override = file_override.as_deref().map(Path::new);

    Ok(history::resolve_history_file(
        &setup_dir,
        file_override,
        default_filename,
    ))
}

pub fn run_history_command(
    history_file: &Path,
    tail: Option<u32>,
    command_only: bool,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> i32 {
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

    let n = tail.unwrap_or(1).max(1) as usize;
    let start = records.len().saturating_sub(n);
    for record in &records[start..] {
        if command_only && record.starts_with('#') {
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

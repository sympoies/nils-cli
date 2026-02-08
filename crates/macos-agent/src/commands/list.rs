use crate::cli::{ListAppsArgs, ListWindowsArgs, OutputFormat};
use crate::commands::emit_json_success;
use crate::error::CliError;
use crate::model::{ListAppsResult, ListWindowsResult};
use crate::targets;

pub fn run_windows_list(format: OutputFormat, args: &ListWindowsArgs) -> Result<(), CliError> {
    let windows = targets::list_windows(args)?;
    match format {
        OutputFormat::Json => {
            emit_json_success("windows.list", ListWindowsResult { windows })?;
        }
        OutputFormat::Text | OutputFormat::Tsv => {
            for row in windows {
                println!("{}", row.tsv_line());
            }
        }
    }

    Ok(())
}

pub fn run_apps_list(format: OutputFormat, _args: &ListAppsArgs) -> Result<(), CliError> {
    let apps = targets::list_apps()?;
    match format {
        OutputFormat::Json => {
            emit_json_success("apps.list", ListAppsResult { apps })?;
        }
        OutputFormat::Text | OutputFormat::Tsv => {
            for row in apps {
                println!("{}", row.tsv_line());
            }
        }
    }

    Ok(())
}

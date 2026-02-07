use crate::cli::{ListAppsArgs, ListWindowsArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{ListAppsResult, ListWindowsResult, SuccessEnvelope};
use crate::targets;

pub fn run_windows_list(format: OutputFormat, args: &ListWindowsArgs) -> Result<(), CliError> {
    let windows = targets::list_windows(args)?;
    match format {
        OutputFormat::Json => {
            let payload = SuccessEnvelope::new("windows.list", ListWindowsResult { windows });
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
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
            let payload = SuccessEnvelope::new("apps.list", ListAppsResult { apps });
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text | OutputFormat::Tsv => {
            for row in apps {
                println!("{}", row.tsv_line());
            }
        }
    }

    Ok(())
}

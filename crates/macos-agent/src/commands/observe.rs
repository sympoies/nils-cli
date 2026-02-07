use std::path::PathBuf;

use crate::cli::{ObserveScreenshotArgs, OutputFormat};
use crate::error::CliError;
use crate::model::{ScreenshotResult, SuccessEnvelope, WindowRow};
use crate::targets::{self, TargetSelector};
use crate::test_mode;

pub fn run_screenshot(format: OutputFormat, args: &ObserveScreenshotArgs) -> Result<(), CliError> {
    let selector = TargetSelector {
        window_id: args.window_id,
        active_window: args.active_window,
        app: args.app.clone(),
        window_name: args.window_name.clone(),
    };

    let window = targets::resolve_window(&selector)?;
    let output_path = resolve_output_path(args, window.id);
    let image_format = args
        .image_format
        .or_else(|| targets::extension_format(&output_path))
        .unwrap_or(crate::cli::ImageFormat::Png);

    targets::capture_screenshot(&output_path, &window, image_format)?;

    match format {
        OutputFormat::Json => {
            let result = ScreenshotResult {
                path: output_path.display().to_string(),
                target: WindowRow::from(&window),
            };
            let payload = SuccessEnvelope::new("observe.screenshot", result);
            println!(
                "{}",
                serde_json::to_string(&payload).map_err(|err| CliError::runtime(format!(
                    "failed to serialize json output: {err}"
                )))?
            );
        }
        OutputFormat::Text => {
            println!("{}", output_path.display());
        }
        OutputFormat::Tsv => {
            return Err(CliError::usage(
                "--format tsv is only supported for `windows list` and `apps list`",
            ));
        }
    }

    Ok(())
}

fn resolve_output_path(args: &ObserveScreenshotArgs, window_id: u32) -> PathBuf {
    args.path.clone().unwrap_or_else(|| {
        let token = test_mode::timestamp_token();
        PathBuf::from(format!("macos-agent-{token}-window-{window_id}.png"))
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::resolve_output_path;
    use crate::cli::ObserveScreenshotArgs;

    #[test]
    fn preserve_explicit_output_path() {
        let args = ObserveScreenshotArgs {
            window_id: Some(1),
            active_window: false,
            app: None,
            window_name: None,
            path: Some(PathBuf::from("./out/image.png")),
            image_format: None,
        };

        assert_eq!(
            resolve_output_path(&args, 123),
            PathBuf::from("./out/image.png")
        );
    }
}

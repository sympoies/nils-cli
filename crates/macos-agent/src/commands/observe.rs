use std::path::PathBuf;

use crate::cli::{ObserveScreenshotArgs, OutputFormat};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{ScreenshotResult, WindowRow};
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
            emit_json_success("observe.screenshot", result)?;
        }
        OutputFormat::Text => {
            println!("{}", output_path.display());
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
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

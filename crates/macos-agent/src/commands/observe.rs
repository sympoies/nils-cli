use std::path::PathBuf;

use crate::backend::process::RealProcessRunner;
use crate::backend::AutoAxBackend;
use crate::cli::{ObserveScreenshotArgs, OutputFormat};
use crate::commands::ax_common::{
    build_selector_from_args, build_target, resolve_selector_node_against_backend,
    selector_args_requested,
};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{AxFrame, ScreenshotResult, ScreenshotSelectorResult, WindowRow};
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

    let selector = if selector_args_requested(&args.ax_selector) {
        let selector = build_selector_from_args(&args.ax_selector)?;
        let ax_target = build_target(
            None,
            args.app.clone().or_else(|| Some(window.owner_name.clone())),
            None,
            args.window_name.clone().or_else(|| {
                if window.title.trim().is_empty() {
                    None
                } else {
                    Some(window.title.clone())
                }
            }),
        )?;
        let backend = AutoAxBackend::default();
        let runner = RealProcessRunner;
        let (evaluation, selected_node) =
            resolve_selector_node_against_backend(&runner, &backend, &ax_target, &selector, 4_000)
                .map_err(|err| {
                    err.with_operation("observe.screenshot").with_hint(
                        "Refine AX selector or validate target window context before retrying.",
                    )
                })?;
        let frame = selected_node.frame.ok_or_else(|| {
            CliError::runtime("selected AX node does not expose frame metadata")
                .with_operation("observe.screenshot")
                .with_hint("Choose an AX element that reports frame bounds.")
        })?;
        let capture_region = padded_region(&frame, args.selector_padding, &window)?;
        targets::capture_screenshot_region(&output_path, &window, image_format, &capture_region)?;
        Some(ScreenshotSelectorResult {
            node_id: selected_node.node_id,
            matched_count: evaluation.matched_count,
            padding: args.selector_padding,
            frame,
            capture_region,
        })
    } else {
        targets::capture_screenshot(&output_path, &window, image_format)?;
        None
    };

    match format {
        OutputFormat::Json => {
            let result = ScreenshotResult {
                path: output_path.display().to_string(),
                target: WindowRow::from(&window),
                selector,
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

fn padded_region(
    frame: &AxFrame,
    padding: i32,
    window: &crate::screen_record_adapter::WindowInfo,
) -> Result<AxFrame, CliError> {
    let padding = padding.max(0) as f64;
    let window_left = window.bounds.x as f64;
    let window_top = window.bounds.y as f64;
    let window_right = window_left + window.bounds.width.max(1) as f64;
    let window_bottom = window_top + window.bounds.height.max(1) as f64;

    let left = (frame.x - padding).max(window_left);
    let top = (frame.y - padding).max(window_top);
    let right = (frame.x + frame.width + padding).min(window_right);
    let bottom = (frame.y + frame.height + padding).min(window_bottom);

    if right <= left || bottom <= top {
        return Err(CliError::runtime(
            "selector frame collapsed after applying padding/window bounds",
        )
        .with_operation("observe.screenshot")
        .with_hint("Reduce --selector-padding or pick a different selector."));
    }

    Ok(AxFrame {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{padded_region, resolve_output_path};
    use crate::cli::ObserveScreenshotArgs;
    use crate::model::AxFrame;
    use crate::screen_record_adapter::WindowInfo;
    use screen_record::types::Rect;

    #[test]
    fn preserve_explicit_output_path() {
        let args = ObserveScreenshotArgs {
            window_id: Some(1),
            active_window: false,
            app: None,
            window_name: None,
            path: Some(PathBuf::from("./out/image.png")),
            image_format: None,
            ax_selector: crate::cli::AxSelectorArgs::default(),
            selector_padding: 0,
        };

        assert_eq!(
            resolve_output_path(&args, 123),
            PathBuf::from("./out/image.png")
        );
    }

    #[test]
    fn padded_region_clamps_to_window_bounds() {
        let frame = AxFrame {
            x: 100.0,
            y: 60.0,
            width: 40.0,
            height: 20.0,
        };
        let window = WindowInfo {
            id: 1,
            owner_name: "Terminal".to_string(),
            title: "Main".to_string(),
            bounds: Rect {
                x: 90,
                y: 50,
                width: 45,
                height: 25,
            },
            on_screen: true,
            active: true,
            owner_pid: 1,
            z_order: 0,
        };

        let region = padded_region(&frame, 20, &window).expect("region");
        assert_eq!(
            region,
            AxFrame {
                x: 90.0,
                y: 50.0,
                width: 45.0,
                height: 25.0,
            }
        );
    }

    #[test]
    fn padded_region_errors_when_result_collapses() {
        let frame = AxFrame {
            x: 10.0,
            y: 10.0,
            width: 0.0,
            height: 0.0,
        };
        let window = WindowInfo {
            id: 2,
            owner_name: "Terminal".to_string(),
            title: "Main".to_string(),
            bounds: Rect {
                x: 100,
                y: 100,
                width: 10,
                height: 10,
            },
            on_screen: true,
            active: true,
            owner_pid: 1,
            z_order: 0,
        };

        let err = padded_region(&frame, 0, &window).expect_err("expected collapsed region");
        assert!(err
            .message()
            .contains("selector frame collapsed after applying padding/window bounds"));
    }
}

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::backend::process::RealProcessRunner;
use crate::backend::AutoAxBackend;
use crate::cli::{ObserveScreenshotArgs, OutputFormat};
use crate::commands::ax_common::{
    build_selector_from_args, build_target, resolve_selector_node_against_backend,
    selector_args_requested,
};
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{
    AxFrame, IfChangedResult, ScreenshotResult, ScreenshotSelectorResult, WindowRow,
};
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

    let (selector, if_changed) = if args.if_changed {
        let (selector, result) = capture_if_changed(args, &window, image_format, &output_path)?;
        (selector, Some(result))
    } else {
        let selector = capture_to_path(args, &window, image_format, &output_path)?;
        (selector, None)
    };

    match format {
        OutputFormat::Json => {
            let result = ScreenshotResult {
                path: output_path.display().to_string(),
                target: WindowRow::from(&window),
                selector,
                if_changed,
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

fn capture_to_path(
    args: &ObserveScreenshotArgs,
    window: &crate::screen_record_adapter::WindowInfo,
    image_format: crate::cli::ImageFormat,
    capture_path: &Path,
) -> Result<Option<ScreenshotSelectorResult>, CliError> {
    if selector_args_requested(&args.ax_selector) {
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
        let capture_region = padded_region(&frame, args.selector_padding, window)?;
        targets::capture_screenshot_region(capture_path, window, image_format, &capture_region)?;
        Ok(Some(ScreenshotSelectorResult {
            node_id: selected_node.node_id,
            matched_count: evaluation.matched_count,
            padding: args.selector_padding,
            frame,
            capture_region,
        }))
    } else {
        targets::capture_screenshot(capture_path, window, image_format)?;
        Ok(None)
    }
}

fn capture_if_changed(
    args: &ObserveScreenshotArgs,
    window: &crate::screen_record_adapter::WindowInfo,
    image_format: crate::cli::ImageFormat,
    output_path: &Path,
) -> Result<(Option<ScreenshotSelectorResult>, IfChangedResult), CliError> {
    let threshold = args.if_changed_threshold.unwrap_or(0);
    let baseline_path = resolve_baseline_path(args, output_path)?;
    let baseline_hash = baseline_path
        .as_ref()
        .map(|path| hash_file_u64(path))
        .transpose()?;

    let staged_path = staged_if_changed_path(output_path)?;
    let selector = match capture_to_path(args, window, image_format, &staged_path) {
        Ok(selector) => selector,
        Err(err) => {
            let _ = std::fs::remove_file(&staged_path);
            return Err(err);
        }
    };
    let current_hash = match hash_file_u64(&staged_path) {
        Ok(value) => value,
        Err(err) => {
            let _ = std::fs::remove_file(&staged_path);
            return Err(err);
        }
    };

    let changed_by_threshold = baseline_hash
        .map(|baseline| hamming_distance(baseline, current_hash) > threshold)
        .unwrap_or(true);
    let changed = changed_by_threshold || !output_path.exists();
    let captured_path = if changed {
        publish_if_changed_capture(&staged_path, output_path)?;
        Some(output_path.display().to_string())
    } else {
        let _ = std::fs::remove_file(&staged_path);
        None
    };

    Ok((
        selector,
        IfChangedResult {
            changed,
            baseline_hash: baseline_hash.map(hash_hex),
            current_hash: hash_hex(current_hash),
            threshold,
            captured_path,
        },
    ))
}

fn resolve_baseline_path(
    args: &ObserveScreenshotArgs,
    output_path: &Path,
) -> Result<Option<PathBuf>, CliError> {
    if let Some(path) = args.if_changed_baseline.as_ref() {
        if !path.exists() {
            return Err(CliError::runtime(format!(
                "--if-changed-baseline path does not exist: {}",
                path.display()
            )));
        }
        return Ok(Some(path.clone()));
    }

    if output_path.exists() {
        return Ok(Some(output_path.to_path_buf()));
    }

    Ok(None)
}

fn staged_if_changed_path(output_path: &Path) -> Result<PathBuf, CliError> {
    let parent = output_path
        .parent()
        .ok_or_else(|| CliError::runtime("missing output parent dir"))?;
    let name = output_path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("screenshot");
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    Ok(parent.join(format!(".{name}.ifchanged-{pid}-{nanos}")))
}

fn publish_if_changed_capture(staged_path: &Path, output_path: &Path) -> Result<(), CliError> {
    if output_path.exists() {
        std::fs::remove_file(output_path).map_err(|err| {
            CliError::runtime(format!("failed to replace output screenshot file: {err}"))
        })?;
    }
    std::fs::rename(staged_path, output_path)
        .map_err(|err| CliError::runtime(format!("failed to write output screenshot: {err}")))
}

fn hash_file_u64(path: &Path) -> Result<u64, CliError> {
    let bytes = std::fs::read(path).map_err(|err| {
        CliError::runtime(format!(
            "failed to read image for --if-changed hash: {} ({err})",
            path.display()
        ))
    })?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}

fn hash_hex(value: u64) -> String {
    format!("{value:016x}")
}

fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
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
            if_changed: false,
            if_changed_baseline: None,
            if_changed_threshold: None,
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

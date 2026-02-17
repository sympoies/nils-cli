use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::backend::process::ProcessRunner;
use crate::backend::{AutoAxBackend, AxBackendAdapter};
use crate::cli::{DebugBundleArgs, ImageFormat, ListAppsArgs, ListWindowsArgs, OutputFormat};
use crate::commands::ax_common::build_target;
use crate::commands::{emit_json_success, reject_tsv_for_list_only};
use crate::error::CliError;
use crate::model::{
    DebugBundleArtifactEntry, DebugBundleResult, ListAppsResult, ListWindowsResult, WindowRow,
};
use crate::run::ActionPolicy;
use crate::targets::{self, TargetSelector};
use crate::test_mode;

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

pub fn run_debug_bundle(
    format: OutputFormat,
    args: &DebugBundleArgs,
    policy: ActionPolicy,
    runner: &dyn ProcessRunner,
) -> Result<(), CliError> {
    let output_dir = resolve_debug_output_dir(args);
    std::fs::create_dir_all(&output_dir).map_err(|err| {
        CliError::runtime(format!(
            "failed to create debug bundle output directory `{}`: {err}",
            output_dir.display()
        ))
        .with_operation("debug.bundle")
    })?;

    let selector = target_selector_from_debug_args(args);
    let mut artifacts = Vec::new();
    let mut resolved_window = None;

    let target_window_path = output_dir.join("01-target-window.json");
    match targets::resolve_window(&selector) {
        Ok(window) => {
            resolved_window = Some(window.clone());
            push_artifact_json(
                &mut artifacts,
                "target-window",
                &target_window_path,
                &WindowRow::from(&window),
            );
        }
        Err(err) => {
            push_artifact_error(&mut artifacts, "target-window", &target_window_path, &err);
        }
    }

    let windows_list_path = output_dir.join("02-windows-list.json");
    match targets::list_windows(&ListWindowsArgs {
        app: None,
        window_name: None,
        on_screen_only: false,
    }) {
        Ok(windows) => {
            push_artifact_json(&mut artifacts, "windows-list", &windows_list_path, &windows);
        }
        Err(err) => {
            push_artifact_error(&mut artifacts, "windows-list", &windows_list_path, &err);
        }
    }

    let screenshot_path = output_dir.join("03-active-window.png");
    match resolved_window.as_ref() {
        Some(window) => {
            match targets::capture_screenshot(&screenshot_path, window, ImageFormat::Png) {
                Ok(()) => {
                    push_artifact_ok(&mut artifacts, "active-window-screenshot", &screenshot_path)
                }
                Err(err) => {
                    push_artifact_error(
                        &mut artifacts,
                        "active-window-screenshot",
                        &screenshot_path,
                        &err,
                    );
                }
            }
        }
        None => push_artifact_error(
            &mut artifacts,
            "active-window-screenshot",
            &screenshot_path,
            &CliError::runtime("target window was not resolved"),
        ),
    }

    let backend = AutoAxBackend::default();
    let ax_app = args.app.clone().or_else(|| {
        resolved_window
            .as_ref()
            .map(|window| window.owner_name.clone())
    });
    let ax_target = build_target(
        None,
        ax_app,
        None,
        if args.app.is_some() {
            args.window_name.clone()
        } else {
            None
        },
    )?;

    capture_ax_role_artifact(
        &mut artifacts,
        &output_dir.join("04-ax-links.json"),
        "ax-links",
        "AXLink",
        &backend,
        runner,
        &ax_target,
        policy.timeout_ms,
    );
    capture_ax_role_artifact(
        &mut artifacts,
        &output_dir.join("05-ax-buttons.json"),
        "ax-buttons",
        "AXButton",
        &backend,
        runner,
        &ax_target,
        policy.timeout_ms,
    );
    capture_ax_role_artifact(
        &mut artifacts,
        &output_dir.join("06-ax-textfields.json"),
        "ax-textfields",
        "AXTextField",
        &backend,
        runner,
        &ax_target,
        policy.timeout_ms,
    );

    let focused_path = output_dir.join("07-ax-focused.json");
    match backend.list(
        runner,
        &crate::model::AxListRequest {
            target: ax_target.clone(),
            focused: Some(true),
            limit: Some(1),
            ..crate::model::AxListRequest::default()
        },
        policy.timeout_ms.max(1),
    ) {
        Ok(result) => push_artifact_json(&mut artifacts, "ax-focused", &focused_path, &result),
        Err(err) => push_artifact_error(&mut artifacts, "ax-focused", &focused_path, &err),
    }

    let artifact_index_path = output_dir.join("artifact-index.json");
    let result = DebugBundleResult {
        output_dir: output_dir.display().to_string(),
        artifact_index_path: artifact_index_path.display().to_string(),
        partial_failure: artifacts.iter().any(|artifact| !artifact.ok),
        artifacts,
    };
    write_json_file(&artifact_index_path, &result).map_err(|err| {
        CliError::runtime(format!(
            "failed to write debug bundle artifact index `{}`: {err}",
            artifact_index_path.display()
        ))
        .with_operation("debug.bundle")
    })?;

    match format {
        OutputFormat::Json => {
            emit_json_success("debug.bundle", result)?;
        }
        OutputFormat::Text => {
            println!(
                "debug.bundle\toutput_dir={}\tartifact_index_path={}\tpartial_failure={}",
                result.output_dir, result.artifact_index_path, result.partial_failure
            );
            for artifact in &result.artifacts {
                println!(
                    "debug.bundle.artifact\tid={}\tok={}\tpath={}\terror={}",
                    artifact.id,
                    artifact.ok,
                    artifact.path,
                    artifact.error.clone().unwrap_or_default()
                );
            }
        }
        OutputFormat::Tsv => {
            return reject_tsv_for_list_only();
        }
    }

    Ok(())
}

fn target_selector_from_debug_args(args: &DebugBundleArgs) -> TargetSelector {
    if args.window_id.is_none() && !args.active_window && args.app.is_none() {
        return TargetSelector {
            window_id: None,
            active_window: true,
            app: None,
            window_name: None,
        };
    }

    TargetSelector {
        window_id: args.window_id,
        active_window: args.active_window,
        app: args.app.clone(),
        window_name: args.window_name.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn capture_ax_role_artifact(
    artifacts: &mut Vec<DebugBundleArtifactEntry>,
    path: &Path,
    id: &str,
    role: &str,
    backend: &AutoAxBackend,
    runner: &dyn ProcessRunner,
    target: &crate::model::AxTarget,
    timeout_ms: u64,
) {
    match backend.list(
        runner,
        &crate::model::AxListRequest {
            target: target.clone(),
            role: Some(role.to_string()),
            ..crate::model::AxListRequest::default()
        },
        timeout_ms.max(1),
    ) {
        Ok(result) => push_artifact_json(artifacts, id, path, &result),
        Err(err) => push_artifact_error(artifacts, id, path, &err),
    }
}

fn resolve_debug_output_dir(args: &DebugBundleArgs) -> PathBuf {
    if let Some(path) = args.output_dir.clone() {
        return path;
    }
    agents_out_dir().join(format!(
        "macos-agent-debug-bundle-{}",
        test_mode::timestamp_token()
    ))
}

fn agents_out_dir() -> PathBuf {
    if let Ok(agent_home) = std::env::var("AGENT_HOME") {
        return PathBuf::from(agent_home).join("out");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".agents").join("out");
    }
    PathBuf::from(".agents").join("out")
}

fn write_json_file<T>(path: &Path, value: &T) -> Result<(), std::io::Error>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(path, body)
}

fn push_artifact_json<T>(
    artifacts: &mut Vec<DebugBundleArtifactEntry>,
    id: &str,
    path: &Path,
    value: &T,
) where
    T: Serialize,
{
    match write_json_file(path, value) {
        Ok(()) => push_artifact_ok(artifacts, id, path),
        Err(err) => push_artifact_error(
            artifacts,
            id,
            path,
            &CliError::runtime(format!("failed to write artifact file: {err}")),
        ),
    }
}

fn push_artifact_ok(artifacts: &mut Vec<DebugBundleArtifactEntry>, id: &str, path: &Path) {
    artifacts.push(DebugBundleArtifactEntry {
        id: id.to_string(),
        path: path.display().to_string(),
        ok: true,
        error: None,
    });
}

fn push_artifact_error(
    artifacts: &mut Vec<DebugBundleArtifactEntry>,
    id: &str,
    path: &Path,
    error: &CliError,
) {
    artifacts.push(DebugBundleArtifactEntry {
        id: id.to_string(),
        path: path.display().to_string(),
        ok: false,
        error: Some(error.message().to_string()),
    });
}

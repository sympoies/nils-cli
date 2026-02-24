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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use nils_test_support::{EnvGuard, GlobalStateLock};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{
        DebugBundleArtifactEntry, agents_out_dir, push_artifact_json, resolve_debug_output_dir,
        run_debug_bundle, target_selector_from_debug_args,
    };
    use crate::backend::process::RealProcessRunner;
    use crate::cli::{DebugBundleArgs, OutputFormat};
    use crate::run::ActionPolicy;

    const AX_LIST_JSON_OK: &str = r#"{"nodes":[{"node_id":"1.1","role":"AXButton","enabled":true,"focused":false,"actions":[],"path":["1","1"]}],"warnings":[]}"#;

    fn policy() -> ActionPolicy {
        ActionPolicy {
            dry_run: false,
            retries: 0,
            retry_delay_ms: 0,
            timeout_ms: 1000,
        }
    }

    fn debug_args() -> DebugBundleArgs {
        DebugBundleArgs {
            window_id: None,
            active_window: false,
            app: None,
            window_name: None,
            output_dir: None,
        }
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).expect("read json")).expect("valid json")
    }

    fn artifact_index(output_dir: &Path) -> Value {
        read_json(&output_dir.join("artifact-index.json"))
    }

    #[test]
    fn target_selector_from_debug_args_defaults_to_active_window_and_preserves_explicit_values() {
        let defaults = target_selector_from_debug_args(&debug_args());
        assert!(defaults.active_window);
        assert_eq!(defaults.window_id, None);
        assert_eq!(defaults.app, None);
        assert_eq!(defaults.window_name, None);

        let explicit = DebugBundleArgs {
            window_id: Some(42),
            active_window: false,
            app: Some("Terminal".to_string()),
            window_name: Some("Docs".to_string()),
            output_dir: None,
        };
        let resolved = target_selector_from_debug_args(&explicit);
        assert_eq!(resolved.window_id, Some(42));
        assert!(!resolved.active_window);
        assert_eq!(resolved.app.as_deref(), Some("Terminal"));
        assert_eq!(resolved.window_name.as_deref(), Some("Docs"));
    }

    #[test]
    fn agents_out_dir_uses_agent_home_then_home_then_dot_agents() {
        let lock = GlobalStateLock::new();
        let temp = TempDir::new().expect("tempdir");

        let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &temp.path().to_string_lossy());
        let _home = EnvGuard::set(&lock, "HOME", "/tmp/should-not-win");
        assert_eq!(agents_out_dir(), temp.path().join("out"));
        drop(_agent_home);

        let _agent_home = EnvGuard::remove(&lock, "AGENT_HOME");
        let home_dir = temp.path().join("home");
        fs::create_dir_all(&home_dir).expect("create home");
        let _home = EnvGuard::set(&lock, "HOME", &home_dir.to_string_lossy());
        assert_eq!(agents_out_dir(), home_dir.join(".agents").join("out"));
        drop(_home);

        let _home = EnvGuard::remove(&lock, "HOME");
        assert_eq!(
            agents_out_dir(),
            std::path::PathBuf::from(".agents").join("out")
        );
    }

    #[test]
    fn resolve_debug_output_dir_uses_explicit_output_dir_when_provided() {
        let temp = TempDir::new().expect("tempdir");
        let explicit = temp.path().join("bundle-out");
        let args = DebugBundleArgs {
            output_dir: Some(explicit.clone()),
            ..debug_args()
        };
        assert_eq!(resolve_debug_output_dir(&args), explicit);
    }

    #[test]
    fn push_artifact_json_records_error_when_parent_is_a_file() {
        let temp = TempDir::new().expect("tempdir");
        let parent_file = temp.path().join("not-a-dir");
        fs::write(&parent_file, "x").expect("write parent file");
        let path = parent_file.join("artifact.json");

        let mut artifacts: Vec<DebugBundleArtifactEntry> = Vec::new();
        push_artifact_json(&mut artifacts, "artifact", &path, &json!({"ok": true}));

        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].id, "artifact");
        assert!(!artifacts[0].ok);
        assert!(
            artifacts[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("failed to write artifact file")
        );
    }

    #[test]
    fn run_debug_bundle_json_writes_complete_bundle_in_test_mode() {
        let lock = GlobalStateLock::new();
        let temp = TempDir::new().expect("tempdir");
        let _test_mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let _timestamp = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_TIMESTAMP", "unit-success");
        let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &temp.path().to_string_lossy());
        let _backend = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_AX_BACKEND", "applescript");
        let _ax_list = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_AX_LIST_JSON", AX_LIST_JSON_OK);

        let runner = RealProcessRunner;
        let args = debug_args();
        run_debug_bundle(OutputFormat::Json, &args, policy(), &runner).expect("debug bundle");

        let output_dir = temp
            .path()
            .join("out")
            .join("macos-agent-debug-bundle-unit-success");
        let index = artifact_index(&output_dir);
        assert_eq!(index["partial_failure"], json!(false));
        assert_eq!(index["artifacts"].as_array().expect("artifacts").len(), 7);
        assert!(
            index["artifacts"]
                .as_array()
                .expect("artifacts")
                .iter()
                .all(|artifact| artifact["ok"] == json!(true))
        );
        assert!(output_dir.join("03-active-window.png").exists());
    }

    #[test]
    fn run_debug_bundle_text_records_partial_failures_and_keeps_writing_index() {
        let lock = GlobalStateLock::new();
        let temp = TempDir::new().expect("tempdir");
        let out_dir = temp.path().join("partial");
        let _test_mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let _agent_home = EnvGuard::set(&lock, "AGENT_HOME", &temp.path().to_string_lossy());
        let _backend = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_AX_BACKEND", "applescript");
        let _ax_list = EnvGuard::set(
            &lock,
            "AGENTS_MACOS_AGENT_AX_LIST_JSON",
            r#"{"nodes":"oops"}"#,
        );

        let runner = RealProcessRunner;
        let args = DebugBundleArgs {
            window_id: Some(999_999),
            active_window: false,
            app: Some("Terminal".to_string()),
            window_name: Some("Docs".to_string()),
            output_dir: Some(out_dir.clone()),
        };
        run_debug_bundle(OutputFormat::Text, &args, policy(), &runner).expect("partial bundle");

        let index = artifact_index(&out_dir);
        assert_eq!(index["partial_failure"], json!(true));
        let artifacts = index["artifacts"].as_array().expect("artifacts");
        assert_eq!(artifacts.len(), 7);
        assert!(
            artifacts
                .iter()
                .any(|a| a["id"] == "target-window" && a["ok"] == json!(false))
        );
        assert!(
            artifacts
                .iter()
                .any(|a| a["id"] == "active-window-screenshot" && a["ok"] == json!(false))
        );
        assert!(
            artifacts
                .iter()
                .any(|a| a["id"] == "ax-focused" && a["ok"] == json!(false))
        );
        assert!(
            artifacts
                .iter()
                .any(|a| a["id"] == "windows-list" && a["ok"] == json!(true))
        );
    }

    #[test]
    fn run_debug_bundle_tsv_is_rejected_for_debug_bundle() {
        let lock = GlobalStateLock::new();
        let temp = TempDir::new().expect("tempdir");
        let _test_mode = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_TEST_MODE", "1");
        let _backend = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_AX_BACKEND", "applescript");
        let _ax_list = EnvGuard::set(&lock, "AGENTS_MACOS_AGENT_AX_LIST_JSON", AX_LIST_JSON_OK);

        let runner = RealProcessRunner;
        let args = DebugBundleArgs {
            output_dir: Some(temp.path().join("tsv")),
            ..debug_args()
        };
        let err = run_debug_bundle(OutputFormat::Tsv, &args, policy(), &runner)
            .expect_err("TSV should be rejected");
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().to_ascii_lowercase().contains("tsv"));
    }

    #[test]
    fn run_debug_bundle_reports_output_dir_creation_error() {
        let temp = TempDir::new().expect("tempdir");
        let file_path = temp.path().join("not-a-directory");
        fs::write(&file_path, "x").expect("write file");

        let runner = RealProcessRunner;
        let args = DebugBundleArgs {
            output_dir: Some(file_path),
            ..debug_args()
        };
        let err = run_debug_bundle(OutputFormat::Json, &args, policy(), &runner)
            .expect_err("existing file path should fail create_dir_all");
        assert_eq!(err.exit_code(), 1);
        assert!(
            err.to_string()
                .contains("failed to create debug bundle output directory")
        );
    }
}

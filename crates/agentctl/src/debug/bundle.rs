use super::schema::{
    BundleArtifact, BundleManifest, BUNDLE_ARTIFACTS_DIR, BUNDLE_MANIFEST_FILE_NAME,
};
use super::sources::{git_context, image_processing, macos_agent, screen_record};
use super::{EXIT_OK, EXIT_RUNTIME_ERROR};
use clap::{Args, ValueEnum};
use serde::Serialize;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

const DEFAULT_OUTPUT_NAMESPACE: &str = "agentctl-debug-bundle";

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Default)]
pub enum BundleOutputFormat {
    Text,
    #[default]
    Json,
}

#[derive(Debug, Args)]
pub struct BundleArgs {
    /// Output directory for the manifest and normalized artifacts
    #[arg(long)]
    pub output_dir: Option<PathBuf>,

    /// Render format for command output
    #[arg(long, value_enum, default_value_t = BundleOutputFormat::Json)]
    pub format: BundleOutputFormat,
}

#[derive(Debug, Serialize)]
struct BundleCommandOutput<'a> {
    manifest_path: String,
    manifest: &'a BundleManifest,
}

#[derive(Debug)]
struct CommandCapture {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    spawn_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct CommandInvocation<'a> {
    program: &'a str,
    args: Vec<&'a str>,
}

#[derive(Debug, Serialize)]
struct CommandArtifactPayload<'a> {
    source: &'a str,
    command: CommandInvocation<'a>,
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    spawn_error: Option<String>,
}

pub fn run(args: BundleArgs) -> i32 {
    let output_dir = resolve_output_dir(args.output_dir.as_deref());
    let manifest = match collect_bundle(&output_dir) {
        Ok(manifest) => manifest,
        Err(error) => {
            eprintln!("agentctl debug bundle: {error}");
            return EXIT_RUNTIME_ERROR;
        }
    };

    let manifest_path = manifest_path_for(&output_dir);
    let output = BundleCommandOutput {
        manifest_path: path_to_string(&manifest_path),
        manifest: &manifest,
    };

    match args.format {
        BundleOutputFormat::Json => emit_json(&output),
        BundleOutputFormat::Text => {
            emit_text(&output);
            EXIT_OK
        }
    }
}

pub fn collect_bundle(output_dir: &Path) -> Result<BundleManifest, String> {
    std::fs::create_dir_all(output_dir).map_err(|error| {
        format!(
            "failed to create output directory `{}`: {error}",
            output_dir.display()
        )
    })?;
    std::fs::create_dir_all(output_dir.join(BUNDLE_ARTIFACTS_DIR)).map_err(|error| {
        format!(
            "failed to create artifact directory `{}`: {error}",
            output_dir.join(BUNDLE_ARTIFACTS_DIR).display()
        )
    })?;

    let artifacts = vec![
        git_context::collect(output_dir),
        macos_agent::collect(output_dir),
        screen_record::collect(output_dir),
        image_processing::collect(output_dir),
    ];
    let manifest = BundleManifest::from_artifacts(path_to_string(output_dir), artifacts);
    let manifest_path = manifest_path_for(output_dir);
    write_json_file(&manifest_path, &manifest).map_err(|error| {
        format!(
            "failed to write manifest `{}`: {error}",
            manifest_path.display()
        )
    })?;

    Ok(manifest)
}

pub fn resolve_output_dir(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }

    codex_out_dir().join(DEFAULT_OUTPUT_NAMESPACE)
}

pub(crate) fn collect_command_artifact(
    output_dir: &Path,
    id: &'static str,
    relative_path: &'static str,
    program: &'static str,
    args: &[&'static str],
) -> BundleArtifact {
    let capture = run_command(program, args);
    let payload = CommandArtifactPayload {
        source: id,
        command: CommandInvocation {
            program,
            args: args.to_vec(),
        },
        success: capture.success,
        exit_code: capture.exit_code,
        stdout: capture.stdout.clone(),
        stderr: capture.stderr.clone(),
        spawn_error: capture.spawn_error.clone(),
    };

    let normalized_relative_path = normalize_relative_path(relative_path);
    let mut issues = Vec::new();
    if let Err(error) = write_json_artifact(output_dir, &normalized_relative_path, &payload) {
        issues.push(format!("failed to persist command artifact: {error}"));
    }

    if !capture.success {
        issues.push(command_failure_message(program, &capture));
    }

    if issues.is_empty() {
        BundleArtifact::collected(id, normalized_relative_path)
    } else {
        BundleArtifact::failed(id, normalized_relative_path, issues.join("; "))
    }
}

pub(crate) fn write_json_artifact<T: Serialize>(
    output_dir: &Path,
    relative_path: &str,
    payload: &T,
) -> Result<(), String> {
    let path = output_dir.join(relative_path);
    write_json_file(&path, payload).map_err(|error| format!("{}: {error}", path.display()))
}

pub(crate) fn normalize_relative_path(path: &str) -> String {
    path.trim_start_matches("./").replace('\\', "/")
}

fn emit_json(output: &BundleCommandOutput<'_>) -> i32 {
    match serde_json::to_string_pretty(output) {
        Ok(encoded) => {
            println!("{encoded}");
            EXIT_OK
        }
        Err(error) => {
            eprintln!("agentctl debug bundle: failed to render json output: {error}");
            EXIT_RUNTIME_ERROR
        }
    }
}

fn emit_text(output: &BundleCommandOutput<'_>) {
    println!("manifest_path: {}", output.manifest_path);
    println!("schema_version: {}", output.manifest.schema_version);
    println!("manifest_version: {}", output.manifest.manifest_version);
    println!("output_dir: {}", output.manifest.output_dir);
    println!("partial_failure: {}", output.manifest.partial_failure);
    println!(
        "summary: total={} collected={} failed={}",
        output.manifest.summary.total_artifacts,
        output.manifest.summary.collected,
        output.manifest.summary.failed
    );
    println!("artifacts:");
    for artifact in &output.manifest.artifacts {
        let line = json!({
            "id": artifact.id,
            "path": artifact.path,
            "status": artifact.status,
            "error": artifact.error,
        });
        println!("- {}", line);
    }
}

fn run_command(program: &str, args: &[&str]) -> CommandCapture {
    match Command::new(program).args(args).output() {
        Ok(output) => CommandCapture {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            spawn_error: None,
        },
        Err(error) => CommandCapture {
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            spawn_error: Some(error.to_string()),
        },
    }
}

fn command_failure_message(program: &str, capture: &CommandCapture) -> String {
    if let Some(spawn_error) = capture.spawn_error.as_deref() {
        return format!("failed to launch `{program}`: {spawn_error}");
    }

    let mut message = match capture.exit_code {
        Some(code) => format!("`{program}` exited with status code {code}"),
        None => format!("`{program}` terminated without an exit code"),
    };

    let stderr_excerpt = capture.stderr.trim();
    if !stderr_excerpt.is_empty() {
        message.push_str(": ");
        message.push_str(stderr_excerpt.replace('\n', " ").as_str());
    }

    message
}

fn write_json_file<T: Serialize>(path: &Path, payload: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create parent dir `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let body = serde_json::to_vec_pretty(payload)
        .map_err(|error| format!("failed to serialize json payload: {error}"))?;
    std::fs::write(path, body).map_err(|error| format!("failed to write file: {error}"))
}

fn manifest_path_for(output_dir: &Path) -> PathBuf {
    output_dir.join(BUNDLE_MANIFEST_FILE_NAME)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn codex_out_dir() -> PathBuf {
    if let Ok(codex_home) = std::env::var("CODEX_HOME") {
        return PathBuf::from(codex_home).join("out");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".codex").join("out");
    }
    PathBuf::from(".codex").join("out")
}

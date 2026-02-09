use super::super::bundle::{normalize_relative_path, write_json_artifact};
use super::super::schema::BundleArtifact;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

pub const ARTIFACT_ID: &str = "git-context";
pub const ARTIFACT_RELATIVE_PATH: &str = "artifacts/10-git-context.json";

#[derive(Debug, Serialize)]
struct GitContextArtifact {
    source: &'static str,
    cwd: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    head: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status_porcelain_v1: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    errors: Vec<String>,
}

pub fn collect(output_dir: &Path) -> BundleArtifact {
    let normalized_relative_path = normalize_relative_path(ARTIFACT_RELATIVE_PATH);
    let mut errors = Vec::new();

    let cwd = std::env::current_dir()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|error| {
            let message = format!("failed to resolve current directory: {error}");
            errors.push(message);
            ".".to_string()
        });

    let repo_root = run_git(&["rev-parse", "--show-toplevel"]).ok();
    if repo_root.is_none() {
        errors.push("failed to resolve git repository root".to_string());
    }

    let branch = run_git(&["rev-parse", "--abbrev-ref", "HEAD"]).ok();
    if branch.is_none() {
        errors.push("failed to resolve git branch".to_string());
    }

    let head = run_git(&["rev-parse", "HEAD"]).ok();
    if head.is_none() {
        errors.push("failed to resolve git HEAD".to_string());
    }

    let status_porcelain_v1 = run_git(&["status", "--porcelain=v1", "--branch"]).ok();
    if status_porcelain_v1.is_none() {
        errors.push("failed to collect git status".to_string());
    }

    let payload = GitContextArtifact {
        source: ARTIFACT_ID,
        cwd,
        repo_root,
        branch,
        head,
        status_porcelain_v1,
        errors: errors.clone(),
    };

    let mut failures = Vec::new();
    if let Err(error) = write_json_artifact(output_dir, &normalized_relative_path, &payload) {
        failures.push(format!("failed to persist git context artifact: {error}"));
    }

    failures.extend(errors);

    if failures.is_empty() {
        BundleArtifact::collected(ARTIFACT_ID, normalized_relative_path)
    } else {
        BundleArtifact::failed(ARTIFACT_ID, normalized_relative_path, failures.join("; "))
    }
}

fn run_git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("failed to launch git {:?}: {error}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            format!("git {:?} exited with non-zero status", args)
        } else {
            format!("git {:?} failed: {detail}", args)
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use directories::BaseDirs;

use crate::paths::normalize_root_path;

#[derive(Debug, Clone, Default)]
pub struct PathOverrides {
    pub codex_home: Option<PathBuf>,
    pub project_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ResolvedRoots {
    pub codex_home: PathBuf,
    pub project_path: PathBuf,
}

pub fn resolve_roots(overrides: &PathOverrides) -> Result<ResolvedRoots> {
    let cwd = env::current_dir().context("failed to read current directory")?;
    let codex_home = resolve_codex_home(overrides.codex_home.as_deref(), &cwd);
    let project_path = resolve_project_path(overrides.project_path.as_deref(), &cwd);

    Ok(ResolvedRoots {
        codex_home,
        project_path,
    })
}

fn resolve_codex_home(cli_value: Option<&Path>, cwd: &Path) -> PathBuf {
    if let Some(path) = cli_value {
        return normalize_root_path(path, cwd);
    }

    if let Some(path) = read_env_path("CODEX_HOME") {
        return normalize_root_path(&path, cwd);
    }

    if let Some(base_dirs) = BaseDirs::new() {
        let default = base_dirs.home_dir().join(".codex");
        return normalize_root_path(&default, cwd);
    }

    normalize_root_path(&cwd.join(".codex"), cwd)
}

fn resolve_project_path(cli_value: Option<&Path>, cwd: &Path) -> PathBuf {
    if let Some(path) = cli_value {
        return normalize_root_path(path, cwd);
    }

    if let Some(path) = read_env_path("PROJECT_PATH") {
        return normalize_root_path(&path, cwd);
    }

    if let Some(path) = git_top_level(cwd) {
        return normalize_root_path(&path, cwd);
    }

    normalize_root_path(cwd, cwd)
}

fn read_env_path(name: &str) -> Option<PathBuf> {
    let raw = env::var_os(name)?;
    if raw.is_empty() {
        None
    } else {
        Some(PathBuf::from(raw))
    }
}

fn git_top_level(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

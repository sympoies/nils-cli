use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

fn to_env_key(s: &str) -> String {
    let s = s.trim().to_ascii_uppercase();
    let mut out = String::new();
    let mut prev_us = false;
    for c in s.chars() {
        let ok = c.is_ascii_alphanumeric();
        if ok {
            out.push(c);
            prev_us = false;
            continue;
        }
        if !out.is_empty() && !prev_us {
            out.push('_');
            prev_us = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn list_available_suffixes(file: &Path, prefix: &str) -> Vec<String> {
    if !file.is_file() {
        return Vec::new();
    }

    let Ok(content) = std::fs::read_to_string(file) else {
        return Vec::new();
    };

    let mut out: Vec<String> = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end_matches('\r');
        let mut line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("export") {
            if rest.starts_with(char::is_whitespace) {
                line = rest.trim();
            }
        }

        let Some((lhs, _rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let Some(suffix) = key.strip_prefix(prefix) else {
            continue;
        };
        if suffix.is_empty()
            || !suffix
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            continue;
        }
        out.push(suffix.to_ascii_lowercase());
    }

    out.sort();
    out.dedup();
    out
}

pub fn find_repo_root(start_dir: &Path) -> Result<PathBuf> {
    let mut dir = start_dir;
    loop {
        if dir.join(".git").exists() {
            return Ok(dir.to_path_buf());
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => anyhow::bail!("Must run inside a git work tree"),
        }
    }
}

pub fn resolve_path_from_repo_root(repo_root: &Path, raw: &str) -> PathBuf {
    let raw = raw.trim();
    if raw.starts_with('/') {
        PathBuf::from(raw)
    } else {
        repo_root.join(raw)
    }
}

pub fn resolve_rest_base_url_for_env(setup_dir: &Path, env_value: &str) -> Result<String> {
    let env_value = env_value.trim();
    if env_value.starts_with("http://") || env_value.starts_with("https://") {
        return Ok(env_value.to_string());
    }

    let endpoints_env = setup_dir.join("endpoints.env");
    let endpoints_local = setup_dir.join("endpoints.local.env");
    let endpoints_files: Vec<&Path> = if endpoints_env.is_file() {
        vec![&endpoints_env, &endpoints_local]
    } else {
        Vec::new()
    };
    if endpoints_files.is_empty() {
        anyhow::bail!("endpoints.env not found (expected under setup/rest/)");
    }

    let env_key = to_env_key(env_value);
    let key = format!("REST_URL_{env_key}");
    let found = crate::env_file::read_var_last_wins(&key, &endpoints_files)?;
    let Some(found) = found else {
        let mut available = list_available_suffixes(&endpoints_env, "REST_URL_");
        if endpoints_local.is_file() {
            available.extend(list_available_suffixes(&endpoints_local, "REST_URL_"));
            available.sort();
            available.dedup();
        }
        let available = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(" ")
        };
        anyhow::bail!("Unknown env '{env_value}' (available: {available})");
    };

    Ok(found)
}

pub fn resolve_gql_url_for_env(setup_dir: &Path, env_value: &str) -> Result<String> {
    let env_value = env_value.trim();
    if env_value.starts_with("http://") || env_value.starts_with("https://") {
        return Ok(env_value.to_string());
    }

    let endpoints_env = setup_dir.join("endpoints.env");
    let endpoints_local = setup_dir.join("endpoints.local.env");
    let endpoints_files: Vec<&Path> = if endpoints_env.is_file() {
        vec![&endpoints_env, &endpoints_local]
    } else {
        Vec::new()
    };
    if endpoints_files.is_empty() {
        anyhow::bail!("endpoints.env not found (expected under setup/graphql/)");
    }

    let env_key = to_env_key(env_value);
    let key = format!("GQL_URL_{env_key}");
    let found = crate::env_file::read_var_last_wins(&key, &endpoints_files)?;
    let Some(found) = found else {
        let mut available = list_available_suffixes(&endpoints_env, "GQL_URL_");
        if endpoints_local.is_file() {
            available.extend(list_available_suffixes(&endpoints_local, "GQL_URL_"));
            available.sort();
            available.dedup();
        }
        let available = if available.is_empty() {
            "none".to_string()
        } else {
            available.join(" ")
        };
        anyhow::bail!("Unknown env '{env_value}' (available: {available})");
    };

    Ok(found)
}

#[derive(Debug, Clone)]
pub struct SuiteSelection {
    pub suite_key: String,
    pub suite_path: PathBuf,
}

fn trim_non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn normalize_suite_key(raw: &str) -> String {
    let key = raw.trim();
    let key = key.strip_suffix(".suite.json").unwrap_or(key);
    let key = key.strip_suffix(".json").unwrap_or(key);
    key.trim().to_string()
}

pub fn resolve_suite_selection(
    repo_root: &Path,
    suite: Option<&str>,
    suite_file: Option<&str>,
) -> Result<SuiteSelection> {
    if suite.is_some() && suite_file.is_some() {
        anyhow::bail!("Use only one of --suite or --suite-file");
    }
    let suite = suite.and_then(trim_non_empty);
    let suite_file = suite_file.and_then(trim_non_empty);
    if suite.is_none() && suite_file.is_none() {
        anyhow::bail!("Missing suite (use --suite or --suite-file)");
    }

    if let Some(suite_file) = suite_file {
        let suite_path = resolve_path_from_repo_root(repo_root, &suite_file);
        let suite_path = std::fs::canonicalize(&suite_path).unwrap_or(suite_path);
        if !suite_path.is_file() {
            anyhow::bail!("Suite file not found: {}", suite_path.to_string_lossy());
        }
        let suite_key = suite_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("suite")
            .to_string();
        return Ok(SuiteSelection {
            suite_key,
            suite_path,
        });
    }

    let suite_key = normalize_suite_key(&suite.unwrap_or_default());
    if suite_key.is_empty() {
        anyhow::bail!("Missing suite (use --suite or --suite-file)");
    }

    let suites_dir_override = std::env::var("API_TEST_SUITES_DIR")
        .ok()
        .and_then(|s| trim_non_empty(&s));

    let candidate = if let Some(dir) = suites_dir_override {
        let abs = resolve_path_from_repo_root(repo_root, &dir);
        abs.join(format!("{suite_key}.suite.json"))
    } else {
        let mut p = repo_root
            .join("tests/api/suites")
            .join(format!("{suite_key}.suite.json"));
        if !p.is_file() {
            p = repo_root
                .join("setup/api/suites")
                .join(format!("{suite_key}.suite.json"));
        }
        p
    };

    let suite_path = std::fs::canonicalize(&candidate).unwrap_or(candidate);
    if !suite_path.is_file() {
        anyhow::bail!("Suite file not found: {}", suite_path.to_string_lossy());
    }

    Ok(SuiteSelection {
        suite_key,
        suite_path,
    })
}

pub fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    let Some(parent) = path.parent() else {
        anyhow::bail!("invalid path: {}", path.display());
    };
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create directory: {}", parent.display()))?;
    std::fs::write(path, contents).with_context(|| format!("write file: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[test]
    fn suite_resolve_resolves_suite_name_under_tests_dir() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("tests/api/suites")).unwrap();
        std::fs::write(
            root.join("tests/api/suites/smoke.suite.json"),
            br#"{"version":1,"cases":[]}"#,
        )
        .unwrap();

        let sel = resolve_suite_selection(root, Some("smoke"), None).unwrap();
        assert!(sel
            .suite_path
            .ends_with("tests/api/suites/smoke.suite.json"));
    }
}

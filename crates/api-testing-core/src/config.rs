use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

fn abs_dir(base_dir: &Path, path: &Path) -> Result<PathBuf> {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    };

    std::fs::canonicalize(&joined)
        .with_context(|| format!("failed to resolve directory path: {}", joined.display()))
}

fn find_upwards_for_file(start_dir: &Path, filename: &str) -> Option<PathBuf> {
    let mut dir = start_dir;
    loop {
        if dir.join(filename).is_file() {
            return Some(dir.to_path_buf());
        }

        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

fn find_upwards_for_setup_subdir(start_dir: &Path, rel_subdir: &str) -> Option<PathBuf> {
    let mut dir = start_dir;
    loop {
        let candidate = dir.join(rel_subdir);
        if candidate.is_dir() {
            return Some(candidate);
        }

        match dir.parent() {
            Some(parent) if parent != dir => dir = parent,
            _ => return None,
        }
    }
}

pub fn resolve_rest_setup_dir_for_call(
    cwd: &Path,
    invocation_dir: &Path,
    request_file: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = match config_dir {
        Some(dir) => dir.to_path_buf(),
        None => request_file
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let seed_abs =
        abs_dir(cwd, &seed).context("Failed to resolve setup dir (try --config-dir).")?;
    let config_dir_explicit = config_dir.is_some();

    let found = find_upwards_for_file(&seed_abs, "endpoints.env")
        .or_else(|| find_upwards_for_file(&seed_abs, "tokens.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "endpoints.local.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "tokens.local.env"));
    if let Some(dir) = found {
        return Ok(dir);
    }

    if let Some(found_setup) = find_upwards_for_setup_subdir(&seed_abs, "setup/rest") {
        return Ok(found_setup);
    }

    if config_dir_explicit {
        return Ok(seed_abs);
    }

    let invocation_abs = abs_dir(cwd, invocation_dir)
        .context("Failed to resolve invocation dir for setup discovery")?;
    if let Some(found_invocation) = find_upwards_for_setup_subdir(&invocation_abs, "setup/rest") {
        return Ok(found_invocation);
    }

    Ok(seed_abs)
}

pub fn resolve_rest_setup_dir_for_history(
    cwd: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));
    let seed_abs = abs_dir(cwd, seed).context("Failed to resolve setup dir (try --config-dir).")?;
    let config_dir_explicit = config_dir.is_some();

    let found = find_upwards_for_file(&seed_abs, ".rest_history")
        .or_else(|| find_upwards_for_file(&seed_abs, "endpoints.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "tokens.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "tokens.local.env"));
    if let Some(dir) = found {
        return Ok(dir);
    }

    if let Some(found_setup) = find_upwards_for_setup_subdir(&seed_abs, "setup/rest") {
        return Ok(found_setup);
    }

    if config_dir_explicit {
        return Ok(seed_abs);
    }

    Ok(seed_abs)
}

pub fn resolve_gql_setup_dir_for_call(
    cwd: &Path,
    invocation_dir: &Path,
    operation_file: Option<&Path>,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = match config_dir {
        Some(dir) => dir.to_path_buf(),
        None => operation_file
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".")),
    };

    let seed_abs =
        abs_dir(cwd, &seed).context("Failed to resolve setup dir (try --config-dir).")?;
    let config_dir_explicit = config_dir.is_some();

    let found = find_upwards_for_file(&seed_abs, "endpoints.env")
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.local.env"));
    if let Some(dir) = found {
        return Ok(dir);
    }

    if config_dir_explicit {
        return Ok(seed_abs);
    }

    let invocation_abs = abs_dir(cwd, invocation_dir)
        .context("Failed to resolve invocation dir for setup discovery")?;
    let fallback = invocation_abs.join("setup/graphql");
    if fallback.is_dir() {
        return abs_dir(cwd, &fallback).context("failed to resolve setup/graphql");
    }

    Ok(seed_abs)
}

pub fn resolve_gql_setup_dir_for_history(
    cwd: &Path,
    invocation_dir: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));
    let seed_abs = abs_dir(cwd, seed).context("Failed to resolve setup dir (try --config-dir).")?;
    let config_dir_explicit = config_dir.is_some();

    let found = find_upwards_for_file(&seed_abs, ".gql_history")
        .or_else(|| find_upwards_for_file(&seed_abs, "endpoints.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.local.env"));
    if let Some(dir) = found {
        return Ok(dir);
    }

    if config_dir_explicit {
        return Ok(seed_abs);
    }

    let invocation_abs = abs_dir(cwd, invocation_dir)
        .context("Failed to resolve invocation dir for setup discovery")?;
    let fallback = invocation_abs.join("setup/graphql");
    if fallback.is_dir() {
        return abs_dir(cwd, &fallback).context("failed to resolve setup/graphql");
    }

    Ok(seed_abs)
}

pub fn resolve_gql_setup_dir_for_schema(
    cwd: &Path,
    invocation_dir: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));
    let seed_abs = abs_dir(cwd, seed).context("Failed to resolve setup dir (try --config-dir).")?;
    let config_dir_explicit = config_dir.is_some();

    let found = find_upwards_for_file(&seed_abs, "schema.env")
        .or_else(|| find_upwards_for_file(&seed_abs, "schema.local.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "endpoints.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.env"))
        .or_else(|| find_upwards_for_file(&seed_abs, "jwts.local.env"));
    if let Some(dir) = found {
        return Ok(dir);
    }

    if config_dir_explicit {
        return Ok(seed_abs);
    }

    let invocation_abs = abs_dir(cwd, invocation_dir)
        .context("Failed to resolve invocation dir for setup discovery")?;
    let fallback = invocation_abs.join("setup/graphql");
    if fallback.is_dir() {
        return abs_dir(cwd, &fallback).context("failed to resolve setup/graphql");
    }

    Ok(seed_abs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use tempfile::TempDir;

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn config_rest_call_falls_back_to_upwards_setup_rest() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");

        write_file(
            &root.join("setup/rest/endpoints.env"),
            "export REST_URL_LOCAL=http://x\n",
        );
        write_file(
            &root.join("tests/requests/health.request.json"),
            r#"{"method":"GET","path":"/health"}"#,
        );

        let setup_dir = resolve_rest_setup_dir_for_call(
            &root,
            &root,
            &root.join("tests/requests/health.request.json"),
            None,
        )
        .expect("resolve");

        assert_eq!(setup_dir, root.join("setup/rest"));
    }

    #[test]
    fn config_rest_call_explicit_config_dir_wins() {
        let tmp_root = TempDir::new().expect("tmp root");
        let root = std::fs::canonicalize(tmp_root.path()).expect("root abs");

        let tmp_cfg = TempDir::new().expect("tmp cfg");
        let cfg_root = std::fs::canonicalize(tmp_cfg.path()).expect("cfg abs");
        std::fs::create_dir_all(cfg_root.join("custom/rest")).expect("mkdir");

        write_file(
            &cfg_root.join("req/health.request.json"),
            r#"{"method":"GET","path":"/health"}"#,
        );

        let setup_dir = resolve_rest_setup_dir_for_call(
            &root,
            &root,
            &cfg_root.join("req/health.request.json"),
            Some(&cfg_root.join("custom/rest")),
        )
        .expect("resolve");

        assert_eq!(setup_dir, cfg_root.join("custom/rest"));
    }

    #[test]
    fn config_rest_call_falls_back_to_invocation_dir() {
        let tmp_root = TempDir::new().expect("tmp root");
        let root = std::fs::canonicalize(tmp_root.path()).expect("root abs");

        std::fs::create_dir_all(root.join("setup/rest")).expect("mkdir");

        let tmp_other = TempDir::new().expect("tmp other");
        let other = std::fs::canonicalize(tmp_other.path()).expect("other abs");
        write_file(
            &other.join("place/health.request.json"),
            r#"{"method":"GET","path":"/health"}"#,
        );

        let setup_dir = resolve_rest_setup_dir_for_call(
            &root,
            &root,
            &other.join("place/health.request.json"),
            None,
        )
        .expect("resolve");

        assert_eq!(setup_dir, root.join("setup/rest"));
    }

    #[test]
    fn config_gql_call_falls_back_to_setup_graphql_in_invocation_dir() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");

        write_file(
            &root.join("setup/graphql/endpoints.env"),
            "export GQL_URL_LOCAL=http://x\n",
        );
        write_file(
            &root.join("operations/countries.graphql"),
            "query { __typename }\n",
        );

        let setup_dir = resolve_gql_setup_dir_for_call(
            &root,
            &root,
            Some(&root.join("operations/countries.graphql")),
            None,
        )
        .expect("resolve");

        assert_eq!(setup_dir, root.join("setup/graphql"));
    }

    #[test]
    fn config_gql_schema_discovers_schema_env_upwards() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");

        write_file(
            &root.join("setup/graphql/schema.env"),
            "export GQL_SCHEMA_FILE=schema.gql\n",
        );
        std::fs::create_dir_all(root.join("setup/graphql/ops")).expect("mkdir");

        let setup_dir =
            resolve_gql_setup_dir_for_schema(&root, &root, Some(&root.join("setup/graphql/ops")))
                .expect("resolve");

        assert_eq!(setup_dir, root.join("setup/graphql"));
    }
}

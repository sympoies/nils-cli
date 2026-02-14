use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

const SETUP_DIR_ERROR: &str = "Failed to resolve setup dir (try --config-dir).";
const INVOCATION_DIR_ERROR: &str = "Failed to resolve invocation dir for setup discovery";
const SETUP_GRAPHQL_ERROR: &str = "failed to resolve setup/graphql";

#[derive(Debug, Clone, Copy)]
enum FallbackMode {
    None,
    Upwards(&'static str),
    Direct(&'static str, &'static str),
}

#[derive(Debug)]
struct SetupDiscovery<'a> {
    cwd: &'a Path,
    seed: PathBuf,
    config_dir_explicit: bool,
    files: &'static [&'static str],
    seed_fallback: FallbackMode,
    invocation_dir: Option<&'a Path>,
    invocation_fallback: FallbackMode,
}

impl<'a> SetupDiscovery<'a> {
    fn resolve(&self) -> Result<PathBuf> {
        let seed_abs = abs_dir(self.cwd, &self.seed).context(SETUP_DIR_ERROR)?;

        if let Some(dir) = find_upwards_for_files(&seed_abs, self.files) {
            return Ok(dir);
        }

        if let FallbackMode::Upwards(subdir) = self.seed_fallback
            && let Some(found_setup) = find_upwards_for_setup_subdir(&seed_abs, subdir)
        {
            return Ok(found_setup);
        }

        if self.config_dir_explicit {
            return Ok(seed_abs);
        }

        match self.invocation_fallback {
            FallbackMode::None => {}
            FallbackMode::Upwards(subdir) => {
                let invocation_dir = self
                    .invocation_dir
                    .expect("invocation_dir required for upwards fallback");
                let invocation_abs =
                    abs_dir(self.cwd, invocation_dir).context(INVOCATION_DIR_ERROR)?;
                if let Some(found_setup) = find_upwards_for_setup_subdir(&invocation_abs, subdir) {
                    return Ok(found_setup);
                }
            }
            FallbackMode::Direct(subdir, ctx) => {
                let invocation_dir = self
                    .invocation_dir
                    .expect("invocation_dir required for direct fallback");
                let invocation_abs =
                    abs_dir(self.cwd, invocation_dir).context(INVOCATION_DIR_ERROR)?;
                let fallback = invocation_abs.join(subdir);
                if fallback.is_dir() {
                    return abs_dir(self.cwd, &fallback).context(ctx);
                }
            }
        }

        Ok(seed_abs)
    }
}

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

fn find_upwards_for_files(start_dir: &Path, filenames: &[&str]) -> Option<PathBuf> {
    for filename in filenames {
        if let Some(found) = find_upwards_for_file(start_dir, filename) {
            return Some(found);
        }
    }
    None
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

    SetupDiscovery {
        cwd,
        seed,
        config_dir_explicit: config_dir.is_some(),
        files: &[
            "endpoints.env",
            "tokens.env",
            "endpoints.local.env",
            "tokens.local.env",
        ],
        seed_fallback: FallbackMode::Upwards("setup/rest"),
        invocation_dir: Some(invocation_dir),
        invocation_fallback: FallbackMode::Upwards("setup/rest"),
    }
    .resolve()
}

pub fn resolve_rest_setup_dir_for_history(
    cwd: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));

    SetupDiscovery {
        cwd,
        seed: seed.to_path_buf(),
        config_dir_explicit: config_dir.is_some(),
        files: &[
            ".rest_history",
            "endpoints.env",
            "tokens.env",
            "tokens.local.env",
        ],
        seed_fallback: FallbackMode::Upwards("setup/rest"),
        invocation_dir: None,
        invocation_fallback: FallbackMode::None,
    }
    .resolve()
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

    SetupDiscovery {
        cwd,
        seed,
        config_dir_explicit: config_dir.is_some(),
        files: &["endpoints.env", "jwts.env", "jwts.local.env"],
        seed_fallback: FallbackMode::None,
        invocation_dir: Some(invocation_dir),
        invocation_fallback: FallbackMode::Direct("setup/graphql", SETUP_GRAPHQL_ERROR),
    }
    .resolve()
}

pub fn resolve_gql_setup_dir_for_history(
    cwd: &Path,
    invocation_dir: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));

    SetupDiscovery {
        cwd,
        seed: seed.to_path_buf(),
        config_dir_explicit: config_dir.is_some(),
        files: &[
            ".gql_history",
            "endpoints.env",
            "jwts.env",
            "jwts.local.env",
        ],
        seed_fallback: FallbackMode::None,
        invocation_dir: Some(invocation_dir),
        invocation_fallback: FallbackMode::Direct("setup/graphql", SETUP_GRAPHQL_ERROR),
    }
    .resolve()
}

pub fn resolve_gql_setup_dir_for_schema(
    cwd: &Path,
    invocation_dir: &Path,
    config_dir: Option<&Path>,
) -> Result<PathBuf> {
    let seed = config_dir.unwrap_or_else(|| Path::new("."));

    SetupDiscovery {
        cwd,
        seed: seed.to_path_buf(),
        config_dir_explicit: config_dir.is_some(),
        files: &[
            "schema.env",
            "schema.local.env",
            "endpoints.env",
            "jwts.env",
            "jwts.local.env",
        ],
        seed_fallback: FallbackMode::None,
        invocation_dir: Some(invocation_dir),
        invocation_fallback: FallbackMode::Direct("setup/graphql", SETUP_GRAPHQL_ERROR),
    }
    .resolve()
}

#[derive(Debug, Clone)]
pub struct ResolvedSetup {
    pub setup_dir: PathBuf,
    pub history_file: PathBuf,
    pub endpoints_env: PathBuf,
    pub endpoints_local_env: PathBuf,
    pub tokens_env: Option<PathBuf>,
    pub tokens_local_env: Option<PathBuf>,
    pub jwts_env: Option<PathBuf>,
    pub jwts_local_env: Option<PathBuf>,
}

impl ResolvedSetup {
    pub fn rest(setup_dir: PathBuf, history_override: Option<&Path>) -> Self {
        let history_file =
            crate::history::resolve_history_file(&setup_dir, history_override, ".rest_history");
        let endpoints_env = setup_dir.join("endpoints.env");
        let endpoints_local_env = setup_dir.join("endpoints.local.env");
        let tokens_env = setup_dir.join("tokens.env");
        let tokens_local_env = setup_dir.join("tokens.local.env");
        Self {
            setup_dir,
            history_file,
            endpoints_env,
            endpoints_local_env,
            tokens_env: Some(tokens_env),
            tokens_local_env: Some(tokens_local_env),
            jwts_env: None,
            jwts_local_env: None,
        }
    }

    pub fn graphql(setup_dir: PathBuf, history_override: Option<&Path>) -> Self {
        let history_file =
            crate::history::resolve_history_file(&setup_dir, history_override, ".gql_history");
        let endpoints_env = setup_dir.join("endpoints.env");
        let endpoints_local_env = setup_dir.join("endpoints.local.env");
        let jwts_env = setup_dir.join("jwts.env");
        let jwts_local_env = setup_dir.join("jwts.local.env");
        Self {
            setup_dir,
            history_file,
            endpoints_env,
            endpoints_local_env,
            tokens_env: None,
            tokens_local_env: None,
            jwts_env: Some(jwts_env),
            jwts_local_env: Some(jwts_local_env),
        }
    }

    pub fn endpoints_files(&self) -> Vec<&Path> {
        if self.endpoints_env.is_file() || self.endpoints_local_env.is_file() {
            vec![&self.endpoints_env, &self.endpoints_local_env]
        } else {
            Vec::new()
        }
    }

    pub fn tokens_files(&self) -> Vec<&Path> {
        let mut files: Vec<&Path> = Vec::new();
        if let Some(tokens_env) = self.tokens_env.as_deref() {
            files.push(tokens_env);
        }
        if let Some(tokens_local) = self.tokens_local_env.as_deref() {
            files.push(tokens_local);
        }

        if files.iter().any(|path| path.is_file()) {
            files
        } else {
            Vec::new()
        }
    }

    pub fn jwts_files(&self) -> Vec<&Path> {
        let mut files: Vec<&Path> = Vec::new();
        if let Some(jwts_env) = self.jwts_env.as_deref() {
            files.push(jwts_env);
        }
        if let Some(jwts_local) = self.jwts_local_env.as_deref() {
            files.push(jwts_local);
        }

        if files.iter().any(|path| path.is_file()) {
            files
        } else {
            Vec::new()
        }
    }
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
    fn endpoints_files_includes_local_when_env_missing() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");
        let setup = root.join("setup/rest");

        write_file(
            &setup.join("endpoints.local.env"),
            "export REST_URL_LOCAL=http://localhost:1234\n",
        );

        let resolved = ResolvedSetup::rest(setup, None);
        let files = resolved.endpoints_files();

        assert_eq!(
            files,
            vec![
                resolved.endpoints_env.as_path(),
                resolved.endpoints_local_env.as_path()
            ]
        );
    }

    #[test]
    fn tokens_files_handles_missing_local_path_without_panicking() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");
        let setup = root.join("setup/rest");

        let mut resolved = ResolvedSetup::rest(setup, None);
        let tokens_env = resolved.tokens_env.as_ref().expect("tokens env").clone();
        write_file(&tokens_env, "REST_TOKEN_DEFAULT=abc\n");
        resolved.tokens_local_env = None;

        let files: Vec<PathBuf> = resolved
            .tokens_files()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();

        assert_eq!(files, vec![tokens_env]);
    }

    #[test]
    fn jwts_files_handles_missing_local_path_without_panicking() {
        let tmp = TempDir::new().expect("tmp");
        let root = std::fs::canonicalize(tmp.path()).expect("root abs");
        let setup = root.join("setup/graphql");

        let mut resolved = ResolvedSetup::graphql(setup, None);
        let jwts_env = resolved.jwts_env.as_ref().expect("jwts env").clone();
        write_file(&jwts_env, "GQL_JWT_DEFAULT=abc\n");
        resolved.jwts_local_env = None;

        let files: Vec<PathBuf> = resolved
            .jwts_files()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();

        assert_eq!(files, vec![jwts_env]);
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

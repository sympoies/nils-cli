use std::path::{Path, PathBuf};

use crate::{env_file, Result};

const FALLBACK_CANDIDATES: &[&str] = &[
    "schema.gql",
    "schema.graphql",
    "schema.graphqls",
    "api.graphql",
    "api.gql",
];

fn resolve_relative_under_setup(setup_dir: &Path, rel: &Path) -> PathBuf {
    if rel.is_absolute() {
        return rel.to_path_buf();
    }

    let parent = rel.parent().unwrap_or_else(|| Path::new("."));
    let parent_abs = std::fs::canonicalize(setup_dir.join(parent)).unwrap_or_else(|_| {
        // best-effort: mirror legacy script behavior which canonicalizes the directory when possible.
        setup_dir.join(parent)
    });
    let filename = rel.file_name().unwrap_or(rel.as_os_str());
    parent_abs.join(filename)
}

pub fn resolve_schema_path(setup_dir: &Path, schema_file_arg: Option<&str>) -> Result<PathBuf> {
    let schema_file = schema_file_arg
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            std::env::var("GQL_SCHEMA_FILE").ok().and_then(|s| {
                let s = s.trim().to_string();
                (!s.is_empty()).then_some(s)
            })
        })
        .or_else(|| {
            let schema_local = setup_dir.join("schema.local.env");
            let schema_env = setup_dir.join("schema.env");
            let files: Vec<&Path> = vec![&schema_local, &schema_env];
            env_file::read_var_last_wins("GQL_SCHEMA_FILE", &files)
                .ok()
                .flatten()
        })
        .or_else(|| {
            for c in FALLBACK_CANDIDATES {
                if setup_dir.join(c).is_file() {
                    return Some((*c).to_string());
                }
            }
            None
        });

    let Some(schema_file) = schema_file else {
        anyhow::bail!(
            "Schema file not configured. Set GQL_SCHEMA_FILE in schema.env (or pass --file)."
        );
    };

    let schema_path = resolve_relative_under_setup(setup_dir, Path::new(&schema_file));
    if !schema_path.is_file() {
        anyhow::bail!("Schema file not found: {}", schema_path.display());
    }

    Ok(schema_path)
}

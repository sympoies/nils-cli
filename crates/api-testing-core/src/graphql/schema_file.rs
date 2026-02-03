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
            let files: Vec<&Path> = vec![&schema_env, &schema_local];
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn write_file(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn schema_file_arg_is_trimmed_and_resolved_under_setup() {
        let tmp = TempDir::new().expect("tmp");
        let setup_dir = std::fs::canonicalize(tmp.path()).expect("setup abs");

        write_file(
            &setup_dir.join("schemas/api.graphql"),
            "schema { query: Query }\n",
        );

        let got = resolve_schema_path(&setup_dir, Some("  schemas/api.graphql  ")).expect("path");
        let expected = std::fs::canonicalize(setup_dir.join("schemas/api.graphql")).expect("abs");
        assert_eq!(got, expected);
    }

    #[test]
    fn schema_file_env_var_is_used_when_no_arg() {
        let _guard = env_lock().lock().expect("lock env");
        let old = std::env::var("GQL_SCHEMA_FILE").ok();
        std::env::set_var("GQL_SCHEMA_FILE", "  schema.gql  ");

        let tmp = TempDir::new().expect("tmp");
        let setup_dir = std::fs::canonicalize(tmp.path()).expect("setup abs");
        write_file(&setup_dir.join("schema.gql"), "schema { query: Query }\n");

        let got = resolve_schema_path(&setup_dir, None).expect("path");
        let expected = std::fs::canonicalize(setup_dir.join("schema.gql")).expect("abs");
        assert_eq!(got, expected);

        match old {
            Some(v) => std::env::set_var("GQL_SCHEMA_FILE", v),
            None => std::env::remove_var("GQL_SCHEMA_FILE"),
        }
    }

    #[test]
    fn schema_file_schema_env_is_used_when_no_arg_or_env() {
        let _guard = env_lock().lock().expect("lock env");
        let old = std::env::var("GQL_SCHEMA_FILE").ok();
        std::env::remove_var("GQL_SCHEMA_FILE");

        let tmp = TempDir::new().expect("tmp");
        let setup_dir = std::fs::canonicalize(tmp.path()).expect("setup abs");

        write_file(
            &setup_dir.join("schema.env"),
            "export GQL_SCHEMA_FILE=schemas/schema.graphql\n",
        );
        write_file(
            &setup_dir.join("schemas/schema.graphql"),
            "schema { query: Query }\n",
        );

        let got = resolve_schema_path(&setup_dir, None).expect("path");
        let expected =
            std::fs::canonicalize(setup_dir.join("schemas/schema.graphql")).expect("abs");
        assert_eq!(got, expected);

        match old {
            Some(v) => std::env::set_var("GQL_SCHEMA_FILE", v),
            None => std::env::remove_var("GQL_SCHEMA_FILE"),
        }
    }

    #[test]
    fn schema_file_falls_back_to_candidate_filenames() {
        let _guard = env_lock().lock().expect("lock env");
        let old = std::env::var("GQL_SCHEMA_FILE").ok();
        std::env::remove_var("GQL_SCHEMA_FILE");

        let tmp = TempDir::new().expect("tmp");
        let setup_dir = std::fs::canonicalize(tmp.path()).expect("setup abs");

        write_file(&setup_dir.join("schema.gql"), "schema { query: Query }\n");

        let got = resolve_schema_path(&setup_dir, None).expect("path");
        let expected = std::fs::canonicalize(setup_dir.join("schema.gql")).expect("abs");
        assert_eq!(got, expected);

        match old {
            Some(v) => std::env::set_var("GQL_SCHEMA_FILE", v),
            None => std::env::remove_var("GQL_SCHEMA_FILE"),
        }
    }

    #[test]
    fn schema_file_errors_when_not_configured() {
        let _guard = env_lock().lock().expect("lock env");
        let old = std::env::var("GQL_SCHEMA_FILE").ok();
        std::env::remove_var("GQL_SCHEMA_FILE");

        let tmp = TempDir::new().expect("tmp");
        let setup_dir = std::fs::canonicalize(tmp.path()).expect("setup abs");

        let err = resolve_schema_path(&setup_dir, None).unwrap_err();
        assert!(err
            .to_string()
            .contains("Schema file not configured. Set GQL_SCHEMA_FILE"));

        match old {
            Some(v) => std::env::set_var("GQL_SCHEMA_FILE", v),
            None => std::env::remove_var("GQL_SCHEMA_FILE"),
        }
    }
}

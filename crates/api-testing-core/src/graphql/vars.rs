use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::Result;

#[derive(Debug, Clone, PartialEq)]
pub struct GraphqlVariablesFile {
    pub path: PathBuf,
    pub variables: serde_json::Value,
    /// Number of numeric `limit` fields bumped to the configured minimum.
    pub bumped_limit_fields: usize,
}

fn bump_limits_in_json(value: &mut serde_json::Value, min_limit: u64) -> usize {
    match value {
        serde_json::Value::Object(map) => {
            let mut bumped = 0usize;
            for (k, v) in map.iter_mut() {
                if k == "limit"
                    && let serde_json::Value::Number(n) = v
                    && let Some(as_f64) = n.as_f64()
                    && as_f64 < (min_limit as f64)
                {
                    *n = serde_json::Number::from(min_limit);
                    bumped += 1;
                }
                bumped += bump_limits_in_json(v, min_limit);
            }
            bumped
        }
        serde_json::Value::Array(values) => values
            .iter_mut()
            .map(|v| bump_limits_in_json(v, min_limit))
            .sum(),
        _ => 0,
    }
}

impl GraphqlVariablesFile {
    pub fn load(path: impl AsRef<Path>, min_limit: u64) -> Result<Self> {
        let path = path.as_ref();
        let bytes = std::fs::read(path)
            .with_context(|| format!("read variables file: {}", path.display()))?;
        let mut variables: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("Variables file is not valid JSON: {}", path.display()))?;

        let bumped_limit_fields = if min_limit > 0 {
            bump_limits_in_json(&mut variables, min_limit)
        } else {
            0
        };

        Ok(Self {
            path: std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            variables,
            bumped_limit_fields,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, contents).expect("write");
    }

    #[test]
    fn graphql_vars_bumps_nested_limit_fields() {
        let tmp = TempDir::new().expect("tmp");
        let vars_path = tmp.path().join("vars.json");
        write(
            &vars_path,
            r#"{"limit":1,"nested":{"limit":2},"arr":[{"limit":3},{"limit":10}],"str":{"limit":"2"}}"#,
        );

        let vars = GraphqlVariablesFile::load(&vars_path, 5).expect("load");
        assert_eq!(vars.bumped_limit_fields, 3);
        assert_eq!(vars.variables["limit"], 5);
        assert_eq!(vars.variables["nested"]["limit"], 5);
        assert_eq!(vars.variables["arr"][0]["limit"], 5);
        assert_eq!(vars.variables["arr"][1]["limit"], 10);
        assert_eq!(vars.variables["str"]["limit"], "2");
    }

    #[test]
    fn graphql_vars_invalid_json_includes_path() {
        let tmp = TempDir::new().expect("tmp");
        let vars_path = tmp.path().join("vars.json");
        write(&vars_path, "{nope");

        let err = GraphqlVariablesFile::load(&vars_path, 5).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("Variables file is not valid JSON"));
        assert!(msg.contains(vars_path.to_string_lossy().as_ref()));
    }
}

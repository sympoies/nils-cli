//! Repository-owned path classification shared by pre-edit and docs-impact gates.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use toml::Value;

const CLASS_KEYS: [&str; 4] = ["production", "test", "docs", "generated"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathClassContract {
    pub production: Vec<String>,
    pub test: Vec<String>,
    pub docs: Vec<String>,
    pub generated: Vec<String>,
    pub unmatched: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathClassification {
    pub path: String,
    pub path_class: String,
    pub matched_classes: Vec<String>,
}

impl PathClassContract {
    /// Loads `[path_classes]` from the repository-root `VALIDATION.toml`.
    ///
    /// A missing file or a file without a `path_classes` table is
    /// not configured.
    pub fn load_project(repo: &Path) -> Result<Option<Self>, String> {
        Self::load_file(&repo.join("VALIDATION.toml"))
    }

    fn load_file(path: &Path) -> Result<Option<Self>, String> {
        if !path.try_exists().map_err(|err| err.to_string())? {
            return Ok(None);
        }
        let source = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
        let root = toml::from_str::<Value>(&source).map_err(|err| err.to_string())?;
        let Some(value) = root.get("path_classes") else {
            return Ok(None);
        };
        let table = value
            .as_table()
            .ok_or_else(|| "`path_classes` must be a table".to_string())?;
        Self::from_toml(table).map(Some)
    }

    pub fn from_toml(table: &toml::map::Map<String, Value>) -> Result<Self, String> {
        for key in table.keys() {
            if !CLASS_KEYS.contains(&key.as_str()) && key != "unmatched" {
                return Err(format!(
                    "unsupported field `{key}`; allowed fields: production, test, docs, generated, unmatched"
                ));
            }
        }
        let read_patterns = |key: &str| -> Result<Vec<String>, String> {
            let Some(value) = table.get(key) else {
                return Ok(Vec::new());
            };
            let items = value
                .as_array()
                .ok_or_else(|| format!("`{key}` must be an array of glob strings"))?;
            let mut patterns = Vec::new();
            for value in items {
                let pattern = value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| format!("`{key}` entries must be non-empty strings"))?;
                validate_pattern(pattern)?;
                patterns.push(pattern.to_string());
            }
            patterns.sort();
            patterns.dedup();
            Ok(patterns)
        };
        let unmatched = table
            .get("unmatched")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .trim();
        if unmatched != "unknown" {
            return Err("`unmatched` must equal `unknown`".to_string());
        }
        Ok(Self {
            production: read_patterns("production")?,
            test: read_patterns("test")?,
            docs: read_patterns("docs")?,
            generated: read_patterns("generated")?,
            unmatched: unmatched.to_string(),
        })
    }

    pub fn classify(&self, path: &Path) -> Result<PathClassification, String> {
        let normalized = normalize_relative(path)?;
        let mut by_class = BTreeMap::new();
        by_class.insert("production", &self.production);
        by_class.insert("test", &self.test);
        by_class.insert("docs", &self.docs);
        by_class.insert("generated", &self.generated);
        let matched: Vec<String> = by_class
            .into_iter()
            .filter(|(_, patterns)| patterns.iter().any(|glob| glob_matches(glob, &normalized)))
            .map(|(class, _)| class.to_string())
            .collect();
        let path_class = match matched.as_slice() {
            [] => self.unmatched.clone(),
            [only] => only.clone(),
            _ => "ambiguous".to_string(),
        };
        Ok(PathClassification {
            path: normalized,
            path_class,
            matched_classes: matched,
        })
    }
}

fn validate_pattern(pattern: &str) -> Result<(), String> {
    let path = Path::new(pattern);
    if path.is_absolute()
        || path.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "path-class glob `{pattern}` must be repository-relative and cannot traverse parents"
        ));
    }
    Ok(())
}

fn normalize_relative(path: &Path) -> Result<String, String> {
    if path.is_absolute() {
        return Err("path must be repository-relative".to_string());
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(value.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("path cannot contain traversal or root components".to_string());
            }
        }
    }
    if parts.is_empty() {
        return Err("path must not be empty".to_string());
    }
    Ok(parts.join("/"))
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    let pattern: Vec<&str> = pattern
        .trim_start_matches("./")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let path: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match_segments(&pattern, &path)
}

fn match_segments(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.split_first(), path.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((&"**", rest)), _) => {
            match_segments(rest, path)
                || path
                    .split_first()
                    .is_some_and(|(_, tail)| match_segments(pattern, tail))
        }
        (Some((head, rest)), Some((value, tail))) => {
            segment_matches(head, value) && match_segments(rest, tail)
        }
        (Some(_), None) => pattern.iter().all(|part| *part == "**"),
    }
}

fn segment_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut p, mut v) = (0, 0);
    let (mut star, mut star_value) = (None, 0);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            star_value = v;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            star_value += 1;
            v = star_value;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

pub fn changed_path_digest(paths: impl IntoIterator<Item = String>) -> String {
    use sha2::{Digest, Sha256};
    let paths: BTreeSet<String> = paths.into_iter().collect();
    let mut hasher = Sha256::new();
    for path in paths {
        hasher.update(path.as_bytes());
        hasher.update(b"\n");
    }
    format!("sha256:{}", hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALIDATION_CLASSES: &str = r#"[[validation]]
context = "project-dev"
commands = ["true"]

[path_classes]
production = ["lib/**"]
test = ["spec/**"]
"#;

    const RETIRED_CATALOG_CLASSES: &str = r#"[path_classes]
production = ["src/**"]
test = ["tests/**"]
"#;

    fn class_of(repo: &Path, path: &str) -> String {
        PathClassContract::load_project(repo)
            .unwrap()
            .expect("path-class contract")
            .classify(Path::new(path))
            .unwrap()
            .path_class
    }

    #[test]
    fn load_project_reads_validation_toml_path_classes() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("VALIDATION.toml"), VALIDATION_CLASSES).unwrap();
        assert_eq!(class_of(tmp.path(), "lib/value.rs"), "production");
        assert_eq!(class_of(tmp.path(), "spec/value.rs"), "test");
    }

    #[test]
    fn load_project_ignores_retired_agent_docs_toml() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("AGENT_DOCS.toml"), RETIRED_CATALOG_CLASSES).unwrap();
        assert_eq!(PathClassContract::load_project(tmp.path()).unwrap(), None);

        std::fs::write(
            tmp.path().join("VALIDATION.toml"),
            "[[validation]]\ncontext = \"project-dev\"\ncommands = [\"true\"]\n",
        )
        .unwrap();
        assert_eq!(PathClassContract::load_project(tmp.path()).unwrap(), None);

        std::fs::write(tmp.path().join("VALIDATION.toml"), VALIDATION_CLASSES).unwrap();
        assert_eq!(class_of(tmp.path(), "lib/value.rs"), "production");
        assert_eq!(class_of(tmp.path(), "src/value.rs"), "unknown");
    }

    #[test]
    fn load_project_without_contract_is_not_configured() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(PathClassContract::load_project(tmp.path()).unwrap(), None);
    }
}

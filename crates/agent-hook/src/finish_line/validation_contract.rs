use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub(super) struct ValidationContract {
    pub context: String,
    pub declared: bool,
    pub commands: Vec<String>,
    pub marker: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValidationEntry {
    context: String,
    commands: Vec<String>,
    #[serde(default)]
    marker: Option<String>,
    #[serde(rename = "description")]
    #[serde(default)]
    _description: Option<String>,
    // Product selectors are accepted for shared VALIDATION.toml files and
    // do not narrow the contract read here.
    #[serde(rename = "product")]
    #[serde(default)]
    _product: Option<ProductSelector>,
    #[serde(rename = "products")]
    #[serde(default)]
    _products: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
enum ProductSelector {
    One(String),
    Many(Vec<String>),
}

pub(super) fn valid_intent(intent: &str) -> bool {
    !intent.trim().is_empty()
        && intent
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
}

pub(super) fn load_from_repo(repo: &Path) -> Result<Vec<ValidationContract>, String> {
    let mut contracts = BTreeMap::<String, ValidationContract>::new();
    let path = repo.join("VALIDATION.toml");
    if !path.try_exists().map_err(|err| err.to_string())? {
        return Ok(Vec::new());
    }
    for mut entry in read_entries(&path)? {
        let context = entry.context.trim().to_string();
        if !valid_intent(&context) {
            return Err("invalid validation context".into());
        }
        entry.context = context;
        entry.commands = normalize_commands(entry.commands)?;
        match contracts.get_mut(&entry.context) {
            Some(current) => merge(current, entry),
            None => {
                contracts.insert(entry.context.clone(), contract(entry));
            }
        }
    }
    Ok(contracts.into_values().collect())
}

fn merge(current: &mut ValidationContract, entry: ValidationEntry) {
    for command in entry.commands {
        if !current.commands.contains(&command) {
            current.commands.push(command);
        }
    }
    if entry.marker.is_some() {
        current.marker = entry.marker;
    }
}

fn normalize_commands(commands: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for command in commands {
        let command = command.trim();
        if command.is_empty() {
            return Err("validation commands must be non-empty after trimming".into());
        }
        if !normalized.iter().any(|existing| existing == command) {
            normalized.push(command.to_string());
        }
    }
    if normalized.is_empty() {
        return Err("validation commands must contain at least one command".into());
    }
    Ok(normalized)
}

fn read_entries(path: &Path) -> Result<Vec<ValidationEntry>, String> {
    let text = fs::read_to_string(path).map_err(|err| err.to_string())?;
    let document = toml::from_str::<toml::Value>(&text).map_err(|err| err.to_string())?;
    let Some(validation) = document.get("validation") else {
        return Ok(Vec::new());
    };
    let entries = validation
        .as_array()
        .ok_or_else(|| "validation must be an array of tables".to_string())?;
    entries
        .iter()
        .cloned()
        .map(toml::Value::try_into)
        .collect::<Result<Vec<ValidationEntry>, _>>()
        .map_err(|err| err.to_string())
}

fn contract(entry: ValidationEntry) -> ValidationContract {
    ValidationContract {
        context: entry.context,
        declared: true,
        commands: entry.commands,
        marker: entry.marker,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn reads_validation_toml_and_merges_repeated_contexts() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("VALIDATION.toml"),
            "[[validation]]\ncontext = \"project-dev\"\nproduct = [\"codex\", \"dsh\"]\ncommands = [\" cargo test \", \"cargo test\"]\nmarker = \".cache/validation.ok\"\n\n[[validation]]\ncontext = \"shared\"\ncommands = [\"first\"]\n\n[[validation]]\ncontext = \"shared\"\nproduct = \"dsh\"\ncommands = [\"second\"]\n\n[path_classes]\nproduction = [\"src/**\"]\n",
        )
        .expect("contract");

        let contracts = load_from_repo(temp.path()).expect("contracts");
        assert_eq!(contracts.len(), 2);
        assert_eq!(contracts[0].context, "project-dev");
        assert_eq!(contracts[0].commands, ["cargo test"]);
        assert_eq!(contracts[0].marker.as_deref(), Some(".cache/validation.ok"));
        assert_eq!(contracts[1].context, "shared");
        assert_eq!(contracts[1].commands, ["first", "second"]);
    }

    #[test]
    fn retired_agent_docs_toml_is_not_read() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("AGENT_DOCS.toml"),
            "[[validation]]\ncontext = \"project-dev\"\nproduct = \"dsh\"\ncommands = [\"older check\"]\n",
        )
        .expect("retired catalog");
        assert!(load_from_repo(temp.path()).expect("contracts").is_empty());

        fs::write(
            temp.path().join("VALIDATION.toml"),
            "[[validation]]\ncontext = \"other\"\ncommands = [\"cargo test\"]\n",
        )
        .expect("contract");
        let contracts = load_from_repo(temp.path()).expect("contracts");
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].context, "other");
    }

    #[test]
    fn empty_or_whitespace_commands_are_rejected() {
        for commands in ["[]", "[\"  \"]"] {
            let temp = TempDir::new().expect("temp dir");
            fs::write(
                temp.path().join("VALIDATION.toml"),
                format!("[[validation]]\ncontext = \"project-dev\"\ncommands = {commands}\n"),
            )
            .expect("invalid contract");
            assert!(load_from_repo(temp.path()).is_err(), "commands={commands}");
        }
    }
}

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
    // Transitional fields retained from the earlier catalog format.
    #[serde(default)]
    product: Option<TransitionalProduct>,
    #[serde(default)]
    products: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TransitionalProduct {
    One(String),
    Many(Vec<String>),
}

impl TransitionalProduct {
    fn includes_dsh(&self) -> bool {
        match self {
            Self::One(product) => product == "dsh",
            Self::Many(products) => products.iter().any(|product| product == "dsh"),
        }
    }
}

pub(super) fn valid_intent(intent: &str) -> bool {
    !intent.trim().is_empty()
        && intent
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/'))
}

pub(super) fn load_from_repo(repo: &Path) -> Result<Vec<ValidationContract>, String> {
    let mut contracts = BTreeMap::<String, ValidationContract>::new();
    let mut primary_contexts = std::collections::BTreeSet::new();
    let primary = repo.join("VALIDATION.toml");
    if primary.try_exists().map_err(|err| err.to_string())? {
        let entries = read_entries(&primary)?;
        for entry in entries {
            let context = entry.context.trim().to_string();
            if !valid_intent(&context) {
                return Err("invalid validation context".into());
            }
            let mut entry = entry;
            entry.context = context;
            entry.commands = normalize_commands(entry.commands)?;
            primary_contexts.insert(entry.context.clone());
            match contracts.get_mut(&entry.context) {
                Some(current) => merge(current, entry),
                None => {
                    contracts.insert(entry.context.clone(), contract(entry));
                }
            }
        }
    }

    let compatibility_path = repo.join("AGENT_DOCS.toml");
    if compatibility_path
        .try_exists()
        .map_err(|err| err.to_string())?
    {
        for entry in read_entries(&compatibility_path)? {
            let context = entry.context.trim().to_string();
            if !valid_intent(&context) {
                return Err("invalid validation context".into());
            }
            let mut entry = entry;
            entry.context = context;
            if entry
                .product
                .as_ref()
                .is_some_and(|product| !product.includes_dsh())
                || (!entry.products.is_empty() && !entry.products.iter().any(|p| p == "dsh"))
            {
                continue;
            }
            let mut entry = entry;
            entry.commands = normalize_commands(entry.commands)?;
            if primary_contexts.contains(&entry.context) {
                continue;
            }
            match contracts.get_mut(&entry.context) {
                Some(current) => merge(current, entry),
                None => {
                    contracts.insert(entry.context.clone(), contract(entry));
                }
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
    fn reads_new_contract_and_transitional_dsh_contracts() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("VALIDATION.toml"),
            "[[validation]]\ncontext = \"project-dev\"\ncommands = [\"cargo test\"]\nmarker = \".cache/validation.ok\"\n",
        )
        .expect("new contract");
        fs::write(
            temp.path().join("AGENT_DOCS.toml"),
            "[[document]]\ncontext = \"project-dev\"\npath = \"GUIDE.md\"\n\n[skills]\nenforce_name_prefix = true\n\n[[validation]]\ncontext = \"project-dev\"\nproduct = \"dsh\"\ncommands = [\"older check\"]\n\n[[validation]]\ncontext = \"shared\"\ncommands = [\"first\"]\n\n[[validation]]\ncontext = \"shared\"\nproduct = \"dsh\"\ncommands = [\"second\"]\n\n[[validation]]\ncontext = \"other\"\nproduct = \"codex\"\ncommands = [\"ignored\"]\n",
        )
        .expect("compatible contract");

        let contracts = load_from_repo(temp.path()).expect("contracts");
        assert_eq!(contracts.len(), 2);
        assert_eq!(contracts[0].commands, ["cargo test"]);
        assert_eq!(contracts[0].marker.as_deref(), Some(".cache/validation.ok"));
        assert_eq!(contracts[1].context, "shared");
        assert_eq!(contracts[1].commands, ["first", "second"]);
    }

    #[test]
    fn compatibility_products_accept_arrays_and_commands_are_normalized() {
        let temp = TempDir::new().expect("temp dir");
        fs::write(
            temp.path().join("AGENT_DOCS.toml"),
            "[[validation]]\ncontext = \"project-dev\"\nproduct = [\"hermes\", \"dsh\"]\ncommands = [\" cargo test \", \"cargo test\"]\n\n[[validation]]\ncontext = \"other\"\nproduct = [\"hermes\"]\ncommands = [\"ignored\"]\n",
        )
        .expect("transitional contracts");

        let contracts = load_from_repo(temp.path()).expect("contracts");
        assert_eq!(contracts.len(), 1);
        assert_eq!(contracts[0].commands, ["cargo test"]);
    }

    #[test]
    fn empty_or_whitespace_commands_are_rejected_in_both_formats() {
        for (file, product) in [
            ("VALIDATION.toml", ""),
            ("AGENT_DOCS.toml", "product = \"dsh\"\n"),
        ] {
            for commands in ["[]", "[\"  \"]"] {
                let temp = TempDir::new().expect("temp dir");
                fs::write(
                    temp.path().join(file),
                    format!("[[validation]]\ncontext = \"project-dev\"\n{product}commands = {commands}\n"),
                )
                .expect("invalid contract");
                assert!(
                    load_from_repo(temp.path()).is_err(),
                    "file={file} commands={commands}"
                );
            }
        }
    }
}

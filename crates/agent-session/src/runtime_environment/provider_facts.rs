//! Only derived launch facts leave the private provider configuration.
use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::SessionRecord;

const KEY: &str = "provider_launch_facts";
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
pub(super) const CLAUDE_KEYS: [&str; 4] = [
    "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
];

#[derive(Debug, Deserialize, Serialize)]
struct Facts {
    claude_window_tokens: Option<u64>,
    account_applicable: bool,
}

fn capacity(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse::<u64>().ok())
        .flatten()
        .filter(|value| *value > 0 && *value <= MAX_SAFE_INTEGER)
}

fn derive(record: &SessionRecord, ambient: bool) -> Facts {
    let mut facts = Facts {
        claude_window_tokens: None,
        account_applicable: true,
    };
    let mut env = BTreeMap::new();
    if record.agent == "claude" && ambient {
        for key in CLAUDE_KEYS {
            match std::env::var(key) {
                Ok(value) => {
                    env.insert(key.to_string(), value);
                }
                Err(std::env::VarError::NotUnicode(_)) => {
                    env.insert(key.to_string(), "<non-unicode>".into());
                }
                Err(std::env::VarError::NotPresent) => {}
            }
        }
    }
    let mut provider = "openai".to_string();
    let mut provider_auth = BTreeMap::new();
    let mut args = record.agent_args.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            break;
        }
        let (flag, inline) = arg
            .split_once('=')
            .map_or((arg.as_str(), None), |(key, value)| (key, Some(value)));
        if record.agent == "claude" && flag == "--settings" {
            if let Some(raw) = inline.or_else(|| args.next().map(String::as_str))
                && raw.len() <= 64 * 1024
                && let Ok(settings) = serde_json::from_str::<Value>(raw)
                && let Some(settings) = settings.get("env").and_then(Value::as_object)
            {
                for key in CLAUDE_KEYS {
                    if let Some(value) = settings.get(key) {
                        // An explicit invalid value must not resurrect a lower
                        // precedence ambient value, especially for capacity.
                        env.insert(
                            key.to_string(),
                            value.as_str().unwrap_or("<invalid>").to_string(),
                        );
                    }
                }
            }
        } else if record.agent == "codex" {
            if flag == "--oss" {
                facts.account_applicable = false;
            }
            let config = match flag {
                "-c" | "--config" => inline.or_else(|| args.next().map(String::as_str)),
                _ if arg.starts_with("-c") && arg.len() > 2 => Some(&arg[2..]),
                _ => None,
            };
            if let Some(config) = config.filter(|value| value.len() <= 64 * 1024) {
                if let Some(("model_provider", raw)) = config
                    .split_once('=')
                    .map(|(key, value)| (key.trim(), value.trim()))
                {
                    let quoted = format!("value={raw}")
                        .parse::<toml_edit::DocumentMut>()
                        .ok()
                        .and_then(|doc| {
                            doc.get("value")
                                .and_then(toml_edit::Item::as_str)
                                .map(str::to_string)
                        });
                    provider = quoted.unwrap_or_else(|| raw.to_string());
                }
                let Ok(doc) = config.parse::<toml_edit::DocumentMut>() else {
                    continue;
                };
                if let Some(value) = doc.get("model_provider").and_then(toml_edit::Item::as_str) {
                    provider = value.to_string();
                }
                if let Some(providers) = doc
                    .get("model_providers")
                    .and_then(toml_edit::Item::as_table_like)
                {
                    for (name, config) in providers.iter() {
                        if let Some(auth) = config
                            .get("requires_openai_auth")
                            .and_then(toml_edit::Item::as_bool)
                        {
                            provider_auth.insert(name.to_string(), auth);
                        }
                    }
                }
            }
        }
    }
    if record.agent == "claude" {
        facts.claude_window_tokens = env.get(CLAUDE_KEYS[0]).and_then(|value| capacity(value));
        let custom_endpoint = env.get(CLAUDE_KEYS[1]).is_some_and(|value| {
            !matches!(
                value.trim(),
                "" | "https://api.anthropic.com" | "https://api.anthropic.com/"
            )
        });
        facts.account_applicable = !custom_endpoint
            && CLAUDE_KEYS[2..]
                .iter()
                .all(|key| env.get(*key).is_none_or(|value| value.trim().is_empty()));
    } else if record.agent == "codex" && provider_auth.get(&provider) == Some(&false) {
        facts.account_applicable = false;
    }
    facts
}

fn configuration_digest(record: &SessionRecord) -> String {
    // Bind the snapshot without retaining raw launch inputs or private paths.
    let inputs = serde_json::to_vec(&(
        &record.agent_args,
        &record.agent_bin,
        crate::session_agent_profile(record),
        crate::session_provider_config_dir(record),
    ))
    .expect("launch inputs serialize");
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(inputs) {
        write!(&mut digest, "{byte:02x}").expect("write to string");
    }
    digest
}

pub(super) fn capture(record: &mut SessionRecord) {
    let Some(runtime_id) = record
        .runtime
        .as_ref()
        .map(|runtime| &runtime.launch_id)
        .filter(|id| !id.is_empty())
    else {
        return;
    };
    let mut value =
        serde_json::to_value(derive(record, true)).expect("primitive launch facts serialize");
    value["runtime_id"] = json!(runtime_id);
    value["agent"] = json!(record.agent);
    value["configuration_digest"] = json!(configuration_digest(record));
    record.extra.insert(KEY.into(), value);
}

fn for_record(record: &SessionRecord) -> Facts {
    if let Some(value) = record.extra.get(KEY)
        && let Some(runtime) = record
            .runtime
            .as_ref()
            .filter(|runtime| !runtime.launch_id.is_empty())
        && value.get("runtime_id").and_then(Value::as_str) == Some(&runtime.launch_id)
        && value.get("agent").and_then(Value::as_str) == Some(&record.agent)
        && value.get("configuration_digest").and_then(Value::as_str)
            == Some(configuration_digest(record).as_str())
        && let Ok(mut facts) = serde_json::from_value::<Facts>(value.clone())
    {
        facts.claude_window_tokens = facts
            .claude_window_tokens
            .filter(|value| *value > 0 && *value <= MAX_SAFE_INTEGER);
        return facts;
    }
    // Older records still have explicit launch args, but today's daemon env is
    // never evidence of the environment inherited by an existing runtime.
    derive(record, false)
}

pub(crate) fn claude_window(record: &SessionRecord) -> Option<u64> {
    for_record(record).claude_window_tokens
}

pub(crate) fn account_applicable(record: &SessionRecord) -> bool {
    for_record(record).account_applicable
}

#[cfg(test)]
mod tests;

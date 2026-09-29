//! `agent-session serve --config <file>`: one versioned document for the serve
//! inputs that deployments otherwise assemble as JSON-in-environment values.
//!
//! The document is resolved into the same environment variables serve already
//! reads, so the daemon behaves exactly as if a launcher had exported them.
//! Non-empty environment variables keep working and take precedence:
//!
//! - `retitle` and `codex_account_broker` are replaced wholesale by
//!   `AGENT_SESSION_RETITLE_CONFIG` / `AGENT_SESSION_CODEX_ACCOUNT_BROKER`.
//! - `launch_profiles` merge with `AGENT_SESSION_LAUNCH_PROFILES`: environment
//!   entries first, then file entries in order; the first entry for an id wins
//!   and a later duplicate is dropped with a warning.
//! - `path.append` entries are appended after the inherited `PATH`.
//!
//! Diagnostics are path- and value-free: they name the offending key and never
//! echo the file location or its content. The schema has no field that holds a
//! secret; credentials are referenced by environment variable name
//! (`retitle.api_key_env`), and secret-shaped keys are refused outright.

use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};

use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, exit, schema_version_for};
use serde::Serialize;
use serde_json::{Map, Number, Value, json};

use crate::CliError;

pub(crate) const CONFIG_SCHEMA_VERSION: &str = "agent-session.serve-config.v1";
const ENVELOPE_COMMAND: &str = "serve-config";
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
const MAX_PATH_APPEND: usize = 16;
const MAX_PATH_ENTRY_BYTES: usize = 4096;

const LAUNCH_PROFILES_ENV: &str = "AGENT_SESSION_LAUNCH_PROFILES";
const RETITLE_ENV: &str = "AGENT_SESSION_RETITLE_CONFIG";
const BROKER_ENV: &str = "AGENT_SESSION_CODEX_ACCOUNT_BROKER";

const ROOT_KEYS: &[&str] = &[
    "schema_version",
    "launch_profiles",
    "retitle",
    "codex_account_broker",
    "path",
];
const PATH_KEYS: &[&str] = &["append"];
const BROKER_KEYS: &[&str] = &["argv"];
const LAUNCH_PROFILE_KEYS: &[&str] = &[
    "id",
    "label",
    "agent",
    "agent_bin",
    "provider_config_dir",
    "readiness_args",
    "auto_resume_supported",
    "graceful_shutdown",
    "codex_usage_account",
    "dsh_history",
];
const DSH_HISTORY_KEYS: &[&str] = &["command", "root", "compression", "resume"];
const RETITLE_KEYS: &[&str] = &[
    "provider",
    "fallback",
    "account",
    "account_selection",
    "model",
    "reasoning_effort",
    "codex_bin",
    "base_url",
    "api_key_env",
    "argv",
    "timeout_ms",
    "max_output_tokens",
    "temperature",
    "extra_body",
    "json_response",
    "max_concurrency",
    "queue_size",
    "context",
];
/// Root-owned retitle fields a fallback provider may not carry.
const RETITLE_ROOT_ONLY_KEYS: &[&str] = &["fallback", "max_concurrency", "queue_size", "context"];
const RETITLE_CONTEXT_KEYS: &[&str] = &["max_chars", "per_message_chars", "recent_turns"];

/// Key-name words that mark a value as a credential wherever they appear in a
/// key. Such a key must instead name an environment variable (`*_env`) or a
/// file (`*_file`).
const SECRET_WORDS: &[&str] = &[
    "secret",
    "password",
    "passwd",
    "apikey",
    "credential",
    "credentials",
    "bearer",
    "authorization",
];
/// Words that mark a credential only as a key's final word: `access_token` is
/// one, while `stop_token_ids` and `max_output_tokens` are provider
/// parameters that merely mention tokens.
const FINAL_SECRET_WORDS: &[&str] = &["token"];
/// Adjacent word pairs that mark a credential wherever they appear in a key.
const SECRET_PAIRS: &[(&str, &str)] = &[
    ("api", "key"),
    ("private", "key"),
    ("access", "key"),
    ("secret", "key"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Source {
    None,
    File,
    Environment,
    Merged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DocumentFormat {
    Toml,
    Json,
}

#[derive(Debug, Serialize)]
struct LaunchProfilesSummary {
    source: Source,
    ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct SourceSummary {
    source: Source,
}

#[derive(Debug, Serialize)]
struct PathSummary {
    append: usize,
}

/// The `--check` result. It names sources and ids only, never paths or values.
#[derive(Debug, Serialize)]
struct CheckSummary {
    config_schema_version: &'static str,
    format: DocumentFormat,
    launch_profiles: LaunchProfilesSummary,
    retitle: SourceSummary,
    codex_account_broker: SourceSummary,
    path: PathSummary,
}

/// A validated document merged with the environment it will run under.
#[derive(Debug)]
pub(crate) struct ResolvedServeConfig {
    summary: CheckSummary,
    warnings: Vec<String>,
    assignments: Vec<(&'static str, OsString)>,
}

impl ResolvedServeConfig {
    /// Environment variables serve must set so the rest of the daemon observes
    /// the file exactly as if a launcher had exported them. Only values the
    /// environment does not already supply appear here.
    pub(crate) fn environment_assignments(&self) -> &[(&'static str, OsString)] {
        &self.assignments
    }
}

/// Validate and apply `--config` for `serve`. `Break` carries the exit code
/// for a completed `--check` or a rejected document; `Continue` means the
/// resolved values are now in this process's environment.
pub(crate) fn apply_for_serve(path: &Path, check: bool, format: OutputFormat) -> ControlFlow<i32> {
    let resolved = match load(path, &|key| env::var_os(key)) {
        Ok(resolved) => resolved,
        Err(error) => return ControlFlow::Break(render_error(format, error)),
    };
    if check {
        return ControlFlow::Break(render_check(format, &resolved));
    }
    for warning in &resolved.warnings {
        let _ = writeln!(io::stderr(), "warning: {warning}");
    }
    for (key, value) in resolved.environment_assignments() {
        // SAFETY: serve applies its config on the main thread before it builds
        // the async runtime or starts any helper thread, so no other thread can
        // be reading the environment concurrently.
        unsafe { env::set_var(key, value) };
    }
    ControlFlow::Continue(())
}

pub(crate) fn load(
    path: &Path,
    lookup: &dyn Fn(&str) -> Option<OsString>,
) -> Result<ResolvedServeConfig, CliError> {
    let format = document_format(path)?;
    let raw = read_bounded(path)?;
    let document = match format {
        DocumentFormat::Toml => parse_toml(&raw)?,
        DocumentFormat::Json => parse_json(&raw)?,
    };
    resolve(format, &document, lookup)
}

fn document_format(path: &Path) -> Result<DocumentFormat, CliError> {
    match path.extension().and_then(|value| value.to_str()) {
        Some("toml") => Ok(DocumentFormat::Toml),
        Some("json") => Ok(DocumentFormat::Json),
        _ => Err(CliError::usage(
            "serve-config-unsupported-format",
            "serve config must be a .toml or .json file",
            None,
        )),
    }
}

fn read_bounded(path: &Path) -> Result<String, CliError> {
    let unreadable = |reason: &str| {
        CliError::usage(
            "serve-config-unreadable",
            "serve config file could not be read",
            Some(json!({ "reason": reason })),
        )
    };
    let io_reason = |error: &io::Error| match error.kind() {
        io::ErrorKind::NotFound => "not_found",
        io::ErrorKind::PermissionDenied => "permission_denied",
        _ => "io_error",
    };
    let file = File::open(path).map_err(|error| unreadable(io_reason(&error)))?;
    let metadata = file
        .metadata()
        .map_err(|error| unreadable(io_reason(&error)))?;
    if !metadata.is_file() {
        return Err(unreadable("not_a_file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| unreadable(io_reason(&error)))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(CliError::usage(
            "serve-config-too-large",
            "serve config exceeds 256 KiB",
            None,
        ));
    }
    String::from_utf8(bytes).map_err(|_| parse_failed("serve config is not valid UTF-8", None))
}

fn parse_failed(message: &str, position: Option<(usize, usize)>) -> CliError {
    CliError::usage(
        "serve-config-parse-failed",
        message,
        position.map(|(line, column)| json!({ "line": line, "column": column })),
    )
}

fn parse_json(raw: &str) -> Result<Value, CliError> {
    serde_json::from_str(raw).map_err(|error| {
        parse_failed(
            "serve config is not a valid JSON document",
            Some((error.line(), error.column())),
        )
    })
}

fn parse_toml(raw: &str) -> Result<Value, CliError> {
    let document = raw.parse::<toml_edit::DocumentMut>().map_err(|error| {
        // Report only the position: the parser's own rendering quotes the
        // offending source line, which is document content.
        let position = error.span().map(|span| line_column(raw, span.start));
        parse_failed("serve config is not a valid TOML document", position)
    })?;
    toml_table_to_json(document.as_table(), "")
}

fn line_column(raw: &str, offset: usize) -> (usize, usize) {
    let prefix = &raw.as_bytes()[..offset.min(raw.len())];
    let line = prefix.iter().filter(|byte| **byte == b'\n').count() + 1;
    let column = prefix
        .iter()
        .rev()
        .take_while(|byte| **byte != b'\n')
        .count()
        + 1;
    (line, column)
}

fn toml_table_to_json(table: &toml_edit::Table, prefix: &str) -> Result<Value, CliError> {
    let mut object = Map::new();
    for (key, item) in table.iter() {
        let path = join_key(prefix, key);
        if let Some(value) = toml_item_to_json(item, &path)? {
            object.insert(key.to_string(), value);
        }
    }
    Ok(Value::Object(object))
}

fn toml_item_to_json(item: &toml_edit::Item, path: &str) -> Result<Option<Value>, CliError> {
    Ok(match item {
        toml_edit::Item::None => None,
        toml_edit::Item::Value(value) => Some(toml_value_to_json(value, path)?),
        toml_edit::Item::Table(table) => Some(toml_table_to_json(table, path)?),
        toml_edit::Item::ArrayOfTables(tables) => Some(Value::Array(
            tables
                .iter()
                .enumerate()
                .map(|(index, table)| toml_table_to_json(table, &format!("{path}[{index}]")))
                .collect::<Result<_, _>>()?,
        )),
    })
}

fn toml_value_to_json(value: &toml_edit::Value, path: &str) -> Result<Value, CliError> {
    Ok(match value {
        toml_edit::Value::String(value) => Value::String(value.value().clone()),
        toml_edit::Value::Integer(value) => Value::Number((*value.value()).into()),
        toml_edit::Value::Float(value) => Number::from_f64(*value.value())
            .map(Value::Number)
            .ok_or_else(|| invalid_value(path, "must be a finite number"))?,
        toml_edit::Value::Boolean(value) => Value::Bool(*value.value()),
        toml_edit::Value::Datetime(_) => {
            return Err(invalid_value(path, "date-time values are not supported"));
        }
        toml_edit::Value::Array(array) => Value::Array(
            array
                .iter()
                .enumerate()
                .map(|(index, value)| toml_value_to_json(value, &format!("{path}[{index}]")))
                .collect::<Result<_, _>>()?,
        ),
        toml_edit::Value::InlineTable(table) => {
            let mut object = Map::new();
            for (key, value) in table.iter() {
                object.insert(
                    key.to_string(),
                    toml_value_to_json(value, &join_key(path, key))?,
                );
            }
            Value::Object(object)
        }
    })
}

/// Render a document key for a diagnostic. Keys come from the document, so a
/// key that is not a plain identifier is replaced rather than echoed.
fn display_key(key: &str) -> &str {
    if !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        key
    } else {
        "<non-identifier-key>"
    }
}

fn join_key(prefix: &str, key: &str) -> String {
    let key = display_key(key);
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

fn keyed_error(code: &str, key: &str, source: &str, reason: &str) -> CliError {
    CliError::usage(
        code,
        format!("{key}: {reason}"),
        Some(json!({ "key": key, "source": source })),
    )
}

fn invalid_value(key: &str, reason: &str) -> CliError {
    keyed_error("serve-config-invalid-value", key, "file", reason)
}

fn unknown_key(key: &str) -> CliError {
    keyed_error(
        "serve-config-unknown-key",
        key,
        "file",
        "unknown serve config key",
    )
}

fn resolve(
    format: DocumentFormat,
    document: &Value,
    lookup: &dyn Fn(&str) -> Option<OsString>,
) -> Result<ResolvedServeConfig, CliError> {
    reject_inline_secrets(document, "", false)?;
    let root = document.as_object().ok_or_else(|| {
        keyed_error(
            "serve-config-unsupported-version",
            "schema_version",
            "file",
            "the document must be a table carrying schema_version",
        )
    })?;
    if root.get("schema_version").and_then(Value::as_str) != Some(CONFIG_SCHEMA_VERSION) {
        return Err(keyed_error(
            "serve-config-unsupported-version",
            "schema_version",
            "file",
            "schema_version must be \"agent-session.serve-config.v1\"",
        ));
    }
    reject_unknown_keys(root, ROOT_KEYS, "")?;

    let mut warnings = Vec::new();
    let mut assignments = Vec::new();
    let env_value = |key: &str| {
        lookup(key)
            .and_then(|value| value.into_string().ok())
            .filter(|value| !value.trim().is_empty())
    };

    let path_entries = resolve_path_append(root.get("path"))?;
    if !path_entries.is_empty() {
        let inherited = lookup("PATH").unwrap_or_default();
        let mut entries: Vec<PathBuf> = env::split_paths(&inherited).collect();
        for entry in &path_entries {
            if !entries.iter().any(|existing| existing == entry) {
                entries.push(entry.clone());
            }
        }
        let joined = env::join_paths(entries)
            .map_err(|_| invalid_value("path.append", "entries cannot be joined into PATH"))?;
        assignments.push(("PATH", joined));
    }

    let broker = resolve_broker(root.get("codex_account_broker"))?;
    let broker_source = match (broker, env_value(BROKER_ENV)) {
        (None, None) => Source::None,
        (None, Some(_)) => Source::Environment,
        (Some(_), Some(_)) => {
            warnings.push(format!(
                "{BROKER_ENV} overrides the config file's codex_account_broker table"
            ));
            Source::Environment
        }
        (Some(argv), None) => {
            assignments.push((BROKER_ENV, OsString::from(argv)));
            Source::File
        }
    };

    let retitle = resolve_retitle(root.get("retitle"))?;
    let retitle_source = match (retitle, env_value(RETITLE_ENV)) {
        (None, None) => Source::None,
        (None, Some(_)) => Source::Environment,
        (Some(_), Some(_)) => {
            warnings.push(format!(
                "{RETITLE_ENV} overrides the config file's retitle table"
            ));
            Source::Environment
        }
        (Some(raw), None) => {
            assignments.push((RETITLE_ENV, OsString::from(raw)));
            Source::File
        }
    };

    let file_profiles = resolve_file_launch_profiles(root.get("launch_profiles"))?;
    let launch_profiles = merge_launch_profiles(
        file_profiles,
        env_value(LAUNCH_PROFILES_ENV),
        &mut warnings,
        &mut assignments,
    )?;

    Ok(ResolvedServeConfig {
        summary: CheckSummary {
            config_schema_version: CONFIG_SCHEMA_VERSION,
            format,
            launch_profiles,
            retitle: SourceSummary {
                source: retitle_source,
            },
            codex_account_broker: SourceSummary {
                source: broker_source,
            },
            path: PathSummary {
                append: path_entries.len(),
            },
        },
        warnings,
        assignments,
    })
}

/// Whether `key` names a credential. In a schema-owned table a `*_env` or
/// `*_file` key is a reference and ends in a different word, so it passes. In a
/// free-form payload (`retitle.extra_body`) the same key is sent to the
/// provider verbatim, so the reference suffix is ignored there.
fn is_secret_key(key: &str, payload: bool) -> bool {
    let lower = key.to_ascii_lowercase();
    let mut words: Vec<&str> = lower
        .split(['_', '-', '.'])
        .filter(|word| !word.is_empty())
        .collect();
    if matches!(words.last(), Some(&"env") | Some(&"file")) {
        if !payload {
            return false;
        }
        while matches!(words.last(), Some(&"env") | Some(&"file")) {
            words.pop();
        }
    }
    words.iter().any(|word| SECRET_WORDS.contains(word))
        || words
            .last()
            .is_some_and(|word| FINAL_SECRET_WORDS.contains(word))
        || words
            .windows(2)
            .any(|pair| SECRET_PAIRS.contains(&(pair[0], pair[1])))
}

/// Free-form tables whose keys are forwarded to a provider rather than
/// interpreted by serve.
const PAYLOAD_KEYS: &[&str] = &["extra_body"];

fn reject_inline_secrets(value: &Value, prefix: &str, payload: bool) -> Result<(), CliError> {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = join_key(prefix, key);
                if is_secret_key(key, payload) {
                    return Err(keyed_error(
                        "serve-config-inline-secret",
                        &path,
                        "file",
                        "secrets must not be written inline; name an environment variable with a *_env key",
                    ));
                }
                let child_payload = payload || PAYLOAD_KEYS.contains(&key.as_str());
                reject_inline_secrets(child, &path, child_payload)?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                reject_inline_secrets(child, &format!("{prefix}[{index}]"), payload)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_unknown_keys(
    object: &Map<String, Value>,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), CliError> {
    match object.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(unknown_key(&join_key(prefix, key))),
        None => Ok(()),
    }
}

fn section<'a>(value: &'a Value, key: &str) -> Result<&'a Map<String, Value>, CliError> {
    value
        .as_object()
        .ok_or_else(|| invalid_value(key, "must be a table"))
}

fn resolve_path_append(value: Option<&Value>) -> Result<Vec<PathBuf>, CliError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let table = section(value, "path")?;
    reject_unknown_keys(table, PATH_KEYS, "path")?;
    let Some(append) = table.get("append") else {
        return Ok(Vec::new());
    };
    let items = append
        .as_array()
        .ok_or_else(|| invalid_value("path.append", "must be an array of absolute paths"))?;
    if items.len() > MAX_PATH_APPEND {
        return Err(invalid_value(
            "path.append",
            "at most 16 entries may be appended",
        ));
    }
    let mut entries = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let key = format!("path.append[{index}]");
        let entry = item.as_str().filter(|entry| {
            entry.starts_with('/')
                && entry.len() <= MAX_PATH_ENTRY_BYTES
                && !entry.contains(':')
                && !entry.chars().any(char::is_control)
        });
        let Some(entry) = entry else {
            return Err(invalid_value(
                &key,
                "entries must be absolute paths of at most 4096 bytes without ':' or control characters",
            ));
        };
        entries.push(PathBuf::from(entry));
    }
    Ok(entries)
}

fn resolve_broker(value: Option<&Value>) -> Result<Option<String>, CliError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let table = section(value, "codex_account_broker")?;
    reject_unknown_keys(table, BROKER_KEYS, "codex_account_broker")?;
    let Some(argv) = table.get("argv") else {
        return Err(invalid_value(
            "codex_account_broker.argv",
            "the broker table requires an argv array",
        ));
    };
    let argv: Option<Vec<String>> = argv.as_array().and_then(|items| {
        items
            .iter()
            .map(|item| item.as_str().map(str::to_string))
            .collect()
    });
    match argv {
        Some(argv) if crate::codex_account::valid_broker_argv(&argv) => {
            Ok(Some(Value::from(argv).to_string()))
        }
        _ => Err(invalid_value(
            "codex_account_broker.argv",
            "must be 1-16 non-empty strings of at most 4096 bytes",
        )),
    }
}

fn check_retitle_keys(
    table: &Map<String, Value>,
    prefix: &str,
    fallback: bool,
) -> Result<(), CliError> {
    reject_unknown_keys(table, RETITLE_KEYS, prefix)?;
    if fallback
        && let Some(key) = table
            .keys()
            .find(|key| RETITLE_ROOT_ONLY_KEYS.contains(&key.as_str()))
    {
        return Err(invalid_value(
            &join_key(prefix, key),
            "a fallback provider cannot carry root-owned retitle fields",
        ));
    }
    if let Some(context) = table.get("context") {
        let key = join_key(prefix, "context");
        reject_unknown_keys(section(context, &key)?, RETITLE_CONTEXT_KEYS, &key)?;
    }
    if let Some(extra_body) = table.get("extra_body") {
        section(extra_body, &join_key(prefix, "extra_body"))?;
    }
    Ok(())
}

fn resolve_retitle(value: Option<&Value>) -> Result<Option<String>, CliError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let table = section(value, "retitle")?;
    // The retitle parser is authoritative. The key lists only name the
    // offending key after it has refused the table, so a field added to the
    // parser is accepted here without a matching list edit.
    let raw = value.to_string();
    if crate::retitle::config_is_valid(&raw) {
        return Ok(Some(raw));
    }
    check_retitle_keys(table, "retitle", false)?;
    let fallback = match table.get("fallback") {
        Some(fallback) => {
            let fallback = section(fallback, "retitle.fallback")?;
            check_retitle_keys(fallback, "retitle.fallback", true)?;
            Some(fallback)
        }
        None => None,
    };
    // Attribute a failure to the fallback when that provider is invalid on its
    // own; otherwise the primary (or the shared timeout budget) is at fault.
    let key = if fallback.is_some_and(|fallback| {
        !crate::retitle::config_is_valid(&Value::from(fallback.clone()).to_string())
    }) {
        "retitle.fallback"
    } else {
        "retitle"
    };
    Err(invalid_value(
        key,
        "retitle provider configuration is invalid; see the retitle spec for provider fields and bounds",
    ))
}

fn resolve_file_launch_profiles(value: Option<&Value>) -> Result<Vec<(String, Value)>, CliError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid_value("launch_profiles", "must be an array of tables"))?;
    let mut profiles: Vec<(String, Value)> = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let key = format!("launch_profiles[{index}]");
        let table = section(item, &key)?;
        // The startup validator is authoritative; the key lists only name an
        // unknown key after it has refused the entry.
        let ids = match crate::serve::validate_launch_profiles_json(
            &Value::from(vec![item.clone()]).to_string(),
        ) {
            Ok(ids) => ids,
            Err(error) => {
                name_unknown_launch_profile_key(table, &key)?;
                return Err(invalid_value(&key, error.message()));
            }
        };
        let id = ids.into_iter().next().unwrap_or_default();
        if profiles.iter().any(|(existing, _)| *existing == id) {
            return Err(invalid_value(
                &format!("{key}.id"),
                "launch profile ids must be unique within the file",
            ));
        }
        profiles.push((id, item.clone()));
    }
    Ok(profiles)
}

fn name_unknown_launch_profile_key(table: &Map<String, Value>, key: &str) -> Result<(), CliError> {
    reject_unknown_keys(table, LAUNCH_PROFILE_KEYS, key)?;
    if let Some(history) = table.get("dsh_history").and_then(Value::as_object) {
        reject_unknown_keys(history, DSH_HISTORY_KEYS, &format!("{key}.dsh_history"))?;
    }
    Ok(())
}

fn merge_launch_profiles(
    file: Vec<(String, Value)>,
    environment: Option<String>,
    warnings: &mut Vec<String>,
    assignments: &mut Vec<(&'static str, OsString)>,
) -> Result<LaunchProfilesSummary, CliError> {
    let env_error = |reason: &str| {
        keyed_error(
            "serve-config-invalid-value",
            LAUNCH_PROFILES_ENV,
            "environment",
            reason,
        )
    };
    let Some(environment) = environment else {
        if file.is_empty() {
            return Ok(LaunchProfilesSummary {
                source: Source::None,
                ids: Vec::new(),
            });
        }
        let (ids, entries): (Vec<String>, Vec<Value>) = file.into_iter().unzip();
        let merged = Value::from(entries).to_string();
        crate::serve::validate_launch_profiles_json(&merged)
            .map_err(|error| invalid_value("launch_profiles", error.message()))?;
        assignments.push((LAUNCH_PROFILES_ENV, OsString::from(merged)));
        return Ok(LaunchProfilesSummary {
            source: Source::File,
            ids,
        });
    };
    let mut ids = crate::serve::validate_launch_profiles_json(&environment)
        .map_err(|error| env_error(error.message()))?;
    if file.is_empty() {
        return Ok(LaunchProfilesSummary {
            source: Source::Environment,
            ids,
        });
    }
    let mut entries: Vec<Value> = serde_json::from_str(&environment)
        .map_err(|_| env_error("launch profiles must be a valid JSON array"))?;
    let known: HashSet<String> = ids.iter().cloned().collect();
    for (index, (id, entry)) in file.into_iter().enumerate() {
        if known.contains(&id) {
            warnings.push(format!(
                "launch_profiles[{index}] ({id}) is shadowed by the same id in {LAUNCH_PROFILES_ENV}"
            ));
            continue;
        }
        ids.push(id);
        entries.push(entry);
    }
    if entries.len() > crate::serve::MAX_AGENT_LAUNCH_PROFILES {
        return Err(invalid_value(
            "launch_profiles",
            "the merged environment and file launch profiles exceed 16 entries",
        ));
    }
    let merged = Value::from(entries).to_string();
    crate::serve::validate_launch_profiles_json(&merged)
        .map_err(|error| invalid_value("launch_profiles", error.message()))?;
    assignments.push((LAUNCH_PROFILES_ENV, OsString::from(merged)));
    Ok(LaunchProfilesSummary {
        source: Source::Merged,
        ids,
    })
}

fn render_check(format: OutputFormat, resolved: &ResolvedServeConfig) -> i32 {
    match format {
        OutputFormat::Json => {
            let mut envelope = Envelope::success(
                schema_version_for(crate::BINARY, ENVELOPE_COMMAND, 1),
                &resolved.summary,
            );
            envelope.warnings = resolved.warnings.clone();
            print_json(&envelope)
        }
        OutputFormat::Text => {
            let summary = &resolved.summary;
            let source = |source: Source| match source {
                Source::None => "none",
                Source::File => "file",
                Source::Environment => "environment",
                Source::Merged => "environment+file",
            };
            println!("serve config ok ({CONFIG_SCHEMA_VERSION})");
            println!(
                "launch_profiles: {} [{}]",
                source(summary.launch_profiles.source),
                summary.launch_profiles.ids.join(", ")
            );
            println!("retitle: {}", source(summary.retitle.source));
            println!(
                "codex_account_broker: {}",
                source(summary.codex_account_broker.source)
            );
            println!("path.append: {}", summary.path.append);
            for warning in &resolved.warnings {
                let _ = writeln!(io::stderr(), "warning: {warning}");
            }
            exit::SUCCESS
        }
    }
}

fn render_error(format: OutputFormat, error: CliError) -> i32 {
    let error = error.into_inner();
    match format {
        OutputFormat::Json => {
            let mut envelope_error = EnvelopeError::new(error.code, error.message);
            if let Some(details) = error.details {
                envelope_error = envelope_error.with_details(details);
            }
            let envelope: Envelope<()> = Envelope::failure(
                schema_version_for(crate::BINARY, ENVELOPE_COMMAND, 1),
                envelope_error,
            );
            print_json(&envelope);
        }
        OutputFormat::Text => {
            let _ = writeln!(io::stderr(), "error: {}: {}", error.code, error.message);
        }
    }
    error.exit_code
}

fn print_json<T: Serialize>(value: &T) -> i32 {
    match serde_json::to_string(value) {
        Ok(serialized) => {
            println!("{serialized}");
            exit::SUCCESS
        }
        Err(_) => exit::RUNTIME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn document(extra: &str) -> Value {
        parse_toml(&format!(
            "schema_version = \"{CONFIG_SCHEMA_VERSION}\"\n{extra}"
        ))
        .expect("toml")
    }

    fn lookup_from(
        pairs: &'static [(&'static str, &'static str)],
    ) -> impl Fn(&str) -> Option<OsString> {
        move |key| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| OsString::from(value))
        }
    }

    fn assignment<'a>(resolved: &'a ResolvedServeConfig, key: &str) -> Option<&'a OsString> {
        resolved
            .environment_assignments()
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value)
    }

    #[test]
    fn path_entries_append_after_the_inherited_path_without_duplicates() {
        let resolved = resolve(
            DocumentFormat::Toml,
            &document("[path]\nappend = [\"/usr/bin\", \"/opt/tools/bin\"]\n"),
            &lookup_from(&[("PATH", "/home/linuxbrew/bin:/usr/bin")]),
        )
        .expect("resolve");

        assert_eq!(
            assignment(&resolved, "PATH"),
            Some(&OsString::from(
                "/home/linuxbrew/bin:/usr/bin:/opt/tools/bin"
            ))
        );
    }

    #[test]
    fn file_values_materialize_as_the_environment_variables_serve_reads() {
        let resolved = resolve(
            DocumentFormat::Toml,
            &document(
                "[codex_account_broker]\nargv = [\"/opt/broker\"]\n\
                 [retitle]\nprovider = \"openai_compatible\"\nbase_url = \"http://127.0.0.1:1/v1\"\nmodel = \"m\"\n\
                 [[launch_profiles]]\nid = \"a\"\nlabel = \"A\"\nagent = \"codex\"\nagent_bin = \"/opt/a\"\n",
            ),
            &lookup_from(&[]),
        )
        .expect("resolve");

        assert_eq!(
            assignment(&resolved, BROKER_ENV),
            Some(&OsString::from(r#"["/opt/broker"]"#))
        );
        let retitle: Value = serde_json::from_str(
            assignment(&resolved, RETITLE_ENV)
                .and_then(|value| value.to_str())
                .expect("retitle assignment"),
        )
        .expect("retitle json");
        assert_eq!(retitle["provider"], "openai_compatible");
        let profiles: Value = serde_json::from_str(
            assignment(&resolved, LAUNCH_PROFILES_ENV)
                .and_then(|value| value.to_str())
                .expect("profiles assignment"),
        )
        .expect("profiles json");
        assert_eq!(profiles[0]["id"], "a");
        assert_eq!(assignment(&resolved, "PATH"), None);
    }

    #[test]
    fn environment_values_are_never_reassigned() {
        let resolved = resolve(
            DocumentFormat::Toml,
            &document("[codex_account_broker]\nargv = [\"/opt/broker\"]\n"),
            &lookup_from(&[(BROKER_ENV, r#"["/opt/env-broker"]"#)]),
        )
        .expect("resolve");

        assert_eq!(assignment(&resolved, BROKER_ENV), None);
    }

    #[test]
    fn merged_profiles_keep_environment_entries_first() {
        let resolved = resolve(
            DocumentFormat::Toml,
            &document(
                "[[launch_profiles]]\nid = \"b\"\nlabel = \"B file\"\nagent = \"codex\"\nagent_bin = \"/opt/file-b\"\n\
                 [[launch_profiles]]\nid = \"c\"\nlabel = \"C\"\nagent = \"codex\"\nagent_bin = \"/opt/c\"\n",
            ),
            &lookup_from(&[(
                LAUNCH_PROFILES_ENV,
                r#"[{"id":"b","label":"B env","agent":"codex","agent_bin":"/opt/env-b"}]"#,
            )]),
        )
        .expect("resolve");

        let merged: Value = serde_json::from_str(
            assignment(&resolved, LAUNCH_PROFILES_ENV)
                .and_then(|value| value.to_str())
                .expect("profiles assignment"),
        )
        .expect("merged json");
        assert_eq!(merged[0]["label"], "B env");
        assert_eq!(merged[1]["id"], "c");
        assert_eq!(merged.as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn secret_shaped_keys_are_recognized_by_their_final_word() {
        for key in [
            "token",
            "api_key",
            "x-api-key",
            "client_secret",
            "access_token",
            "private-key",
            "Password",
            "apikey",
            "Authorization",
        ] {
            assert!(is_secret_key(key, false), "{key}");
            assert!(is_secret_key(key, true), "{key}");
        }
        for key in [
            "api_key_env",
            "token_file",
            "max_output_tokens",
            "stop_token_ids",
            "codex_bin",
            "keyring",
        ] {
            assert!(!is_secret_key(key, false), "{key}");
        }
        // Inside a forwarded payload a reference suffix is just part of the
        // key the provider receives, so it does not excuse a credential.
        for key in [
            "api_key_env",
            "token_file",
            "client_secret_env",
            "secret_key",
            "private_key_pem",
            "password_hash",
            "api_key_value",
            "client_secret_value",
        ] {
            assert!(is_secret_key(key, true), "{key}");
        }
        for key in ["stop_token_ids", "max_output_tokens", "profile"] {
            assert!(!is_secret_key(key, true), "{key}");
        }
    }

    /// The key lists only name keys for diagnostics; the startup validators
    /// decide. Every listed key must appear in a document those validators
    /// accept, so a list entry cannot outlive the struct field it mirrors.
    #[test]
    fn every_listed_key_is_accepted_by_the_authoritative_validators() {
        const PROFILE: &str = r#"
[[launch_profiles]]
id = "dsh"
label = "DSH"
agent = "hermes"
agent_bin = "/opt/dsh"
provider_config_dir = "/opt/dsh-home"
readiness_args = ["--version"]
auto_resume_supported = true
graceful_shutdown = "double-ctrl-c"
codex_usage_account = "main"
[launch_profiles.dsh_history]
command = "/opt/dsh-history"
root = "/opt/dsh-root"
compression = "zstd"
resume = "exact-id"
"#;
        const OPENAI_WITH_FALLBACK: &str = r#"
[retitle]
provider = "openai_compatible"
base_url = "http://127.0.0.1:1/v1"
model = "m"
api_key_env = "EXAMPLE_KEY"
timeout_ms = 20000
max_output_tokens = 100
temperature = 0.0
json_response = true
max_concurrency = 1
queue_size = 4
[retitle.extra_body]
stop_token_ids = [1]
[retitle.context]
max_chars = 12000
per_message_chars = 2000
recent_turns = 12
[retitle.fallback]
provider = "codex_subscription"
account = "main"
codex_bin = "/opt/codex"
model = "m"
reasoning_effort = "low"
timeout_ms = 20000
"#;
        const COMMAND: &str = "[retitle]\nprovider = \"command\"\nargv = [\"/opt/title\"]\n";
        const SUBSCRIPTION: &str = "[retitle]\nprovider = \"codex_subscription\"\n\
             account_selection = \"default_with_capacity\"\ncodex_bin = \"/opt/codex\"\n";
        let bodies = [
            format!("{PROFILE}{OPENAI_WITH_FALLBACK}"),
            COMMAND.to_string(),
            SUBSCRIPTION.to_string(),
        ];
        for body in &bodies {
            let resolved = resolve(DocumentFormat::Toml, &document(body), &lookup_from(&[]))
                .unwrap_or_else(|error| panic!("{}: {body}", error.message()));
            assert_eq!(resolved.summary.retitle.source, Source::File);
        }
        let all = bodies.join("\n");
        for key in LAUNCH_PROFILE_KEYS
            .iter()
            .chain(DSH_HISTORY_KEYS)
            .chain(RETITLE_KEYS)
            .chain(RETITLE_CONTEXT_KEYS)
        {
            assert!(
                all.contains(&format!("\n{key} = ")) || all.contains(&format!(".{key}]")),
                "{key} is listed but no accepted fixture uses it"
            );
        }
    }

    #[test]
    fn non_identifier_keys_are_not_echoed() {
        assert_eq!(
            join_key("path", "/home/user/secret"),
            "path.<non-identifier-key>"
        );
    }
}

//! Per-session Claude account binding over the provider-neutral account
//! broker contract (`agent-session.account-broker.v2`).
//!
//! Claude Code has no public way to inject credentials into a running process,
//! but it honors a separate `CLAUDE_CONFIG_DIR` per account. The host broker
//! therefore *materializes* an account directory and returns only its absolute
//! path; no token ever crosses the broker. Durable session state holds the
//! nickname, selection provenance, revision, and the directory path used for
//! the current runtime. HTTP projections never include the path.
//!
//! A bound account is kept for the life of the session: every resume
//! re-materializes the same nickname, and a requested switch is queued as a
//! durable next-account intent that applies on the next launch.

use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::account_broker::{self, BrokerArgvError, BrokerProcessError};
use crate::{CliError, SessionRecord};

pub(crate) const BROKER_SCHEMA_VERSION: &str = "agent-session.account-broker.v2";
pub(crate) const BINDING_SCHEMA_VERSION: &str = "agent-session.claude-account-binding.v1";
pub(crate) const VIEW_SCHEMA_VERSION: &str = "agent-session.claude-account.v1";
pub(crate) const NEXT_SCHEMA_VERSION: &str = "agent-session.claude-account-next.v1";
const PROVIDER: &str = "claude";
const BINDING_KEY: &str = "claude_account_binding";
const NEXT_KEY: &str = "claude_account_next";
const BROKER_ENV: &str = "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER";
const BROKER_TIMEOUT: Duration = Duration::from_secs(10);
const CREDENTIALS_FILE: &str = ".credentials.json";
const MAX_ACCOUNT_BYTES: usize = 64;
const MAX_PLAN_BYTES: usize = 128;
const MAX_CONFIG_DIR_BYTES: usize = 4096;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct DurableBinding {
    schema_version: String,
    selected_account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selection_source: Option<String>,
    revision: u64,
    state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    applied_runtime_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_dir: Option<String>,
    updated_at: String,
}

/// Durable intent to run the session on a different account from its next
/// launch. The applied binding stays authoritative until that launch succeeds.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
struct DurableNextAccount {
    schema_version: String,
    account: String,
    revision: u64,
    state: String,
    updated_at: String,
}

enum Decoded<T> {
    Absent,
    Valid(T),
    Invalid,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ClaudeAccountView {
    schema_version: &'static str,
    pub(crate) supported: bool,
    pub(crate) state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) selected_account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    selection_source: Option<String>,
    revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    applied_runtime_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next: Option<ClaudeNextAccountView>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ClaudeNextAccountView {
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    revision: u64,
    state: &'static str,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ClaudeAccountSummary {
    pub(crate) account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) plan: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct ClaudeAccountInventory {
    pub(crate) accounts: Vec<ClaudeAccountSummary>,
    pub(crate) selection_strategies: Vec<String>,
}

#[derive(Deserialize)]
struct BrokerListResponse {
    schema_version: String,
    provider: String,
    accounts: Vec<ClaudeAccountSummary>,
    #[serde(default)]
    selection_strategies: Vec<String>,
}

#[derive(Deserialize)]
struct BrokerSelectResponse {
    schema_version: String,
    provider: String,
    account: Option<String>,
}

#[derive(Deserialize)]
struct BrokerMaterializeResponse {
    schema_version: String,
    provider: String,
    account: String,
    config_dir: String,
}

pub(crate) fn broker_is_configured() -> bool {
    matches!(broker_argv(), Ok(Some(_)))
}

/// Public, path-free account projection. `None` keeps every non-Claude
/// session, and every Claude session on a daemon without a Claude broker,
/// byte-identical to the pre-binding contract.
pub(crate) fn view_for_record(record: &SessionRecord) -> Option<ClaudeAccountView> {
    if record.agent != PROVIDER {
        return None;
    }
    let binding = decode_binding(record);
    let next = decode_next(record);
    let configured = broker_is_configured();
    if !configured && matches!(binding, Decoded::Absent) && matches!(next, Decoded::Absent) {
        return None;
    }
    let next_view = match &next {
        Decoded::Absent => None,
        Decoded::Valid(next) => Some(ClaudeNextAccountView {
            account: Some(next.account.clone()),
            revision: next.revision,
            state: "queued",
        }),
        Decoded::Invalid => Some(ClaudeNextAccountView {
            account: None,
            revision: 0,
            state: "failed",
        }),
    };
    let mut view = ClaudeAccountView {
        schema_version: VIEW_SCHEMA_VERSION,
        supported: configured,
        state: "unbound",
        selected_account: None,
        selection_source: None,
        revision: 0,
        applied_runtime_id: None,
        failure_reason: None,
        next: next_view,
    };
    match binding {
        Decoded::Absent => {}
        Decoded::Invalid => {
            view.state = "failed";
            view.failure_reason = Some("binding_invalid".to_string());
        }
        Decoded::Valid(binding) => {
            view.state = "bound";
            view.selected_account = Some(binding.selected_account);
            view.selection_source = binding.selection_source;
            view.revision = binding.revision;
            view.applied_runtime_id = binding.applied_runtime_id;
        }
    }
    if !configured {
        view.state = "unsupported";
    }
    Some(view)
}

/// Account directory for the record's current runtime, if it was bound to it.
pub(crate) fn config_dir_for_runtime(record: &SessionRecord) -> Option<PathBuf> {
    if record.agent != PROVIDER {
        return None;
    }
    let Decoded::Valid(binding) = decode_binding(record) else {
        return None;
    };
    let launch_id = record.runtime.as_ref().map(|runtime| &runtime.launch_id)?;
    if binding.state != "bound" || binding.applied_runtime_id.as_ref() != Some(launch_id) {
        return None;
    }
    binding
        .config_dir
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

pub(crate) fn has_binding_state(record: &SessionRecord) -> bool {
    record.extra.contains_key(BINDING_KEY) || record.extra.contains_key(NEXT_KEY)
}

/// Resolves the account for a fresh daemon-created Claude session. An explicit
/// nickname wins; otherwise the broker's `current_default` is recorded as
/// `default_at_launch`. Without a configured broker the session stays on the
/// host login, exactly as before account binding existed.
pub(crate) fn resolve_initial_account(
    explicit: Option<String>,
) -> Result<Option<(String, &'static str)>, CliError> {
    if let Some(account) = explicit {
        validate_account(&account)?;
        if !broker_is_configured() {
            broker_argv()?;
            return Err(unsupported_error());
        }
        return Ok(Some((account, "explicit")));
    }
    if !broker_is_configured() {
        return Ok(None);
    }
    let inventory = list_accounts()?;
    if !inventory
        .selection_strategies
        .iter()
        .any(|strategy| strategy == "current_default")
    {
        return Ok(None);
    }
    Ok(Some((select_current_default()?, "default_at_launch")))
}

/// Binds a freshly created record to `account` before its first launch.
pub(crate) fn bind_initial(
    record: &mut SessionRecord,
    account: &str,
    selection_source: &str,
) -> Result<(), CliError> {
    validate_account(account)?;
    validate_selection_source(Some(selection_source))?;
    if record.agent != PROVIDER {
        return Err(agent_conflict_error());
    }
    let launch_id = current_launch_id(record)?;
    let config_dir = materialize(account)?;
    store_binding(
        record,
        &DurableBinding {
            schema_version: BINDING_SCHEMA_VERSION.to_string(),
            selected_account: account.to_string(),
            selection_source: Some(selection_source.to_string()),
            revision: 1,
            state: "bound".to_string(),
            applied_runtime_id: Some(launch_id),
            config_dir: Some(config_dir.to_string_lossy().into_owned()),
            updated_at: jiff::Timestamp::now().to_string(),
        },
    )
}

/// Re-materializes the session's account for a new runtime. A queued next
/// account is applied here; otherwise the bound account is kept. Fails closed
/// when a bound session can no longer reach its broker, so it never silently
/// falls back to the host login.
pub(crate) fn prepare_launch(record: &mut SessionRecord) -> Result<(), CliError> {
    if record.agent != PROVIDER || !has_binding_state(record) {
        return Ok(());
    }
    let binding = match decode_binding(record) {
        Decoded::Absent => None,
        Decoded::Valid(binding) => Some(binding),
        Decoded::Invalid => return Err(invalid_binding_error(record)),
    };
    let next = match decode_next(record) {
        Decoded::Absent => None,
        Decoded::Valid(next) => Some(next),
        Decoded::Invalid => return Err(invalid_binding_error(record)),
    };
    if broker_argv()?.is_none() {
        return Err(unsupported_error());
    }
    let launch_id = current_launch_id(record)?;
    let (account, selection_source, revision) = match (&binding, &next) {
        (Some(binding), Some(next)) if next.account != binding.selected_account => (
            next.account.clone(),
            Some("explicit".to_string()),
            binding.revision.saturating_add(1).max(next.revision),
        ),
        (None, Some(next)) => (next.account.clone(), Some("explicit".to_string()), 1),
        (Some(binding), _) => (
            binding.selected_account.clone(),
            binding.selection_source.clone(),
            binding.revision,
        ),
        (None, None) => return Ok(()),
    };
    let config_dir = materialize(&account)?;
    record.extra.remove(NEXT_KEY);
    store_binding(
        record,
        &DurableBinding {
            schema_version: BINDING_SCHEMA_VERSION.to_string(),
            selected_account: account,
            selection_source,
            revision,
            state: "bound".to_string(),
            applied_runtime_id: Some(launch_id),
            config_dir: Some(config_dir.to_string_lossy().into_owned()),
            updated_at: jiff::Timestamp::now().to_string(),
        },
    )
}

/// Durably queues `account` for the session's next launch. Requesting the
/// currently bound account cancels any queued intent instead.
pub(crate) fn queue_next(record: &mut SessionRecord, account: &str) -> Result<(), CliError> {
    validate_account(account)?;
    if record.agent != PROVIDER {
        return Err(agent_conflict_error());
    }
    if broker_argv()?.is_none() {
        return Err(unsupported_error());
    }
    let binding = match decode_binding(record) {
        Decoded::Absent => None,
        Decoded::Valid(binding) => Some(binding),
        // A malformed binding is repaired by an explicit account choice.
        Decoded::Invalid => {
            record.extra.remove(BINDING_KEY);
            None
        }
    };
    if binding
        .as_ref()
        .is_some_and(|binding| binding.selected_account == account)
    {
        record.extra.remove(NEXT_KEY);
        return Ok(());
    }
    if !list_accounts()?
        .accounts
        .iter()
        .any(|listed| listed.account == account)
    {
        return Err(CliError::usage(
            "claude-account-unknown",
            "Claude account is not configured in the account broker",
            None,
        ));
    }
    let prior_next_revision = match decode_next(record) {
        Decoded::Valid(next) => next.revision,
        Decoded::Absent | Decoded::Invalid => 0,
    };
    let revision = binding
        .as_ref()
        .map_or(1, |binding| binding.revision.saturating_add(1))
        .max(prior_next_revision.saturating_add(1));
    let next = DurableNextAccount {
        schema_version: NEXT_SCHEMA_VERSION.to_string(),
        account: account.to_string(),
        revision,
        state: "queued".to_string(),
        updated_at: jiff::Timestamp::now().to_string(),
    };
    let value = serde_json::to_value(&next).map_err(|_| encode_error(record))?;
    record.extra.insert(NEXT_KEY.to_string(), value);
    Ok(())
}

pub(crate) fn has_queued_next(record: &SessionRecord) -> bool {
    matches!(decode_next(record), Decoded::Valid(_))
}

/// Materializes the queued next account before a running session is stopped
/// for it, so a broker refusal or an unsafe directory never costs the user a
/// running session. The refusal is typed and carries the broker's code.
pub(crate) fn preflight_next(record: &SessionRecord) -> Result<(), CliError> {
    let Decoded::Valid(next) = decode_next(record) else {
        return Ok(());
    };
    materialize(&next.account).map(|_| ()).map_err(|error| {
        let mut details = json!({ "id": record.id, "cause": error.code() });
        if let Some(reason) = error.details().and_then(|value| value.get("reason")) {
            details["reason"] = reason.clone();
        }
        CliError::data(
            "claude-account-switch-refused",
            "the next Claude account could not be prepared; the running session was left unchanged",
            Some(details),
        )
    })
}

pub(crate) fn list_accounts() -> Result<ClaudeAccountInventory, CliError> {
    let value = run_broker(&["list", "--provider", PROVIDER, "--format", "json"])?;
    let response: BrokerListResponse =
        serde_json::from_value(value).map_err(|_| invalid_response("an invalid account list"))?;
    ensure_envelope(&response.schema_version, &response.provider)?;
    let mut accounts = Vec::with_capacity(response.accounts.len());
    for mut account in response.accounts {
        validate_account(&account.account)
            .map_err(|_| invalid_response("an unsafe account nickname"))?;
        validate_public_string(&account.label, MAX_ACCOUNT_BYTES)?;
        validate_public_string(&account.plan, MAX_PLAN_BYTES)?;
        if accounts
            .iter()
            .any(|seen: &ClaudeAccountSummary| seen.account == account.account)
        {
            return Err(invalid_response("duplicate account nicknames"));
        }
        account.label = account.label.filter(|value| !value.trim().is_empty());
        account.plan = account.plan.filter(|value| !value.trim().is_empty());
        accounts.push(account);
    }
    let selection_strategies = response
        .selection_strategies
        .into_iter()
        .filter(|strategy| strategy == "current_default")
        .collect();
    Ok(ClaudeAccountInventory {
        accounts,
        selection_strategies,
    })
}

fn select_current_default() -> Result<String, CliError> {
    let value = run_broker(&[
        "select",
        "--provider",
        PROVIDER,
        "--strategy",
        "current_default",
        "--format",
        "json",
    ])?;
    let response: BrokerSelectResponse = serde_json::from_value(value)
        .map_err(|_| invalid_response("an invalid account selection"))?;
    ensure_envelope(&response.schema_version, &response.provider)?;
    let account = response
        .account
        .ok_or_else(|| invalid_response("an invalid account selection"))?;
    validate_account(&account).map_err(|_| invalid_response("an invalid account selection"))?;
    Ok(account)
}

/// Asks the broker to materialize `account` and validates the returned
/// directory before any Claude process may run in it.
pub(crate) fn materialize(account: &str) -> Result<PathBuf, CliError> {
    validate_account(account)?;
    let value = run_broker(&[
        "materialize",
        "--provider",
        PROVIDER,
        "--account",
        account,
        "--format",
        "json",
    ])?;
    let response: BrokerMaterializeResponse = serde_json::from_value(value)
        .map_err(|_| invalid_response("an invalid account directory"))?;
    ensure_envelope(&response.schema_version, &response.provider)?;
    if response.account != account {
        return Err(invalid_response("a mismatched account directory"));
    }
    if response.config_dir.is_empty()
        || response.config_dir.len() > MAX_CONFIG_DIR_BYTES
        || response.config_dir.contains(['\0', '\n', '\r'])
    {
        return Err(invalid_response("an invalid account directory"));
    }
    let config_dir = PathBuf::from(response.config_dir);
    validate_config_dir(&config_dir, current_euid())?;
    Ok(config_dir)
}

/// Requires an absolute, normalized, real directory owned by `uid` that is not
/// world-writable and holds a regular (non-symlink) `.credentials.json` owned
/// by the same user. Claude Code opens that file with `O_NOFOLLOW`.
pub(crate) fn validate_config_dir(path: &Path, uid: u32) -> Result<(), CliError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(unsafe_dir_error("not_absolute"));
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| unsafe_dir_error("missing"))?;
    if !metadata.file_type().is_dir() {
        return Err(unsafe_dir_error("not_directory"));
    }
    if metadata.uid() != uid {
        return Err(unsafe_dir_error("not_owned"));
    }
    if metadata.mode() & 0o002 != 0 {
        return Err(unsafe_dir_error("world_writable"));
    }
    let credentials = fs::symlink_metadata(path.join(CREDENTIALS_FILE))
        .map_err(|_| unsafe_dir_error("credentials_missing"))?;
    if !credentials.file_type().is_file() {
        return Err(unsafe_dir_error("credentials_not_regular"));
    }
    if credentials.uid() != uid {
        return Err(unsafe_dir_error("credentials_not_owned"));
    }
    Ok(())
}

pub(crate) fn validate_account(account: &str) -> Result<(), CliError> {
    if account.is_empty()
        || account.len() > MAX_ACCOUNT_BYTES
        || !account.as_bytes()[0].is_ascii_alphanumeric()
        || !account
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(CliError::usage(
            "invalid-claude-account",
            "Claude account must be a short configured nickname",
            None,
        ));
    }
    Ok(())
}

fn validate_selection_source(source: Option<&str>) -> Result<(), CliError> {
    if source.is_none_or(|source| matches!(source, "default_at_launch" | "explicit")) {
        return Ok(());
    }
    Err(CliError::data(
        "claude-account-selection-source-invalid",
        "Claude account selection source is invalid",
        None,
    ))
}

fn validate_public_string(value: &Option<String>, max: usize) -> Result<(), CliError> {
    if value
        .as_ref()
        .is_some_and(|value| value.len() > max || value.contains(['\n', '\r', '\0']))
    {
        return Err(invalid_response("invalid public metadata"));
    }
    Ok(())
}

fn decode_binding(record: &SessionRecord) -> Decoded<DurableBinding> {
    let Some(value) = record.extra.get(BINDING_KEY).cloned() else {
        return Decoded::Absent;
    };
    match serde_json::from_value::<DurableBinding>(value) {
        Ok(binding)
            if binding.schema_version == BINDING_SCHEMA_VERSION
                && validate_account(&binding.selected_account).is_ok()
                && validate_selection_source(binding.selection_source.as_deref()).is_ok()
                && binding.revision > 0
                && binding.state == "bound" =>
        {
            Decoded::Valid(binding)
        }
        Ok(_) | Err(_) => Decoded::Invalid,
    }
}

fn decode_next(record: &SessionRecord) -> Decoded<DurableNextAccount> {
    let Some(value) = record.extra.get(NEXT_KEY).cloned() else {
        return Decoded::Absent;
    };
    match serde_json::from_value::<DurableNextAccount>(value) {
        Ok(next)
            if next.schema_version == NEXT_SCHEMA_VERSION
                && validate_account(&next.account).is_ok()
                && next.revision > 0
                && next.state == "queued" =>
        {
            Decoded::Valid(next)
        }
        Ok(_) | Err(_) => Decoded::Invalid,
    }
}

fn store_binding(record: &mut SessionRecord, binding: &DurableBinding) -> Result<(), CliError> {
    let value = serde_json::to_value(binding).map_err(|_| encode_error(record))?;
    record.extra.insert(BINDING_KEY.to_string(), value);
    Ok(())
}

fn current_launch_id(record: &SessionRecord) -> Result<String, CliError> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .filter(|launch_id| !launch_id.is_empty())
        .ok_or_else(|| {
            CliError::runtime(
                "claude-account-runtime-missing",
                "Claude account binding requires a session runtime",
                Some(json!({ "id": record.id })),
            )
        })
}

fn current_euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

fn broker_argv() -> Result<Option<Vec<String>>, CliError> {
    account_broker::parse_argv(env::var(BROKER_ENV).ok()).map_err(|error| match error {
        BrokerArgvError::NotJsonArgv => broker_error(
            "claude-account-broker-invalid-config",
            "Claude account broker configuration must be a JSON argv array",
        ),
        BrokerArgvError::Invalid => broker_error(
            "claude-account-broker-invalid-config",
            "Claude account broker configuration is invalid",
        ),
    })
}

fn run_broker(args: &[&str]) -> Result<Value, CliError> {
    let argv = broker_argv()?.ok_or_else(unsupported_error)?;
    account_broker::run(&argv, args, BROKER_TIMEOUT).map_err(|error| match error {
        BrokerProcessError::SpawnFailed
        | BrokerProcessError::StdoutUnavailable
        | BrokerProcessError::StderrUnavailable => broker_error(
            "claude-account-broker-unavailable",
            "Claude account broker could not be started",
        ),
        BrokerProcessError::WaitFailed => broker_error(
            "claude-account-broker-failed",
            "Claude account broker failed",
        ),
        BrokerProcessError::Rejected => broker_error(
            "claude-account-broker-rejected",
            "Claude account broker rejected the request",
        ),
        BrokerProcessError::OutputTooLarge | BrokerProcessError::MalformedJson => broker_error(
            "claude-account-broker-invalid-response",
            "Claude account broker returned malformed output",
        ),
        BrokerProcessError::Timeout => broker_error(
            "claude-account-broker-timeout",
            "Claude account broker timed out",
        ),
    })
}

fn ensure_envelope(schema: &str, provider: &str) -> Result<(), CliError> {
    if schema == BROKER_SCHEMA_VERSION && provider == PROVIDER {
        Ok(())
    } else {
        Err(invalid_response("an unsupported schema or provider"))
    }
}

fn invalid_response(what: &str) -> CliError {
    CliError::runtime(
        "claude-account-broker-invalid-response",
        format!("Claude account broker returned {what}"),
        None,
    )
}

fn unsafe_dir_error(reason: &'static str) -> CliError {
    CliError::runtime(
        "claude-account-dir-unsafe",
        "Claude account broker returned an unsafe account directory",
        Some(json!({ "reason": reason })),
    )
}

fn unsupported_error() -> CliError {
    CliError::data(
        "claude-account-unsupported",
        "Claude account binding is not configured for this daemon",
        None,
    )
}

pub(crate) fn agent_conflict_error() -> CliError {
    CliError::usage(
        "claude-account-agent-conflict",
        "claude_account is supported only for Claude sessions",
        None,
    )
}

fn invalid_binding_error(record: &SessionRecord) -> CliError {
    CliError::data(
        "claude-account-binding-invalid",
        "Claude account binding state is invalid and must be repaired explicitly",
        Some(json!({ "id": record.id })),
    )
}

fn encode_error(record: &SessionRecord) -> CliError {
    CliError::runtime(
        "claude-account-binding-encode-failed",
        "failed to encode Claude account binding state",
        Some(json!({ "id": record.id })),
    )
}

fn broker_error(code: &'static str, message: &'static str) -> CliError {
    CliError::runtime(code, message, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn account_dir(root: &Path) -> PathBuf {
        let dir = root.join("alpha");
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(dir.join(CREDENTIALS_FILE), "{}").unwrap();
        dir
    }

    fn reason(result: Result<(), CliError>) -> String {
        let error = result.unwrap_err();
        assert_eq!(error.code(), "claude-account-dir-unsafe");
        error
            .details()
            .and_then(|details| details["reason"].as_str())
            .map(str::to_string)
            .expect("unsafe directory reason")
    }

    #[test]
    fn config_dir_validation_accepts_a_private_account_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = account_dir(tmp.path());
        validate_config_dir(&dir, current_euid()).unwrap();
    }

    #[test]
    fn config_dir_validation_rejects_unsafe_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = account_dir(tmp.path());
        let uid = current_euid();

        assert_eq!(
            reason(validate_config_dir(Path::new("relative/alpha"), uid)),
            "not_absolute"
        );
        assert_eq!(
            reason(validate_config_dir(&tmp.path().join("alpha/../alpha"), uid)),
            "not_absolute"
        );
        assert_eq!(
            reason(validate_config_dir(&tmp.path().join("missing"), uid)),
            "missing"
        );
        let linked = tmp.path().join("linked");
        symlink(&dir, &linked).unwrap();
        assert_eq!(reason(validate_config_dir(&linked, uid)), "not_directory");
        assert_eq!(
            reason(validate_config_dir(&dir, uid.wrapping_add(1))),
            "not_owned"
        );

        fs::set_permissions(&dir, fs::Permissions::from_mode(0o703)).unwrap();
        assert_eq!(reason(validate_config_dir(&dir, uid)), "world_writable");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();

        let credentials = dir.join(CREDENTIALS_FILE);
        fs::remove_file(&credentials).unwrap();
        assert_eq!(
            reason(validate_config_dir(&dir, uid)),
            "credentials_missing"
        );
        let elsewhere = tmp.path().join("elsewhere.json");
        fs::write(&elsewhere, "{}").unwrap();
        symlink(&elsewhere, &credentials).unwrap();
        assert_eq!(
            reason(validate_config_dir(&dir, uid)),
            "credentials_not_regular"
        );
    }

    #[test]
    fn account_nicknames_are_short_and_shell_free() {
        validate_account("alpha.team-1_x").unwrap();
        for invalid in [
            "",
            "has space",
            "semi;colon",
            "../up",
            "-x",
            "--format",
            ".",
            "..",
            "_hidden",
            &"a".repeat(65),
        ] {
            assert_eq!(
                validate_account(invalid).unwrap_err().code(),
                "invalid-claude-account"
            );
        }
    }
}

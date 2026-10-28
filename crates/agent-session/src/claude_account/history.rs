//! Read-only account-root authority for a deleted managed conversation source.
use super::*;
use crate::start_resume::refusal;

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HistoryScope {
    Account,
    Shared,
}

pub(crate) struct HistoryRoot {
    pub(crate) config_dir: PathBuf,
    pub(crate) history_scope: HistoryScope,
}

#[derive(Deserialize)]
struct HistoryRootResponse {
    schema_version: String,
    provider: String,
    account: String,
    config_dir: String,
    history_scope: HistoryScope,
}

/// Resolve an existing account root without selecting defaults or preparing
/// credentials. Older brokers fail closed; materialize is never a fallback.
pub(crate) fn root(account: &str) -> Result<HistoryRoot, CliError> {
    validate_account(account).map_err(|_| refusal("resume-account-mismatch"))?;
    let accounts = list_accounts().map_err(|_| refusal("resume-unsupported"))?;
    if !accounts
        .accounts
        .iter()
        .any(|entry| entry.account == account)
    {
        return Err(refusal("resume-account-mismatch"));
    }
    let value = BROKER
        .call("history-root", &["--account", account], BROKER_TIMEOUT)
        .map_err(|_| refusal("resume-unsupported"))?;
    let response: HistoryRootResponse = BROKER
        .decode(
            value,
            "an invalid history root",
            |response: &HistoryRootResponse| {
                (
                    response.schema_version.as_str(),
                    Some(response.provider.as_str()),
                )
            },
        )
        .map_err(|_| refusal("resume-unsupported"))?;
    if response.account != account
        || response.config_dir.is_empty()
        || response.config_dir.len() > MAX_CONFIG_DIR_BYTES
        || response.config_dir.contains(['\0', '\n', '\r'])
    {
        return Err(refusal("resume-account-mismatch"));
    }
    let path = PathBuf::from(response.config_dir);
    validate_config_dir(&path, current_euid()).map_err(|_| refusal("resume-account-mismatch"))?;
    Ok(HistoryRoot {
        config_dir: path,
        history_scope: response.history_scope,
    })
}

/// Retained bindings already carry account authority. Describe whether their
/// configured projects directory redirects to shared history, without a new
/// broker capability requirement for existing retained-source recovery.
pub(crate) fn observed_scope(config_dir: &Path) -> Result<HistoryScope, CliError> {
    let account_root =
        fs::canonicalize(config_dir).map_err(|_| refusal("resume-account-mismatch"))?;
    let projects = fs::canonicalize(config_dir.join("projects"))
        .map_err(|_| refusal("resume-history-missing"))?;
    Ok(if projects.starts_with(account_root) {
        HistoryScope::Account
    } else {
        HistoryScope::Shared
    })
}

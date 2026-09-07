//! Daemon-owned session retitling from bounded provider transcript context.
//!
//! Provider prompts and model output are transient. The session record retains
//! only hashes, request state, and stable diagnostic codes so it never becomes
//! a second transcript or a title-model log.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::env;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::activity;
use crate::provider_history::{
    HistoryCatalog, HistoryError, HistoryMessage, HistoryMessageDirection,
};
use crate::{
    CliContext, CliError, SessionRecord, SessionTitleState, SessionTitleTopicSource,
    canonicalize_structured_title_pair, load_session_record, session_agent_profile,
};

pub(crate) const CAPABILITY: &str = "agent-session.session-retitle.v2";
pub(crate) const REQUEST_SCHEMA: &str = "agent-session.session-retitle.request.v2";
pub(crate) const RESPONSE_SCHEMA: &str = "agent-session.session-retitle.v2";
pub(crate) const READINESS_SCHEMA: &str = "agent-session.session-retitle.readiness.v2";
pub(crate) const CONFIG_ENV: &str = "AGENT_SESSION_RETITLE_CONFIG";
const MARKER_KEY: &str = "session_retitle_v2";
const MARKER_SCHEMA: &str = "agent-session.session-retitle-state.v2";
const MAX_RECEIPTS: usize = 32;
const MAX_CONFIG_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_TIMEOUT_MS: u64 = 45_000;
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 300;
const DEFAULT_MAX_CONCURRENCY: usize = 1;
const DEFAULT_QUEUE_SIZE: usize = 8;
const DEFAULT_MAX_CONTEXT_CHARS: usize = 12_000;
const DEFAULT_PER_MESSAGE_CHARS: usize = 2_000;
const DEFAULT_RECENT_TURNS: usize = 12;
const MAX_CONTEXT_CHARS: usize = 64 * 1024;
const MAX_PER_MESSAGE_CHARS: usize = 8 * 1024;
const MAX_RECENT_TURNS: usize = 32;

pub(crate) const ERROR_CONTRACT: &[(&str, &str, &str)] = &[
    (
        "invalid-retitle-request",
        "refresh_session",
        "replace_request",
    ),
    (
        "retitle-provider-not-configured",
        "configure_provider",
        "configure_provider",
    ),
    (
        "retitle-config-invalid",
        "configure_provider",
        "replace_configuration",
    ),
    (
        "retitle-queue-saturated",
        "wait_and_retry",
        "bounded_backoff",
    ),
    (
        "retitle-context-unavailable",
        "wait_for_context",
        "retry_after_transcript_catchup",
    ),
    (
        "retitle-account-missing",
        "select_account",
        "refresh_account",
    ),
    ("retitle-api-key-missing", "set_api_key", "configure_secret"),
    ("retitle-provider-timeout", "retry", "retry_request"),
    ("retitle-provider-unavailable", "retry", "retry_request"),
    (
        "retitle-provider-rate-limited",
        "wait_and_retry",
        "bounded_backoff",
    ),
    (
        "retitle-provider-quota-exceeded",
        "wait_and_retry",
        "wait_for_quota",
    ),
    (
        "retitle-provider-malformed-response",
        "inspect_provider",
        "repair_provider_output",
    ),
    (
        "idempotency-key-reused",
        "refresh_session",
        "replace_idempotency_key",
    ),
    (
        "session-incarnation-conflict",
        "refresh_session",
        "refresh_session",
    ),
    (
        "title-revision-conflict",
        "refresh_session",
        "refresh_session",
    ),
    (
        "retitle-turn-conflict",
        "refresh_session",
        "process_newest_turn",
    ),
    (
        "retitle-state-conflict",
        "refresh_session",
        "refresh_session",
    ),
    ("retitle-worker-failed", "retry", "retry_request"),
    ("title-revision-overflow", "none", "none"),
];

pub(crate) fn is_public_error_code(code: &str) -> bool {
    code == "session-not-found" || ERROR_CONTRACT.iter().any(|entry| entry.0 == code)
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RetitleTrigger {
    Initial,
    Prompt,
    CompletionRecovery,
    Manual,
}

impl RetitleTrigger {
    pub(crate) fn is_automatic(self) -> bool {
        self != Self::Manual
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetitleRequest {
    pub(crate) schema_version: String,
    pub(crate) trigger: RetitleTrigger,
    pub(crate) idempotency_key: String,
    pub(crate) expected_session_incarnation: String,
    pub(crate) expected_title_revision: u64,
    #[serde(default)]
    pub(crate) expected_activity_revision: Option<u64>,
    #[serde(default)]
    pub(crate) expected_provider_turn_id: Option<String>,
}

impl RetitleRequest {
    pub(crate) fn validate(&self) -> Result<(), CliError> {
        if self.schema_version != REQUEST_SCHEMA {
            return Err(invalid_request(
                "schema_version",
                "unsupported request schema",
            ));
        }
        if !(8..=128).contains(&self.idempotency_key.len())
            || !self.idempotency_key.is_ascii()
            || self
                .idempotency_key
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte == b' ')
        {
            return Err(invalid_request(
                "idempotency_key",
                "idempotency_key must be 8-128 printable non-space ASCII bytes",
            ));
        }
        if self.expected_session_incarnation.trim().is_empty()
            || self.expected_session_incarnation.len() > 128
        {
            return Err(invalid_request(
                "expected_session_incarnation",
                "expected_session_incarnation is invalid",
            ));
        }
        match self.trigger {
            RetitleTrigger::Manual
                if self.expected_activity_revision.is_none()
                    && self.expected_provider_turn_id.is_none() => {}
            RetitleTrigger::Manual => {
                return Err(invalid_request(
                    "trigger",
                    "manual retitle must not carry provider-turn fences",
                ));
            }
            _ if self.expected_activity_revision.is_some()
                && self
                    .expected_provider_turn_id
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty() && value.len() <= 256) => {}
            _ => {
                return Err(invalid_request(
                    "expected_provider_turn_id",
                    "automatic retitle requires activity revision and provider turn fences",
                ));
            }
        }
        Ok(())
    }
}

fn invalid_request(field: &'static str, message: &'static str) -> CliError {
    CliError::usage(
        "invalid-retitle-request",
        message,
        Some(json!({
            "field": field,
            "retryable": false,
            "next_action": "refresh_session",
            "recovery": {"strategy":"replace_request", "safe_to_retry":false}
        })),
    )
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextConfig {
    #[serde(default = "default_max_context_chars")]
    max_chars: usize,
    #[serde(default = "default_per_message_chars")]
    per_message_chars: usize,
    #[serde(default = "default_recent_turns")]
    recent_turns: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_chars: default_max_context_chars(),
            per_message_chars: default_per_message_chars(),
            recent_turns: default_recent_turns(),
        }
    }
}

fn default_max_context_chars() -> usize {
    DEFAULT_MAX_CONTEXT_CHARS
}
fn default_per_message_chars() -> usize {
    DEFAULT_PER_MESSAGE_CHARS
}
fn default_recent_turns() -> usize {
    DEFAULT_RECENT_TURNS
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RetitleConfig {
    provider: String,
    #[serde(default)]
    account: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    codex_bin: Option<PathBuf>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key_env: Option<String>,
    #[serde(default)]
    argv: Option<Vec<String>>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_max_output_tokens")]
    max_output_tokens: u32,
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    extra_body: Map<String, Value>,
    #[serde(default)]
    json_response: bool,
    #[serde(default = "default_max_concurrency")]
    max_concurrency: usize,
    #[serde(default = "default_queue_size")]
    queue_size: usize,
    #[serde(default)]
    context: ContextConfig,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}
fn default_max_output_tokens() -> u32 {
    DEFAULT_MAX_OUTPUT_TOKENS
}
fn default_max_concurrency() -> usize {
    DEFAULT_MAX_CONCURRENCY
}
fn default_queue_size() -> usize {
    DEFAULT_QUEUE_SIZE
}

impl RetitleConfig {
    fn parse(raw: &str) -> Result<Self, &'static str> {
        if raw.len() > MAX_CONFIG_BYTES {
            return Err("config_invalid");
        }
        let config: Self = serde_json::from_str(raw).map_err(|_| "config_invalid")?;
        if !(1_000..=120_000).contains(&config.timeout_ms)
            || !(1..=4_096).contains(&config.max_output_tokens)
            || !(1..=8).contains(&config.max_concurrency)
            || config.queue_size > 64
            || !(1_000..=MAX_CONTEXT_CHARS).contains(&config.context.max_chars)
            || !(128..=MAX_PER_MESSAGE_CHARS).contains(&config.context.per_message_chars)
            || !(1..=MAX_RECENT_TURNS).contains(&config.context.recent_turns)
            || config
                .model
                .as_deref()
                .is_some_and(|value| !safe_label(value, 128))
        {
            return Err("config_invalid");
        }
        match config.provider.as_str() {
            "codex_subscription"
                if config
                    .account
                    .as_deref()
                    .is_some_and(|value| safe_label(value, 64))
                    && config.codex_bin.as_deref().is_some_and(Path::is_absolute)
                    && config.base_url.is_none()
                    && config.api_key_env.is_none()
                    && config.argv.is_none() => {}
            "openai_compatible"
                if config.base_url.as_deref().is_some_and(valid_http_base_url)
                    && config
                        .model
                        .as_deref()
                        .is_some_and(|value| safe_label(value, 128))
                    && config.account.is_none()
                    && config.codex_bin.is_none()
                    && config.argv.is_none()
                    && config.api_key_env.as_deref().is_none_or(valid_env_name) => {}
            "command"
                if config.argv.as_ref().is_some_and(|argv| valid_argv(argv))
                    && config.account.is_none()
                    && config.codex_bin.is_none()
                    && config.base_url.is_none()
                    && config.api_key_env.is_none() => {}
            _ => return Err("config_invalid"),
        }
        Ok(config)
    }

    fn kind(&self) -> &'static str {
        match self.provider.as_str() {
            "codex_subscription" => "codex_subscription",
            "openai_compatible" => "openai_compatible",
            _ => "command",
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

fn safe_label(value: &str, max: usize) -> bool {
    !value.trim().is_empty()
        && value.len() <= max
        && !value.chars().any(char::is_control)
        && !value.contains(['/', '\\'])
}

fn valid_http_base_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
}

fn valid_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && !value.as_bytes()[0].is_ascii_digit()
}

fn valid_argv(argv: &[String]) -> bool {
    !argv.is_empty()
        && argv.len() <= 16
        && argv
            .iter()
            .all(|arg| !arg.is_empty() && arg.len() <= 4096 && !arg.contains('\0'))
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct RetitleReadiness {
    schema_version: &'static str,
    capability: &'static str,
    status: &'static str,
    reason_code: &'static str,
    next_action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    provider_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    context_capabilities: BTreeMap<&'static str, &'static str>,
}

impl RetitleReadiness {
    fn unavailable(reason_code: &'static str, next_action: &'static str) -> Self {
        Self {
            schema_version: READINESS_SCHEMA,
            capability: CAPABILITY,
            status: "unavailable",
            reason_code,
            next_action,
            provider_kind: None,
            model_label: None,
            account: None,
            plan: None,
            context_capabilities: context_capabilities(),
        }
    }
}

fn context_capabilities() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        ("claude", "provider_transcript"),
        ("codex", "provider_transcript"),
        ("hermes", "unavailable"),
    ])
}

pub(crate) struct RetitleService {
    config: Result<Option<Arc<RetitleConfig>>, &'static str>,
    semaphore: Arc<Semaphore>,
    waiting: AtomicUsize,
    session_gates: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl RetitleService {
    pub(crate) fn from_environment() -> Self {
        let config = match env::var(CONFIG_ENV) {
            Ok(raw) if !raw.trim().is_empty() => RetitleConfig::parse(&raw).map(Arc::new).map(Some),
            Ok(_) | Err(env::VarError::NotPresent) => Ok(None),
            Err(_) => Err("config_invalid"),
        };
        let permits = config
            .as_ref()
            .ok()
            .and_then(|config| config.as_ref())
            .map_or(DEFAULT_MAX_CONCURRENCY, |config| config.max_concurrency);
        Self {
            config,
            semaphore: Arc::new(Semaphore::new(permits)),
            waiting: AtomicUsize::new(0),
            session_gates: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn readiness(&self) -> RetitleReadiness {
        let config = match &self.config {
            Ok(Some(config)) => config,
            Ok(None) => {
                return RetitleReadiness::unavailable(
                    "provider_not_configured",
                    "configure_provider",
                );
            }
            Err(_) => return RetitleReadiness::unavailable("config_invalid", "configure_provider"),
        };
        let mut readiness = RetitleReadiness {
            schema_version: READINESS_SCHEMA,
            capability: CAPABILITY,
            status: "ready",
            reason_code: "ready",
            next_action: "none",
            provider_kind: Some(config.kind()),
            model_label: config.model.clone(),
            account: None,
            plan: None,
            context_capabilities: context_capabilities(),
        };
        match config.kind() {
            "codex_subscription" => {
                if !crate::codex_account::broker_is_configured() {
                    return RetitleReadiness::unavailable(
                        "account_broker_unavailable",
                        "configure_account_broker",
                    );
                }
                let Some(account) = config.account.as_deref() else {
                    return RetitleReadiness::unavailable("account_missing", "select_account");
                };
                let accounts = match crate::codex_account::list_accounts() {
                    Ok(accounts) => accounts,
                    Err(_) => {
                        return RetitleReadiness::unavailable(
                            "account_broker_unavailable",
                            "configure_account_broker",
                        );
                    }
                };
                let Some(summary) = accounts.into_iter().find(|item| item.account == account)
                else {
                    return RetitleReadiness::unavailable("account_missing", "select_account");
                };
                if !executable(config.codex_bin.as_deref().expect("validated")) {
                    return RetitleReadiness::unavailable(
                        "provider_command_unavailable",
                        "install_provider_command",
                    );
                }
                readiness.account = Some(summary.account);
                readiness.plan = summary.plan;
            }
            "openai_compatible" => {
                if let Some(name) = config.api_key_env.as_deref()
                    && env::var(name)
                        .ok()
                        .is_none_or(|value| value.trim().is_empty())
                {
                    return RetitleReadiness::unavailable("api_key_missing", "set_api_key");
                }
            }
            "command" => {
                if !command_executable(config.argv.as_ref().expect("validated")) {
                    return RetitleReadiness::unavailable(
                        "provider_command_unavailable",
                        "install_provider_command",
                    );
                }
                readiness.status = "degraded";
                readiness.reason_code = "legacy_command_provider";
                readiness.next_action = "migrate_provider";
            }
            _ => unreachable!(),
        }
        readiness
    }

    pub(crate) fn configured_provider_kind(&self) -> Option<&'static str> {
        self.config
            .as_ref()
            .ok()
            .and_then(|config| config.as_ref())
            .map(|config| config.kind())
    }

    fn config(&self) -> Result<Arc<RetitleConfig>, CliError> {
        match &self.config {
            Ok(Some(config)) => Ok(config.clone()),
            Ok(None) => Err(retitle_error(
                "retitle-provider-not-configured",
                "title provider is not configured",
                false,
                "configure_provider",
                "configure_provider",
            )),
            Err(_) => Err(retitle_error(
                "retitle-config-invalid",
                "title provider configuration is invalid",
                false,
                "configure_provider",
                "replace_configuration",
            )),
        }
    }

    pub(crate) async fn acquire(&self) -> Result<RetitlePermit, CliError> {
        let config = self.config()?;
        let permit = match self.semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                let queued = self.waiting.fetch_add(1, Ordering::SeqCst);
                if queued >= config.queue_size {
                    self.waiting.fetch_sub(1, Ordering::SeqCst);
                    return Err(retitle_error(
                        "retitle-queue-saturated",
                        "title provider queue is saturated",
                        true,
                        "wait_and_retry",
                        "bounded_backoff",
                    ));
                }
                let _waiter = WaitingRetitleGuard(&self.waiting);
                let acquired =
                    tokio::time::timeout(config.timeout(), self.semaphore.clone().acquire_owned())
                        .await;
                acquired
                    .map_err(|_| {
                        retitle_error(
                            "retitle-queue-saturated",
                            "title provider queue wait timed out",
                            true,
                            "wait_and_retry",
                            "bounded_backoff",
                        )
                    })?
                    .map_err(|_| {
                        retitle_error(
                            "retitle-provider-unavailable",
                            "title provider is unavailable",
                            true,
                            "retry",
                            "retry_request",
                        )
                    })?
            }
        };
        Ok(RetitlePermit {
            config,
            _permit: permit,
        })
    }

    pub(crate) async fn lock_session(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let gate = {
            let mut gates = self
                .session_gates
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            gates.retain(|_, gate| Arc::strong_count(gate) > 1);
            gates
                .entry(id.to_string())
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        gate.lock_owned().await
    }
}

struct WaitingRetitleGuard<'a>(&'a AtomicUsize);

impl Drop for WaitingRetitleGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

pub(crate) struct RetitlePermit {
    config: Arc<RetitleConfig>,
    _permit: OwnedSemaphorePermit,
}

#[derive(Clone, Debug, Serialize)]
struct TitleContextSession {
    agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    repo_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title_state: Option<TitleContextTitleState>,
}

#[derive(Clone, Debug, Serialize)]
struct TitleContextTitleState {
    topic: Option<String>,
    topic_source: SessionTitleTopicSource,
    references: Vec<String>,
    activity: Option<String>,
}

impl From<&SessionTitleState> for TitleContextTitleState {
    fn from(state: &SessionTitleState) -> Self {
        Self {
            topic: if state.topic_source == SessionTitleTopicSource::User {
                state
                    .topic
                    .as_ref()
                    .map(|_| "<user-owned-topic>".to_string())
            } else {
                state
                    .topic
                    .as_deref()
                    .map(filter_text)
                    .filter(|value| !value.is_empty())
            },
            topic_source: state.topic_source.clone(),
            references: state
                .references
                .iter()
                .filter(|reference| {
                    reference.starts_with('#')
                        && reference[1..]
                            .chars()
                            .all(|character| character.is_ascii_digit())
                })
                .cloned()
                .collect(),
            activity: state
                .activity
                .as_deref()
                .map(filter_text)
                .filter(|value| !value.is_empty()),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct TitleContextTurn {
    id: String,
    user_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    assistant_excerpt: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct TitleContextCoverage {
    source: &'static str,
    complete: bool,
    truncated: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct TitleContextV2 {
    schema_version: &'static str,
    session: TitleContextSession,
    turns: Vec<TitleContextTurn>,
    coverage: TitleContextCoverage,
    trigger: RetitleTrigger,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct CoverageView {
    pub(crate) source: &'static str,
    pub(crate) complete: bool,
    pub(crate) truncated: bool,
    pub(crate) turn_count: usize,
}

impl TitleContextV2 {
    pub(crate) fn coverage_view(&self) -> CoverageView {
        CoverageView {
            source: self.coverage.source,
            complete: self.coverage.complete,
            truncated: self.coverage.truncated,
            turn_count: self.turns.len(),
        }
    }
}

pub(crate) fn build_context(
    catalog: &HistoryCatalog,
    record: &SessionRecord,
    trigger: RetitleTrigger,
    config: &RetitlePermit,
) -> Result<TitleContextV2, CliError> {
    let resume = record
        .provider_resume
        .as_ref()
        .ok_or_else(context_unavailable)?;
    if !matches!(resume.provider.as_str(), "codex" | "claude") {
        return Err(context_unavailable());
    }
    let history_id = crate::provider_history::stable_history_id(
        &resume.provider,
        session_agent_profile(record),
        &resume.session_id,
    );
    let mut first_cursor = None;
    let mut first_user = None;
    let mut anchor_scan_truncated = false;
    for page_index in 0..4 {
        let page = catalog
            .messages(
                &history_id,
                first_cursor.as_deref(),
                100,
                HistoryMessageDirection::Forward,
            )
            .map_err(map_history_error)?;
        first_user = page
            .messages
            .iter()
            .find(|message| message.role == "user" && message.human_prompt)
            .cloned();
        if first_user.is_some() || page.next_cursor.is_none() {
            break;
        }
        first_cursor = page.next_cursor;
        anchor_scan_truncated = page_index == 3;
    }
    let latest_page = catalog
        .messages(&history_id, None, 100, HistoryMessageDirection::Latest)
        .map_err(map_history_error)?;
    let source_truncated = anchor_scan_truncated || latest_page.older_cursor.is_some();
    let mut messages = latest_page.messages;
    if let Some(first) = first_user
        && !messages.iter().any(|message| message.id == first.id)
    {
        messages.insert(0, first);
    }
    let turns = pair_turns(messages);
    let (turns, budget_truncated) = select_turns(turns, &config.config.context);
    if turns.is_empty() {
        return Err(context_unavailable());
    }
    Ok(TitleContextV2 {
        schema_version: "agent-session.title-context.v2",
        session: TitleContextSession {
            agent: record.agent.clone(),
            repo_name: crate::repo_name_from_cwd(&record.cwd),
            title_state: record
                .title_state
                .as_ref()
                .map(TitleContextTitleState::from),
        },
        turns,
        coverage: TitleContextCoverage {
            source: "provider_transcript",
            complete: true,
            truncated: source_truncated || budget_truncated,
        },
        trigger,
    })
}

fn map_history_error(error: HistoryError) -> CliError {
    let _ = error;
    context_unavailable()
}

fn context_unavailable() -> CliError {
    retitle_error(
        "retitle-context-unavailable",
        "authoritative title context is unavailable",
        true,
        "wait_for_context",
        "retry_after_transcript_catchup",
    )
}

fn pair_turns(messages: Vec<HistoryMessage>) -> Vec<TitleContextTurn> {
    let mut turns: Vec<TitleContextTurn> = Vec::new();
    let mut assistant_allowed = false;
    for message in messages {
        match message.role.as_str() {
            "user" if !message.human_prompt => assistant_allowed = false,
            "user" => {
                let prompt = filter_text(&message.text);
                if !prompt.is_empty() {
                    turns.push(TitleContextTurn {
                        id: message.id,
                        user_prompt: prompt,
                        assistant_excerpt: None,
                    });
                    assistant_allowed = true;
                }
            }
            "assistant" if assistant_allowed => {
                if let Some(turn) = turns.last_mut()
                    && turn.assistant_excerpt.is_none()
                {
                    let excerpt = filter_text(&message.text);
                    if !excerpt.is_empty() {
                        turn.assistant_excerpt = Some(excerpt);
                    }
                }
                assistant_allowed = false;
            }
            _ => {}
        }
    }
    turns
}

fn select_turns(
    mut turns: Vec<TitleContextTurn>,
    config: &ContextConfig,
) -> (Vec<TitleContextTurn>, bool) {
    if turns.is_empty() {
        return (turns, false);
    }
    let message_limit = config.per_message_chars.min(config.max_chars);
    for turn in &mut turns {
        turn.user_prompt = truncate_chars(&turn.user_prompt, message_limit);
        turn.assistant_excerpt = turn
            .assistant_excerpt
            .take()
            .map(|value| truncate_chars(&value, message_limit));
    }
    let original_len = turns.len();
    let mut anchor = turns.remove(0);
    let anchor_user_chars = anchor.user_prompt.chars().count();
    if let Some(excerpt) = anchor.assistant_excerpt.take() {
        let remaining = config.max_chars.saturating_sub(anchor_user_chars);
        anchor.assistant_excerpt = (remaining > 0).then(|| truncate_chars(&excerpt, remaining));
    }
    let recent_start = turns
        .len()
        .saturating_sub(config.recent_turns.saturating_sub(1));
    let mut selected = vec![anchor];
    selected.extend(turns.into_iter().skip(recent_start));
    let mut consumed = 0usize;
    let mut kept_reversed = Vec::new();
    let anchor = selected.remove(0);
    consumed += turn_chars(&anchor);
    for turn in selected.into_iter().rev() {
        let chars = turn_chars(&turn);
        if consumed.saturating_add(chars) <= config.max_chars {
            consumed += chars;
            kept_reversed.push(turn);
        }
    }
    kept_reversed.reverse();
    let mut result = vec![anchor];
    result.extend(kept_reversed);
    let truncated = result.len() < original_len;
    (result, truncated)
}

fn turn_chars(turn: &TitleContextTurn) -> usize {
    turn.user_prompt.chars().count()
        + turn
            .assistant_excerpt
            .as_deref()
            .map_or(0, |value| value.chars().count())
}

fn filter_text(value: &str) -> String {
    let mut result = Vec::new();
    let mut injected_block = false;
    for raw in value.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with("<environment_context")
            || trimmed.starts_with("<codex_internal_context")
        {
            injected_block = true;
            continue;
        }
        if injected_block {
            if trimmed.starts_with("</environment_context")
                || trimmed.starts_with("</codex_internal_context")
            {
                injected_block = false;
            }
            continue;
        }
        if trimmed.is_empty()
            || trimmed.starts_with("# AGENTS.md instructions")
            || trimmed.starts_with("<INSTRUCTIONS>")
        {
            continue;
        }
        let words = redact_words(trimmed);
        if !words.is_empty() {
            result.push(words);
        }
    }
    result.join("\n")
}

fn redact_words(line: &str) -> String {
    let mut redact_next = false;
    line.split_whitespace()
        .map(|word| {
            if redact_next {
                redact_next = false;
                return "<redacted>".to_string();
            }
            let candidate = word.trim_matches(|character: char| {
                matches!(
                    character,
                    '(' | ')' | '[' | ']' | '{' | '}' | '\'' | '"' | ',' | ';'
                )
            });
            let lower = candidate.to_ascii_lowercase();
            let key = lower.trim_end_matches([':', '=']);
            if matches!(
                key,
                "token" | "api_key" | "apikey" | "password" | "authorization"
            ) {
                redact_next = !lower.contains('=');
                return "<redacted>".to_string();
            }
            if candidate.starts_with('/')
                || candidate.starts_with("~/")
                || candidate.starts_with("sk-")
                || lower.starts_with("bearer=")
                || lower.starts_with("authorization:")
                || lower.contains("api_key=")
                || lower.contains("token=")
                || lower.contains("password=")
            {
                "<redacted>".to_string()
            } else {
                word.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate_chars(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum TopicAction {
    Keep,
    Set,
    Clear,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDecision {
    topic_action: TopicAction,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    activity: Option<String>,
    #[serde(default)]
    references: Vec<String>,
}

pub(crate) fn infer_title_state(
    permit: &RetitlePermit,
    context: &TitleContextV2,
    existing: Option<&SessionTitleState>,
) -> Result<SessionTitleState, CliError> {
    let input = model_input(context)?;
    let output = match permit.config.kind() {
        "codex_subscription" => invoke_codex(&permit.config, &input),
        "openai_compatible" => invoke_openai_compatible(&permit.config, &input),
        "command" => invoke_command(&permit.config, &input),
        _ => unreachable!(),
    }?;
    parse_decision(&output, context, existing)
}

fn model_input(context: &TitleContextV2) -> Result<String, CliError> {
    let context = serde_json::to_string(context).map_err(|_| provider_malformed())?;
    Ok(format!(
        "Return only one JSON object with keys topic_action (keep|set|clear), topic (string|null), activity (string|null), references (array of #number strings). Preserve the durable first user objective through routine follow-ups; change an automatic topic only for a clear user-directed objective change. Never invent references. Do not use tools. Context:\n{context}"
    ))
}

fn invoke_openai_compatible(config: &RetitleConfig, input: &str) -> Result<String, CliError> {
    let base = config
        .base_url
        .as_deref()
        .expect("validated")
        .trim_end_matches('/');
    let endpoint = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{base}/chat/completions")
    };
    let mut body = Map::from_iter([
        ("model".to_string(), json!(config.model)),
        (
            "messages".to_string(),
            json!([
                {"role":"system","content":"You are a session title classifier. Return strict JSON only and never call tools."},
                {"role":"user","content":input}
            ]),
        ),
        ("max_tokens".to_string(), json!(config.max_output_tokens)),
        (
            "temperature".to_string(),
            json!(config.temperature.unwrap_or(0.0)),
        ),
    ]);
    if config.json_response {
        body.insert("response_format".to_string(), json!({"type":"json_object"}));
    }
    for (key, value) in &config.extra_body {
        if matches!(key.as_str(), "model" | "messages" | "stream") {
            return Err(retitle_error(
                "retitle-config-invalid",
                "title provider extra_body overrides a protected field",
                false,
                "configure_provider",
                "replace_configuration",
            ));
        }
        body.insert(key.clone(), value.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(config.timeout())
        .build()
        .map_err(|_| provider_unavailable())?;
    let mut request = client.post(endpoint).json(&body);
    if let Some(name) = config.api_key_env.as_deref() {
        let key = env::var(name).map_err(|_| api_key_missing())?;
        request = request.bearer_auth(key);
    }
    let response = request.send().map_err(|error| {
        if error.is_timeout() {
            provider_timeout()
        } else {
            provider_unavailable()
        }
    })?;
    let status = response.status();
    if status.as_u16() == 429 {
        return Err(retitle_error(
            "retitle-provider-rate-limited",
            "title provider rate limit was reached",
            true,
            "wait_and_retry",
            "bounded_backoff",
        ));
    }
    if status.as_u16() == 402 || status.as_u16() == 403 {
        return Err(retitle_error(
            "retitle-provider-quota-exceeded",
            "title provider quota is unavailable",
            true,
            "wait_and_retry",
            "wait_for_quota",
        ));
    }
    if !status.is_success() {
        return Err(provider_unavailable());
    }
    let bytes = response.bytes().map_err(|_| provider_malformed())?;
    if bytes.len() > MAX_PROVIDER_OUTPUT_BYTES {
        return Err(provider_malformed());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| provider_malformed())?;
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(provider_malformed)
}

fn invoke_command(config: &RetitleConfig, input: &str) -> Result<String, CliError> {
    let payload = json!({
        "schema_version":"agent-session.title-model.request.v2",
        "input":input,
    });
    let output = run_bounded_child(
        config.argv.as_ref().expect("validated"),
        serde_json::to_vec(&payload).map_err(|_| provider_malformed())?,
        config.timeout(),
        &[],
    )?;
    String::from_utf8(output).map_err(|_| provider_malformed())
}

fn invoke_codex(config: &RetitleConfig, input: &str) -> Result<String, CliError> {
    let account = config.account.as_deref().expect("validated");
    let credentials = crate::codex_account::resolve_account(account, false)
        .or_else(|_| crate::codex_account::resolve_account(account, true))
        .map_err(|_| {
            retitle_error(
                "retitle-account-missing",
                "configured title account is unavailable",
                true,
                "select_account",
                "refresh_account",
            )
        })?;
    let isolated = tempfile::TempDir::new().map_err(|_| provider_unavailable())?;
    let mut argv = vec![
        config
            .codex_bin
            .as_ref()
            .expect("validated")
            .to_string_lossy()
            .to_string(),
        "app-server".to_string(),
        "--stdio".to_string(),
        "-c".to_string(),
        "analytics.enabled=false".to_string(),
        "-c".to_string(),
        "check_for_update_on_startup=false".to_string(),
    ];
    let codex_home = isolated.path().join("codex-home");
    std::fs::create_dir(&codex_home).map_err(|_| provider_unavailable())?;
    let codex_home = codex_home.to_string_lossy().to_string();
    let mut child = spawn_bounded_child(&argv, &[("CODEX_HOME", codex_home.as_str())])?;
    let result = (|| {
        let mut stdin = child.stdin.take().ok_or_else(provider_unavailable)?;
        let stdout = child.stdout.take().ok_or_else(provider_unavailable)?;
        let (tx, rx) = std::sync::mpsc::sync_channel(64);
        thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let line = if line.len() <= MAX_PROVIDER_OUTPUT_BYTES {
                    line
                } else {
                    "oversized-provider-frame".to_string()
                };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let deadline = Instant::now() + config.timeout();
        send_rpc(
            &mut stdin,
            &json!({
            "id":1,
            "method":"initialize",
            "params":{"clientInfo":{"name":"agent-session-retitle","title":"agent-session-retitle","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true,"requestAttestation":false}}
            }),
        )?;
        recv_rpc_response(&rx, 1, deadline)?;
        send_rpc(&mut stdin, &json!({"method":"initialized"}))?;
        send_rpc(
            &mut stdin,
            &json!({
            "id":2,
            "method":"account/login/start",
            "params":{"type":"chatgptAuthTokens","accessToken":credentials.access_token,"chatgptAccountId":credentials.chatgpt_account_id,"chatgptPlanType":credentials.chatgpt_plan_type}
            }),
        )?;
        recv_rpc_response(&rx, 2, deadline)?;
        let mut params = json!({
        "cwd":isolated.path(),
        "ephemeral":true,
        "threadSource":"system",
        "approvalPolicy":"never",
        "sandbox":"read-only",
        "dynamicTools":[],
        "environments":[],
        "runtimeWorkspaceRoots":[isolated.path()],
        "baseInstructions":"Answer directly without tools. Return strict JSON only.",
        "developerInstructions":"Never use tools, shell, filesystem, network tools, skills, or external context. Classify only the supplied title context."
        });
        if let Some(model) = config.model.as_deref() {
            params["model"] = json!(model);
        }
        send_rpc(
            &mut stdin,
            &json!({"id":3,"method":"thread/start","params":params}),
        )?;
        let thread = recv_rpc_response(&rx, 3, deadline)?
            .pointer("/result/thread/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(provider_malformed)?;
        send_rpc(
            &mut stdin,
            &json!({
            "id":4,
            "method":"turn/start",
            "params":{"threadId":thread,"input":[{"type":"text","text":input,"text_elements":[]}]}
            }),
        )?;
        recv_rpc_response(&rx, 4, deadline)?;
        let mut answer = None;
        loop {
            let value = recv_rpc(&rx, deadline)?;
            if value.get("method").and_then(Value::as_str) == Some("item/completed")
                && value.pointer("/params/threadId").and_then(Value::as_str)
                    == Some(thread.as_str())
                && value.pointer("/params/item/type").and_then(Value::as_str)
                    == Some("agentMessage")
            {
                answer = value
                    .pointer("/params/item/text")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            if value.get("method").and_then(Value::as_str) == Some("turn/completed")
                && value.pointer("/params/threadId").and_then(Value::as_str)
                    == Some(thread.as_str())
            {
                break;
            }
        }
        answer.ok_or_else(provider_malformed)
    })();
    terminate_child(&mut child);
    argv.clear();
    result
}

fn send_rpc(stdin: &mut impl Write, value: &Value) -> Result<(), CliError> {
    serde_json::to_writer(&mut *stdin, value).map_err(|_| provider_unavailable())?;
    stdin.write_all(b"\n").map_err(|_| provider_unavailable())?;
    stdin.flush().map_err(|_| provider_unavailable())
}

fn recv_rpc_response(
    receiver: &std::sync::mpsc::Receiver<String>,
    id: u64,
    deadline: Instant,
) -> Result<Value, CliError> {
    loop {
        let value = recv_rpc(receiver, deadline)?;
        if value.get("id").and_then(Value::as_u64) == Some(id) {
            if value.get("error").is_some() {
                return Err(provider_unavailable());
            }
            return Ok(value);
        }
    }
}

fn recv_rpc(
    receiver: &std::sync::mpsc::Receiver<String>,
    deadline: Instant,
) -> Result<Value, CliError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(provider_timeout());
    }
    let line = receiver
        .recv_timeout(remaining)
        .map_err(|error| match error {
            std::sync::mpsc::RecvTimeoutError::Timeout => provider_timeout(),
            std::sync::mpsc::RecvTimeoutError::Disconnected => provider_unavailable(),
        })?;
    serde_json::from_str(&line).map_err(|_| provider_malformed())
}

fn parse_decision(
    output: &str,
    context: &TitleContextV2,
    existing: Option<&SessionTitleState>,
) -> Result<SessionTitleState, CliError> {
    let trimmed = output.trim();
    let json_text = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .map(str::trim)
        .unwrap_or(trimmed);
    let decision: RawDecision =
        serde_json::from_str(json_text).map_err(|_| provider_malformed())?;
    if decision
        .topic
        .as_deref()
        .is_some_and(|value| value.chars().count() > 120)
        || decision
            .activity
            .as_deref()
            .is_some_and(|value| value.chars().count() > 120)
        || decision.references.len() > 2
    {
        return Err(provider_malformed());
    }
    let existing = existing.cloned().unwrap_or(SessionTitleState {
        topic: None,
        topic_source: SessionTitleTopicSource::None,
        references: Vec::new(),
        activity: None,
        extra: BTreeMap::new(),
    });
    let user_owned = existing.topic_source == SessionTitleTopicSource::User;
    let (topic, topic_source, references) =
        if user_owned || decision.topic_action == TopicAction::Keep {
            (existing.topic, existing.topic_source, existing.references)
        } else if decision.topic_action == TopicAction::Clear {
            (None, SessionTitleTopicSource::None, Vec::new())
        } else {
            let topic = decision
                .topic
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(provider_malformed)?;
            let allowed = approved_references(context);
            let references = decision
                .references
                .into_iter()
                .filter(|reference| allowed.contains(reference))
                .take(2)
                .collect();
            (Some(topic), SessionTitleTopicSource::Auto, references)
        };
    let mut activity = decision
        .activity
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if topic
        .as_deref()
        .zip(activity.as_deref())
        .is_some_and(|(topic, activity)| topic.eq_ignore_ascii_case(activity))
    {
        activity = None;
    }
    let state = SessionTitleState {
        topic,
        topic_source,
        references,
        activity,
        extra: existing.extra,
    };
    canonicalize_structured_title_pair(None, false, state.clone())
        .map(|(_, state)| state.expect("structured title state"))
        .map_err(|_| provider_malformed())
}

fn approved_references(context: &TitleContextV2) -> BTreeSet<String> {
    let mut references = BTreeSet::new();
    for turn in &context.turns {
        for word in turn.user_prompt.split_whitespace() {
            let candidate = word.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '#'
            });
            let number = candidate.strip_prefix('#').unwrap_or_default();
            if !number.is_empty()
                && !number.starts_with('0')
                && number.len() <= 10
                && number.chars().all(|character| character.is_ascii_digit())
            {
                references.insert(format!("#{number}"));
            }
        }
    }
    references
}

fn run_bounded_child(
    argv: &[String],
    input: Vec<u8>,
    timeout: Duration,
    extra_env: &[(&str, &str)],
) -> Result<Vec<u8>, CliError> {
    let mut child = spawn_bounded_child(argv, extra_env)?;
    let mut stdin = child.stdin.take().ok_or_else(provider_unavailable)?;
    let writer = thread::spawn(move || stdin.write_all(&input));
    let mut stdout = child.stdout.take().ok_or_else(provider_unavailable)?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .by_ref()
            .take((MAX_PROVIDER_OUTPUT_BYTES + 1) as u64)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let _ = writer.join();
                let output = reader
                    .join()
                    .ok()
                    .and_then(Result::ok)
                    .ok_or_else(provider_unavailable)?;
                if !status.success() {
                    return Err(provider_unavailable());
                }
                if output.len() > MAX_PROVIDER_OUTPUT_BYTES {
                    return Err(provider_malformed());
                }
                return Ok(output);
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                terminate_child(&mut child);
                return Err(provider_timeout());
            }
            Err(_) => {
                terminate_child(&mut child);
                return Err(provider_unavailable());
            }
        }
    }
}

fn spawn_bounded_child(argv: &[String], extra_env: &[(&str, &str)]) -> Result<Child, CliError> {
    let (program, args) = argv.split_first().ok_or_else(provider_unavailable)?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .process_group(0);
    for name in ["HOME", "PATH", "TMPDIR", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
        if let Ok(value) = env::var(name) {
            command.env(name, value);
        }
    }
    for (name, value) in extra_env {
        command.env(name, value);
    }
    command.spawn().map_err(|_| provider_unavailable())
}

fn terminate_child(child: &mut Child) {
    if let Ok(pid) = i32::try_from(child.id()) {
        unsafe {
            libc::kill(-pid, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    let _ = child.wait();
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .ok()
        .is_some_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn command_executable(argv: &[String]) -> bool {
    let Some(program) = argv.first() else {
        return false;
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    env::var_os("PATH")
        .is_some_and(|path| env::split_paths(&path).any(|dir| executable(&dir.join(program))))
}

fn provider_timeout() -> CliError {
    retitle_error(
        "retitle-provider-timeout",
        "title provider timed out",
        true,
        "retry",
        "retry_request",
    )
}

fn provider_unavailable() -> CliError {
    retitle_error(
        "retitle-provider-unavailable",
        "title provider is unavailable",
        true,
        "retry",
        "retry_request",
    )
}

fn provider_malformed() -> CliError {
    retitle_error(
        "retitle-provider-malformed-response",
        "title provider returned an invalid decision",
        true,
        "inspect_provider",
        "repair_provider_output",
    )
}

fn api_key_missing() -> CliError {
    retitle_error(
        "retitle-api-key-missing",
        "title provider API key is unavailable",
        false,
        "set_api_key",
        "configure_secret",
    )
}

pub(crate) fn retitle_error(
    code: &'static str,
    message: &'static str,
    retryable: bool,
    next_action: &'static str,
    strategy: &'static str,
) -> CliError {
    CliError::unavailable(
        code,
        message,
        Some(json!({
            "retryable": retryable,
            "next_action": next_action,
            "recovery": {"strategy":strategy, "safe_to_retry":retryable}
        })),
    )
}

pub(crate) fn hash_identity(value: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"agent-session-retitle-v2\0");
    digest.update(value.as_bytes());
    format!(
        "sha256:{}",
        digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RetitleReceipt {
    key_hash: String,
    request_hash: String,
    state: String,
    diagnostic_code: String,
    #[serde(default)]
    coverage_complete: bool,
    #[serde(default)]
    coverage_truncated: bool,
    #[serde(default)]
    coverage_turn_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct DurableRetitleState {
    schema_version: String,
    #[serde(default)]
    receipts: Vec<RetitleReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_processed_turn_hash: Option<String>,
    #[serde(default)]
    last_coverage_complete: bool,
    #[serde(default)]
    last_coverage_truncated: bool,
    #[serde(default)]
    last_coverage_turn_count: usize,
}

fn durable_state(record: &SessionRecord) -> DurableRetitleState {
    record
        .extra
        .get(MARKER_KEY)
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
        .filter(|state: &DurableRetitleState| state.schema_version == MARKER_SCHEMA)
        .unwrap_or_else(|| DurableRetitleState {
            schema_version: MARKER_SCHEMA.to_string(),
            ..DurableRetitleState::default()
        })
}

fn store_durable_state(record: &mut SessionRecord, state: DurableRetitleState) {
    record.extra.insert(
        MARKER_KEY.to_string(),
        serde_json::to_value(state).expect("retitle state"),
    );
}

pub(crate) fn request_hash(request: &RetitleRequest) -> String {
    hash_identity(&serde_json::to_string(request).unwrap_or_default())
}

pub(crate) enum Admission {
    Evaluate(SessionRecord),
    Replay(SessionRecord, &'static str, CoverageView),
}

pub(crate) fn admit(
    context: &CliContext,
    id: &str,
    request: &RetitleRequest,
) -> Result<Admission, CliError> {
    request.validate()?;
    let _lock = crate::acquire_session_record_lock(context, id)?;
    let mut record = load_session_record(context, id)?;
    validate_incarnation(&record, request)?;
    let key_hash = hash_identity(&request.idempotency_key);
    let digest = request_hash(request);
    let mut state = durable_state(&record);
    if let Some(index) = state
        .receipts
        .iter()
        .position(|receipt| receipt.key_hash == key_hash)
    {
        let receipt = &state.receipts[index];
        if receipt.request_hash != digest {
            return Err(CliError::data(
                "idempotency-key-reused",
                "idempotency key is already bound to another retitle request",
                Some(json!({
                    "retryable":false,
                    "next_action":"refresh_session",
                    "recovery":{"strategy":"replace_idempotency_key", "safe_to_retry":false}
                })),
            ));
        }
        if receipt.state == "complete" {
            return Ok(Admission::Replay(
                record,
                "idempotency_replay",
                CoverageView {
                    source: "provider_transcript",
                    complete: receipt.coverage_complete,
                    truncated: receipt.coverage_truncated,
                    turn_count: receipt.coverage_turn_count,
                },
            ));
        }
        if receipt.state == "failed" {
            return Err(replayed_error(&receipt.diagnostic_code));
        }
        // Every in-process caller holds the per-session async gate before
        // admission. An in-progress receipt that remains here therefore came
        // from a terminated daemon and is safe to recover in this incarnation.
        state.receipts.remove(index);
    }
    if request.trigger.is_automatic()
        && request
            .expected_provider_turn_id
            .as_deref()
            .map(hash_identity)
            .as_ref()
            == state.last_processed_turn_hash.as_ref()
    {
        return Ok(Admission::Replay(
            record,
            "already_processed",
            CoverageView {
                source: "provider_transcript",
                complete: state.last_coverage_complete,
                truncated: state.last_coverage_truncated,
                turn_count: state.last_coverage_turn_count,
            },
        ));
    }
    validate_fences(context, &record, request)?;
    state.receipts.push(RetitleReceipt {
        key_hash,
        request_hash: digest,
        state: "in_progress".to_string(),
        diagnostic_code: "in_progress".to_string(),
        coverage_complete: false,
        coverage_truncated: false,
        coverage_turn_count: 0,
    });
    if state.receipts.len() > MAX_RECEIPTS {
        state.receipts.remove(0);
    }
    store_durable_state(&mut record, state);
    crate::write_session_record(context, &record)?;
    Ok(Admission::Evaluate(record))
}

fn validate_fences(
    context: &CliContext,
    record: &SessionRecord,
    request: &RetitleRequest,
) -> Result<(), CliError> {
    validate_incarnation(record, request)?;
    if record.title_revision != request.expected_title_revision {
        return Err(CliError::data(
            "title-revision-conflict",
            "session title changed before retitle",
            Some(json!({
                "expected_title_revision":request.expected_title_revision,
                "actual_title_revision":record.title_revision,
                "retryable":false,
                "next_action":"refresh_session",
                "recovery":{"strategy":"refresh_session", "safe_to_retry":false}
            })),
        ));
    }
    if request.trigger.is_automatic() {
        let state = activity::state_for_view(context, record).ok_or_else(turn_conflict)?;
        if Some(state.revision) != request.expected_activity_revision
            || !turn_matches(
                &state,
                request
                    .expected_provider_turn_id
                    .as_deref()
                    .expect("validated"),
            )
        {
            return Err(turn_conflict());
        }
    }
    Ok(())
}

fn validate_incarnation(record: &SessionRecord, request: &RetitleRequest) -> Result<(), CliError> {
    let actual_incarnation = crate::coordination::incarnation(record)?;
    if actual_incarnation != request.expected_session_incarnation {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "session incarnation changed before retitle",
            Some(json!({
                "retryable":false,
                "next_action":"refresh_session",
                "recovery":{"strategy":"refresh_session", "safe_to_retry":false}
            })),
        ));
    }
    Ok(())
}

fn turn_matches(state: &activity::TurnState, expected: &str) -> bool {
    state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        == Some(expected)
        || state
            .last_turn
            .as_ref()
            .and_then(|turn| turn.provider_turn_id.as_deref())
            == Some(expected)
}

fn turn_conflict() -> CliError {
    CliError::data(
        "retitle-turn-conflict",
        "provider turn changed before retitle",
        Some(json!({
            "retryable":false,
            "next_action":"refresh_session",
            "recovery":{"strategy":"process_newest_turn", "safe_to_retry":false}
        })),
    )
}

pub(crate) fn fail_code(
    context: &CliContext,
    id: &str,
    request: &RetitleRequest,
    diagnostic_code: &str,
) {
    let Ok(_lock) = crate::acquire_session_record_lock(context, id) else {
        return;
    };
    let Ok(mut record) = load_session_record(context, id) else {
        return;
    };
    if crate::coordination::incarnation(&record).ok().as_deref()
        != Some(request.expected_session_incarnation.as_str())
    {
        return;
    }
    let key_hash = hash_identity(&request.idempotency_key);
    let mut state = durable_state(&record);
    if let Some(receipt) = state
        .receipts
        .iter_mut()
        .find(|receipt| receipt.key_hash == key_hash)
    {
        receipt.state = "failed".to_string();
        receipt.diagnostic_code = diagnostic_code.to_string();
        store_durable_state(&mut record, state);
        let _ = crate::write_session_record(context, &record);
    }
}

fn replayed_error(code: &str) -> CliError {
    match code {
        "retitle-context-unavailable" => context_unavailable(),
        "retitle-provider-timeout" => provider_timeout(),
        "retitle-provider-rate-limited" => retitle_error(
            "retitle-provider-rate-limited",
            "title provider rate limit was reached",
            true,
            "wait_and_retry",
            "bounded_backoff",
        ),
        "retitle-provider-quota-exceeded" => retitle_error(
            "retitle-provider-quota-exceeded",
            "title provider quota is unavailable",
            true,
            "wait_and_retry",
            "wait_for_quota",
        ),
        "retitle-provider-malformed-response" => provider_malformed(),
        _ => provider_unavailable(),
    }
}

pub(crate) struct CommitResult {
    pub(crate) record: SessionRecord,
    pub(crate) changed: bool,
}

pub(crate) fn commit(
    context: &CliContext,
    id: &str,
    request: &RetitleRequest,
    state: SessionTitleState,
    coverage: &CoverageView,
) -> Result<CommitResult, CliError> {
    let (title, state) = canonicalize_structured_title_pair(None, false, state)?;
    let state = state.expect("structured state");
    let key_hash = hash_identity(&request.idempotency_key);
    let request_digest = request_hash(request);
    let (record, changed) =
        crate::mutate_session_record_for_title(context, id, None, None, |record| {
            validate_fences(context, record, request)?;
            let mut durable = durable_state(record);
            let receipt = durable
                .receipts
                .iter_mut()
                .find(|receipt| {
                    receipt.key_hash == key_hash && receipt.request_hash == request_digest
                })
                .ok_or_else(|| {
                    retitle_error(
                        "retitle-state-conflict",
                        "retitle admission state is unavailable",
                        false,
                        "refresh_session",
                        "refresh_session",
                    )
                })?;
            if receipt.state != "in_progress" {
                return Err(retitle_error(
                    "retitle-state-conflict",
                    "retitle admission state changed before commit",
                    false,
                    "refresh_session",
                    "refresh_session",
                ));
            }
            let changed = record.title != title || record.title_state.as_ref() != Some(&state);
            if changed {
                record.title = title.clone();
                record.title_state = Some(state.clone());
                record.title_revision = record.title_revision.checked_add(1).ok_or_else(|| {
                    retitle_error(
                        "title-revision-overflow",
                        "session title revision cannot advance",
                        false,
                        "none",
                        "none",
                    )
                })?;
                record.updated_at = jiff::Timestamp::now().to_string();
            }
            receipt.state = "complete".to_string();
            receipt.diagnostic_code = if changed { "committed" } else { "no_change" }.to_string();
            receipt.coverage_complete = coverage.complete;
            receipt.coverage_truncated = coverage.truncated;
            receipt.coverage_turn_count = coverage.turn_count;
            if request.trigger.is_automatic() {
                durable.last_processed_turn_hash = request
                    .expected_provider_turn_id
                    .as_deref()
                    .map(hash_identity);
                durable.last_coverage_complete = coverage.complete;
                durable.last_coverage_truncated = coverage.truncated;
                durable.last_coverage_turn_count = coverage.turn_count;
            }
            store_durable_state(record, durable);
            Ok((record.clone(), changed))
        })?;
    Ok(CommitResult { record, changed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn turn(id: usize, user: &str) -> TitleContextTurn {
        TitleContextTurn {
            id: format!("turn-{id}"),
            user_prompt: user.to_string(),
            assistant_excerpt: Some(format!("assistant {id}")),
        }
    }

    #[test]
    fn bounded_context_keeps_anchor_and_newest_turns_in_chronological_order() {
        let turns = (0..14)
            .map(|index| {
                turn(
                    index,
                    if index == 0 {
                        "重新設計 retitle 讓多輪不迷失焦點"
                    } else if index == 13 {
                        "handle review"
                    } else {
                        "continue"
                    },
                )
            })
            .collect();
        let (selected, truncated) = select_turns(
            turns,
            &ContextConfig {
                max_chars: 2_000,
                per_message_chars: 200,
                recent_turns: 4,
            },
        );
        assert!(truncated);
        assert_eq!(selected[0].id, "turn-0");
        assert_eq!(selected[1].id, "turn-11");
        assert_eq!(selected[3].id, "turn-13");
        assert!(selected[0].user_prompt.contains("retitle"));
    }

    #[test]
    fn filtering_removes_paths_credentials_and_injected_setup() {
        let filtered = filter_text(
            "# AGENTS.md instructions for /repo\nwork on /home/alice/private sk-secret TOKEN=value keep this",
        );
        assert_eq!(
            filtered,
            "work on <redacted> <redacted> <redacted> keep this"
        );
    }

    #[test]
    fn strict_decision_preserves_user_topic_and_rejects_invented_references() {
        let context = TitleContextV2 {
            schema_version: "agent-session.title-context.v2",
            session: TitleContextSession {
                agent: "codex".to_string(),
                repo_name: Some("console".to_string()),
                title_state: None,
            },
            turns: vec![turn(1, "Handle issue #449")],
            coverage: TitleContextCoverage {
                source: "provider_transcript",
                complete: true,
                truncated: false,
            },
            trigger: RetitleTrigger::Manual,
        };
        let existing = SessionTitleState {
            topic: Some("User title".to_string()),
            topic_source: SessionTitleTopicSource::User,
            references: vec!["#449".to_string()],
            activity: None,
            extra: BTreeMap::new(),
        };
        let result = parse_decision(
            r##"{"topic_action":"set","topic":"Invented","activity":"Review","references":["#999"]}"##,
            &context,
            Some(&existing),
        )
        .unwrap();
        assert_eq!(result.topic.as_deref(), Some("User title"));
        assert_eq!(result.references, vec!["#449"]);
        assert_eq!(result.activity.as_deref(), Some("Review"));
    }

    #[test]
    fn deterministic_multilingual_golden_corpus_enforces_strict_decisions() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/fixtures/retitle/golden-corpus.json"))
                .unwrap();
        for case in cases.as_array().unwrap() {
            let prompts = case["prompts"].as_array().unwrap();
            let context = TitleContextV2 {
                schema_version: "agent-session.title-context.v2",
                session: TitleContextSession {
                    agent: "codex".to_string(),
                    repo_name: Some("console".to_string()),
                    title_state: None,
                },
                turns: prompts
                    .iter()
                    .enumerate()
                    .map(|(index, prompt)| TitleContextTurn {
                        id: format!("turn-{index}"),
                        user_prompt: prompt.as_str().unwrap().to_string(),
                        assistant_excerpt: None,
                    })
                    .collect(),
                coverage: TitleContextCoverage {
                    source: "provider_transcript",
                    complete: true,
                    truncated: false,
                },
                trigger: RetitleTrigger::Manual,
            };
            let output = serde_json::to_string(&case["decision"]).unwrap();
            let state = parse_decision(&output, &context, None)
                .unwrap_or_else(|error| panic!("case {} failed: {}", case["name"], error.code()));
            assert_eq!(state.topic, case["topic"].as_str().map(str::to_string));
            assert_eq!(
                state.activity,
                case["activity"].as_str().map(str::to_string)
            );
            assert_eq!(
                state.references,
                serde_json::from_value::<Vec<String>>(case["references"].clone()).unwrap()
            );
        }
    }

    #[test]
    fn repeated_identical_prompts_remain_distinct_turns() {
        let turns = pair_turns(vec![
            HistoryMessage {
                id: "provider-turn-a".into(),
                role: "user".into(),
                text: "continue".into(),
                timestamp: None,
                human_prompt: true,
            },
            HistoryMessage {
                id: "provider-turn-b".into(),
                role: "user".into(),
                text: "continue".into(),
                timestamp: None,
                human_prompt: true,
            },
        ]);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].user_prompt, turns[1].user_prompt);
        assert_ne!(turns[0].id, turns[1].id);
        assert_ne!(hash_identity(&turns[0].id), hash_identity(&turns[1].id));
    }

    #[test]
    fn injected_user_records_and_their_assistant_output_are_not_context_turns() {
        let turns = pair_turns(vec![
            HistoryMessage {
                id: "setup".into(),
                role: "user".into(),
                text: "injected setup".into(),
                timestamp: None,
                human_prompt: false,
            },
            HistoryMessage {
                id: "setup-answer".into(),
                role: "assistant".into(),
                text: "private setup output".into(),
                timestamp: None,
                human_prompt: false,
            },
            HistoryMessage {
                id: "turn-real".into(),
                role: "user".into(),
                text: "Review #449".into(),
                timestamp: None,
                human_prompt: true,
            },
        ]);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].id, "turn-real");
        assert!(turns[0].assistant_excerpt.is_none());
    }

    #[test]
    fn unavailable_or_truncated_context_fails_closed_or_reports_truncation() {
        assert!(pair_turns(Vec::new()).is_empty());
        let (selected, truncated) = select_turns(
            (0..20).map(|index| turn(index, "bounded prompt")).collect(),
            &ContextConfig {
                max_chars: 1000,
                per_message_chars: 128,
                recent_turns: 3,
            },
        );
        assert!(truncated);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected.first().unwrap().id, "turn-0");
        assert_eq!(selected.last().unwrap().id, "turn-19");
    }

    #[test]
    fn manual_decision_repairs_an_automatic_topic() {
        let context = TitleContextV2 {
            schema_version: "agent-session.title-context.v2",
            session: TitleContextSession {
                agent: "codex".into(),
                repo_name: None,
                title_state: None,
            },
            turns: vec![turn(1, "Switch to session retitle")],
            coverage: TitleContextCoverage {
                source: "provider_transcript",
                complete: true,
                truncated: false,
            },
            trigger: RetitleTrigger::Manual,
        };
        let existing = SessionTitleState {
            topic: Some("Wrong automatic title".into()),
            topic_source: SessionTitleTopicSource::Auto,
            references: Vec::new(),
            activity: None,
            extra: BTreeMap::new(),
        };
        let repaired = parse_decision(
            r#"{"topic_action":"set","topic":"Session Retitle","activity":null,"references":[]}"#,
            &context,
            Some(&existing),
        )
        .unwrap();
        assert_eq!(repaired.topic.as_deref(), Some("Session Retitle"));
        assert_eq!(repaired.topic_source, SessionTitleTopicSource::Auto);
    }

    #[test]
    fn overlong_decision_is_rejected_without_partial_title() {
        let context = TitleContextV2 {
            schema_version: "agent-session.title-context.v2",
            session: TitleContextSession {
                agent: "codex".into(),
                repo_name: None,
                title_state: None,
            },
            turns: vec![turn(1, "safe request")],
            coverage: TitleContextCoverage {
                source: "provider_transcript",
                complete: true,
                truncated: false,
            },
            trigger: RetitleTrigger::Manual,
        };
        let output =
            json!({"topic_action":"set","topic":"x".repeat(121),"activity":null,"references":[]})
                .to_string();
        assert_eq!(
            parse_decision(&output, &context, None).unwrap_err().code(),
            "retitle-provider-malformed-response"
        );
    }

    #[test]
    fn provider_config_accepts_subscription_deepseek_local_and_command_shapes() {
        for (raw, kind) in [
            (
                r#"{"provider":"codex_subscription","account":"sym","codex_bin":"/usr/bin/codex"}"#,
                "codex_subscription",
            ),
            (
                r#"{"provider":"openai_compatible","base_url":"https://api.deepseek.com","model":"deepseek-chat","api_key_env":"DEEPSEEK_API_KEY"}"#,
                "openai_compatible",
            ),
            (
                r#"{"provider":"openai_compatible","base_url":"http://127.0.0.1:8080/v1","model":"local-title-model"}"#,
                "openai_compatible",
            ),
            (
                r#"{"provider":"command","argv":["/usr/bin/title-command"]}"#,
                "command",
            ),
        ] {
            assert_eq!(RetitleConfig::parse(raw).unwrap().kind(), kind);
        }
        assert_eq!(
            RetitleConfig::parse(r#"{"provider":"openai_compatible","base_url":"file:///tmp/socket","model":"unsafe"}"#).unwrap_err(),
            "config_invalid"
        );
    }

    #[test]
    fn strict_request_requires_turn_fences_only_for_automatic_triggers() {
        let manual = RetitleRequest {
            schema_version: REQUEST_SCHEMA.into(),
            trigger: RetitleTrigger::Manual,
            idempotency_key: "manual-0001".into(),
            expected_session_incarnation: "launch".into(),
            expected_title_revision: 0,
            expected_activity_revision: None,
            expected_provider_turn_id: None,
        };
        assert!(manual.validate().is_ok());
        let mut automatic = manual.clone();
        automatic.trigger = RetitleTrigger::Prompt;
        assert_eq!(
            automatic.validate().unwrap_err().code(),
            "invalid-retitle-request"
        );
        automatic.expected_activity_revision = Some(4);
        automatic.expected_provider_turn_id = Some("turn-4".into());
        assert!(automatic.validate().is_ok());
    }

    #[test]
    fn documented_error_contract_matches_the_frozen_machine_vocabulary() {
        let document = include_str!("../docs/specs/session-retitle-v2.md");
        let mut codes = BTreeSet::new();
        for (code, next_action, strategy) in ERROR_CONTRACT {
            assert!(codes.insert(*code), "duplicate code {code}");
            assert!(document.contains(&format!("| `{code}` | `{next_action}` | `{strategy}` |")));
            assert!(is_public_error_code(code));
        }
        assert!(is_public_error_code("session-not-found"));
        assert!(!is_public_error_code("session-write-failed"));
    }
}

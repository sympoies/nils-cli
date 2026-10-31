//! `agent-session serve`: a per-machine control plane (HTTP) plus PTY attach
//! (WebSocket) exposed over loopback for the agent-console edge.
//!
//! The rest of the crate is synchronous; this module builds its own tokio
//! runtime inside the `serve` subcommand and calls the existing synchronous
//! lifecycle functions from `tokio::task::spawn_blocking`, so there is no
//! duplicate state model. Session reads are open on loopback; activity
//! streaming, writes, and the WebSocket attach require a bearer token (fail
//! closed when no token is configured).
//! Every response carries the daemon's `machine` identity so the edge can
//! aggregate multiple machines. Literal keystroke text is never echoed.

#[path = "serve/error_response.rs"]
mod error_response;
pub(crate) use error_response::envelope_err;

#[path = "serve/history_images.rs"]
mod history_images;
#[path = "serve/session_info.rs"]
mod session_info;
#[path = "serve/structured_prompt.rs"]
mod structured_prompt;
#[cfg(test)]
use structured_prompt::submit_claude_structured_prompt;
use structured_prompt::submit_structured_prompt_locked;

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
#[path = "serve/codex_quiet.rs"]
mod codex_quiet;
use std::convert::Infallible;
use std::ffi::CString;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::net::SocketAddr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use futures_util::{SinkExt, StreamExt};
use nils_common::cli_contract::{exit, schema_version_for};
use nils_common::provider_usage::{ProviderUsageReason, prefer_reason};
use nils_common::usage_time::{
    epoch_seconds_from_f64, normalize_epoch_seconds, reset_epoch_seconds_from_str,
};
use notify::{Event as NotifyEvent, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, broadcast, mpsc, watch};
use tokio::task::JoinSet;

use crate::activity;
use crate::auto_resume::{self, UsageSnapshot};
use crate::cli::{self, AgentKind, SpecialKey};
use crate::codex_app_server::{self, ControlHandle};
use crate::coordination::server as coordination_server;
use crate::maintenance::{self, MaintenanceActionRequest, MaintenanceOperation};
use crate::provider_history::{
    self, DshHistorySource, HistoryCatalog, HistoryError, HistoryMessageDirection, HistorySource,
};
use crate::provider_prompt::{
    LastPrompt, MAX_PROVIDER_PROMPT_BYTES, PROVIDER_PROMPT_CAPABILITY, ProviderKind,
    ProviderLastPromptRefresh, ProviderLastPromptTracker, ProviderPromptEvent,
    ProviderPromptSource, ProviderPromptTail,
};
use crate::{
    BINARY, CliContext, CliError, ProviderResumeImportArgs, SessionRecord, SessionRegistryFence,
    SessionTitleState, SessionTitleStateInput, SessionView, StartFailureDisposition,
    WorkdirSearchOptions, archive_session_with_expected_incarnation,
    canonicalize_structured_title_pair, cleanup_session_delete_tombstones, glance_session,
    load_session_record, non_empty_env, prepare_session_attachment, profile_unavailable,
    repo_remote_url_from_cwd, resolve_tmux_bin, resume_session_by_id, search_workdirs,
    send_auto_resume_input, send_input_serialized, session_clipboard_buffer, session_dir,
    session_status, start_provider_resume_session, update_session_title_if_revision,
};
use activity_stream::activity_notify_event_relevant;

mod account_drain;
mod activity_stream;
use account_drain::{apply_pending_codex_accounts, repair_blocked_codex_accounts};
#[path = "serve_claude_completion.rs"]
mod claude_completion;
#[path = "serve_claude_prompt_ack.rs"]
mod claude_prompt_ack;
#[path = "serve/create.rs"]
mod create;
#[path = "serve_create_body.rs"]
mod create_body;
#[path = "serve_prompt_scan.rs"]
mod prompt_scan;
#[path = "serve_remote_outbox.rs"]
mod remote_outbox;
#[path = "serve/resume_start.rs"]
mod resume_start;
#[path = "serve/console_service.rs"]
mod console_service;
use create_body::{CreateBody, create_body_lineage, validate_create_accounts};
#[path = "serve_shell.rs"]
mod shell;

const ATTACH_LIVE_FIFO_NAME: &str = "attach-live.fifo";
const ATTACH_BROADCAST_CAPACITY: usize = 128;
const ATTACH_PIPE_STARTUP_GRACE: Duration = Duration::from_secs(2);
const ATTACH_FIFO_POLL_INTERVAL: Duration = Duration::from_millis(100);
const ATTACH_READER_STOP_TIMEOUT: Duration = Duration::from_millis(250);
const ATTACH_TMUX_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);
const ATTACH_RESIZE_TIMEOUT: Duration = Duration::from_secs(5);
const ATTACH_WEBSOCKET_SEND_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_CONFIGURED_ATTACHMENT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ATTACHMENT_BYTES_ENV: &str = "AGENT_SESSION_MAX_ATTACHMENT_BYTES";
const MAX_STDIN_TOKEN_BYTES: u64 = 8 * 1024;
const USAGE_SCHEMA_VERSION: &str = "agent-session.usage.v1";
const DEFAULT_USAGE_TIMEOUT_MS: u64 = 45_000;
const CLAUDE_USAGE_CLEANUP_SLACK_SECONDS: u64 = 5;
const ATTACH_TERMINAL_QUEUE_CAPACITY: usize = 64;
const ATTACH_CONTROL_QUEUE_CAPACITY: usize = 8;
const ATTACH_TERMINAL_BURST: usize = 32;
const ATTACH_HANDOFF_BUFFER_CAPACITY: usize = ATTACH_BROADCAST_CAPACITY;
const ATTACH_HANDOFF_MAX_RECAPTURES: usize = 1;
const PROVIDER_PROMPT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROVIDER_PROMPT_PENDING_POLL_INTERVAL: Duration = Duration::from_millis(500);
const PROVIDER_PROMPT_PENDING_TIMEOUT: Duration = Duration::from_secs(2 * 60);
const STRUCTURED_PROMPT_CONTROL_WAIT: Duration = Duration::from_secs(3);
const STRUCTURED_PROMPT_CONTROL_POLL: Duration = Duration::from_millis(50);
/// Claude acknowledges a prompt through its own hook, so this wait covers the
/// provider accepting the pasted text and the hook reaching durable turn state.
/// Together with the shorter pane-ready wait it stays inside the dashboard's
/// per-request create deadline, which reports a client-side timeout as unknown.
const CLAUDE_STRUCTURED_PROMPT_ACK_WAIT: Duration = Duration::from_secs(7);
const CLAUDE_STRUCTURED_PROMPT_ACK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const CODEX_ACCOUNT_BINDING_WAIT: Duration = Duration::from_secs(30);
const CODEX_CREATE_PROMPT_CONTROL_WAIT: Duration = CODEX_ACCOUNT_BINDING_WAIT;
const PROVIDER_PROMPT_DISCOVERY_MAX_BACKOFF: Duration = Duration::from_secs(30);
const PROVIDER_PROMPT_DISCOVERY_MAX_ENTRIES: usize = 64;
const PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS: usize = 4;
const PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS: usize = 2;
const MAX_CONCURRENT_AUTO_RESUME_TICKS: usize = 4;
pub(crate) const MAX_AGENT_LAUNCH_PROFILES: usize = 16;

/// Upper bound on the `keys` list a single `send` request may carry.
///
/// The list is remote input whose length is otherwise bounded only by the Axum
/// body limit, and it is what sizes the handler's key allocation. Every accepted
/// name resolves to a canonical `SpecialKey` variant, so this is far above any
/// real caller while keeping the allocation bounded by a constant.
const MAX_SEND_KEYS: usize = 64;
const AGENT_LAUNCH_PROFILE_READINESS_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(test)]
std::thread_local! {
    static PANIC_AGENT_LAUNCH_PROFILE_READINESS_PROBE: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
    static BLOCK_AGENT_LAUNCH_PROFILE_READINESS_PROBE: std::cell::RefCell<Option<(
        std::sync::mpsc::SyncSender<()>,
        std::sync::mpsc::Receiver<()>,
    )>> = const { std::cell::RefCell::new(None) };
    static SIGNAL_AGENT_LAUNCH_PROFILE_READINESS_WAITER: std::cell::RefCell<
        Option<std::sync::mpsc::SyncSender<()>>,
    > = const { std::cell::RefCell::new(None) };
}
const ATTACH_SCHEMA_VERSION: &str = "agent-session.attach.v1";
const ATTACH_EVENT_SCHEMA_VERSION: &str = "agent-session.attach.event.v1";
const ACTIVITY_STREAM_EVENT_SCHEMA_VERSION: &str = "agent-session.activity-stream.event.v1";
const ACTIVITY_STREAM_BROADCAST_CAPACITY: usize = 1;
const ACTIVITY_STREAM_REPLAY_CAPACITY: usize = 128;
const ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY: usize = 512 * 1024;
const ACTIVITY_STREAM_SUBSCRIBER_CAPACITY: usize = 64;
const ACTIVITY_STREAM_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const ACTIVITY_STREAM_DEBOUNCE: Duration = Duration::from_millis(25);
const ACTIVITY_STREAM_MAX_REFRESH_CADENCE: Duration = Duration::from_millis(250);
const ACTIVITY_STREAM_OVERSIZED_REASON: &str = "oversized_snapshot";
const SESSION_DELETE_TOMBSTONE_CLEANUP_LIMIT: usize = 64;
const MANAGED_HISTORY_TITLE_MAX_ENTRIES: usize = 10_000;
const MANAGED_HISTORY_TITLE_SCAN_MAX_DURATION: Duration = Duration::from_secs(2);
const MANAGED_HISTORY_TITLE_CACHE_TTL: Duration = Duration::from_secs(30);
const MANAGED_HISTORY_TITLE_MAX_RECORD_BYTES: u64 = 1024 * 1024;
const COORDINATION_WAIT_WORKER_LIMIT: usize = 16;
const COORDINATION_NOTIFICATION_CONCURRENCY: usize = 8;
const COORDINATION_NOTIFICATION_RETRY: i64 = 1;
const CLAUDE_NOTIFICATION_STOP_DEBOUNCE: Duration = activity::CLAUDE_NOTIFICATION_STOP_DEBOUNCE;
const CLAUDE_NOTIFICATION_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);
const CLAUDE_NOTIFICATION_OBSERVATION_POLL: Duration = Duration::from_millis(100);
const RESET_AT_KEYS: &[&str] = &["reset_at", "resetAt", "resets_at", "resetsAt"];
const RESET_AT_EPOCH_KEYS: &[&str] = &[
    "reset_at_epoch",
    "resetAtEpoch",
    "resets_at_epoch",
    "resetsAtEpoch",
    "reset_at",
    "resetAt",
    "resets_at",
    "resetsAt",
];

/// Shared daemon state handed to every request handler.
type SessionCollector =
    Arc<dyn Fn(&CliContext, &Path) -> Result<Vec<SessionView>, CliError> + Send + Sync>;

struct ServeState {
    context: CliContext,
    machine: String,
    token: Option<String>,
    /// Session board routes (`board.rs`) answer only when enabled.
    board: bool,
    federation: Option<crate::coordination::remote::Config>,
    mailbox_services_file: Option<PathBuf>,
    mailbox_service_workers: Arc<tokio::sync::Semaphore>,
    max_attachment_bytes: u64,
    tmux_bin: PathBuf,
    attach_brokers: AttachBrokerRegistry,
    provider_prompt_discovery: Arc<ProviderPromptDiscoveryRegistry>,
    activity_broker: Arc<ActivityBroker>,
    codex_controls: Arc<StdMutex<HashMap<String, CodexControlEntry>>>,
    account_switches: AccountSwitchRegistry,
    session_collector: SessionCollector,
    launch_profiles: AgentLaunchProfiles,
    history_catalog: Arc<HistoryCatalog>,
    retitle: Arc<crate::retitle::RetitleService>,
    retitle_v3_manual_waiters: Arc<StdMutex<HashMap<String, usize>>>,
    retitle_v3_continuations: Arc<StdMutex<HashSet<String>>>,
    managed_history_titles: Arc<StdMutex<ManagedHistoryTitleCache>>,
    coordination_wait_workers: Arc<tokio::sync::Semaphore>,
    coordination_notification_wake: Arc<tokio::sync::Notify>,
    remote_outbox_wake: Arc<tokio::sync::Notify>,
    /// `GET /usage/v1`, `POST /codex/reset/v1`, and `POST /claude/reset/v1` (`usage.rs`).
    provider_usage: Arc<crate::usage::UsageService>,
    /// `GET /board/programs/v1` (`board/programs.rs`).
    board_programs: Arc<crate::board::ProgramCache>,
}

#[derive(Default)]
struct ManagedHistoryTitleCache {
    generation: u64,
    refreshed_at: Option<Instant>,
    titles: BTreeMap<String, String>,
}

#[derive(Clone)]
struct CoordinationNotificationFence {
    candidate: crate::coordination::NotificationCandidate,
}

#[derive(Default)]
struct StructuredPromptGuards<'a> {
    expected_session_incarnation: Option<&'a str>,
    expected_binding: Option<(&'a str, u64)>,
    notification: Option<CoordinationNotificationFence>,
}

#[derive(Clone)]
struct CodexControlEntry {
    launch_id: String,
    handle: ControlHandle,
}

#[derive(Clone, Default)]
struct AccountSwitchRegistry {
    entries: Arc<tokio::sync::Mutex<HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>>>,
}

impl AccountSwitchRegistry {
    async fn lock(&self, session_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let slot = {
            let mut entries = self.entries.lock().await;
            entries.retain(|_, entry| entry.strong_count() > 0);
            if let Some(slot) = entries.get(session_id).and_then(std::sync::Weak::upgrade) {
                slot
            } else {
                let slot = Arc::new(tokio::sync::Mutex::new(()));
                entries.insert(session_id.to_string(), Arc::downgrade(&slot));
                slot
            }
        };
        slot.lock_owned().await
    }
}

fn default_session_collector() -> SessionCollector {
    Arc::new(|context, tmux_bin| crate::list_sessions_for_serve(context, Some(tmux_bin)))
}

#[derive(Clone, Debug)]
struct AgentLaunchProfiles {
    entries: Vec<AgentLaunchProfile>,
    readiness_probe: Arc<(StdMutex<AgentLaunchProfileReadiness>, std::sync::Condvar)>,
}

#[derive(Debug, Default)]
struct AgentLaunchProfileReadiness {
    refreshing: bool,
    generation: u64,
    summaries: Vec<AgentLaunchProfileSummary>,
}

struct AgentLaunchProfileReadinessRefresh<'a> {
    readiness: &'a StdMutex<AgentLaunchProfileReadiness>,
    completed: &'a std::sync::Condvar,
    active: bool,
}

impl AgentLaunchProfileReadinessRefresh<'_> {
    fn finish(mut self, summaries: &[AgentLaunchProfileSummary]) {
        let mut state = self
            .readiness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.summaries.clear();
        state.summaries.extend_from_slice(summaries);
        state.generation = state.generation.wrapping_add(1);
        state.refreshing = false;
        self.completed.notify_all();
        self.active = false;
    }
}

impl Drop for AgentLaunchProfileReadinessRefresh<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self
            .readiness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.summaries.clear();
        state.generation = state.generation.wrapping_add(1);
        state.refreshing = false;
        self.completed.notify_all();
    }
}

#[derive(Clone, Debug)]
struct AgentLaunchProfile {
    id: String,
    label: String,
    agent: AgentKind,
    agent_bin: PathBuf,
    provider_config_dir: Option<PathBuf>,
    readiness_args: Vec<String>,
    auto_resume_supported: bool,
    graceful_shutdown: Option<AgentLaunchProfileGracefulShutdown>,
    codex_usage_account: Option<String>,
    dsh_history: Option<DshHistoryAdapterConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DshHistoryAdapterConfig {
    command: PathBuf,
    root: PathBuf,
    #[serde(default = "default_dsh_history_compression")]
    compression: String,
    #[serde(default)]
    resume: Option<DshHistoryResumeCapability>,
}

fn default_dsh_history_compression() -> String {
    "zstd".to_string()
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum DshHistoryResumeCapability {
    ExactId,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum AgentLaunchProfileGracefulShutdown {
    DoubleCtrlC,
}

impl AgentLaunchProfileGracefulShutdown {
    fn as_str(self) -> &'static str {
        match self {
            Self::DoubleCtrlC => "double-ctrl-c",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentLaunchProfileConfig {
    id: String,
    label: String,
    agent: String,
    agent_bin: PathBuf,
    provider_config_dir: Option<PathBuf>,
    #[serde(default)]
    readiness_args: Vec<String>,
    #[serde(default)]
    auto_resume_supported: bool,
    #[serde(default)]
    graceful_shutdown: Option<AgentLaunchProfileGracefulShutdown>,
    /// Optional authoritative Codex usage account nickname for a `claude`
    /// profile that runs on a Codex/GPT backend. Absent leaves behavior
    /// unchanged (auto-resume keys off native Claude usage).
    #[serde(default)]
    codex_usage_account: Option<String>,
    #[serde(default)]
    dsh_history: Option<DshHistoryAdapterConfig>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct AgentLaunchProfileSummary {
    id: String,
    label: String,
    agent: String,
    provider_resume_import_supported: bool,
    history_resume_supported: bool,
}

impl AgentLaunchProfiles {
    fn from_environment() -> Result<Self, CliError> {
        let Some(raw) = non_empty_env("AGENT_SESSION_LAUNCH_PROFILES") else {
            return Ok(Self::default());
        };
        let profiles = Self::from_json(&raw)?;
        reject_retired_dsh_launch_profiles(&profiles)?;
        Ok(profiles)
    }

    fn from_json(raw: &str) -> Result<Self, CliError> {
        let configs: Vec<AgentLaunchProfileConfig> = serde_json::from_str(raw).map_err(|_| {
            invalid_agent_launch_profiles("launch profiles must be a valid JSON array")
        })?;
        if configs.len() > MAX_AGENT_LAUNCH_PROFILES {
            return Err(invalid_agent_launch_profiles(
                "too many launch profiles are configured",
            ));
        }
        let mut entries = Vec::with_capacity(configs.len());
        for config in configs {
            if !valid_agent_profile_id(&config.id) {
                return Err(invalid_agent_launch_profiles(
                    "launch profile ids must use 1-32 lowercase letters, digits, or hyphens",
                ));
            }
            if entries
                .iter()
                .any(|entry: &AgentLaunchProfile| entry.id == config.id)
            {
                return Err(invalid_agent_launch_profiles(
                    "launch profile ids must be unique",
                ));
            }
            let label = config.label.trim();
            if label.is_empty() || label.len() > 64 || label.chars().any(char::is_control) {
                return Err(invalid_agent_launch_profiles(
                    "launch profile labels must contain 1-64 safe characters",
                ));
            }
            let Some(agent) = AgentKind::from_name(&config.agent) else {
                return Err(invalid_agent_launch_profiles(
                    "launch profiles must reference a supported base agent",
                ));
            };
            if !config.agent_bin.is_absolute() {
                return Err(invalid_agent_launch_profiles(
                    "launch profile agent_bin paths must be absolute",
                ));
            }
            if config
                .provider_config_dir
                .as_ref()
                .is_some_and(|path| !path.is_absolute())
            {
                return Err(invalid_agent_launch_profiles(
                    "launch profile provider_config_dir paths must be absolute",
                ));
            }
            if config.readiness_args.len() > 8
                || config.readiness_args.iter().any(|arg| {
                    arg.len() > 256 || arg.contains('\0') || arg.chars().any(char::is_control)
                })
            {
                return Err(invalid_agent_launch_profiles(
                    "launch profile readiness arguments exceed safe bounds",
                ));
            }
            if config
                .codex_usage_account
                .as_ref()
                .is_some_and(|nick| nick.trim().is_empty() || nick.chars().any(char::is_control))
            {
                return Err(invalid_agent_launch_profiles(
                    "launch profile codex_usage_account must be a non-empty nickname",
                ));
            }
            if let Some(history) = config.dsh_history.as_ref()
                && (!matches!(agent, AgentKind::Dsh)
                    || !history.command.is_absolute()
                    || !history.root.is_absolute()
                    || !matches!(history.compression.as_str(), "zstd" | "none"))
            {
                return Err(invalid_agent_launch_profiles(
                    "dsh_history requires a dsh profile, absolute command/root paths, and zstd or none compression",
                ));
            }
            entries.push(AgentLaunchProfile {
                id: config.id,
                label: label.to_string(),
                agent,
                agent_bin: config.agent_bin,
                provider_config_dir: config.provider_config_dir,
                readiness_args: config.readiness_args,
                auto_resume_supported: config.auto_resume_supported,
                graceful_shutdown: config.graceful_shutdown,
                codex_usage_account: config
                    .codex_usage_account
                    .map(|nick| nick.trim().to_string()),
                dsh_history: config.dsh_history,
            });
        }
        Ok(Self {
            entries,
            readiness_probe: Arc::new((
                StdMutex::new(AgentLaunchProfileReadiness::default()),
                std::sync::Condvar::new(),
            )),
        })
    }

    fn get(&self, id: &str) -> Option<&AgentLaunchProfile> {
        self.entries.iter().find(|profile| profile.id == id)
    }

    fn history_sources(&self) -> Vec<HistorySource> {
        let mut sources = Vec::new();
        for profile in &self.entries {
            let Some(config_dir) = profile.provider_config_dir.as_ref() else {
                continue;
            };
            let (provider, child) = match profile.agent {
                AgentKind::Codex => ("codex", "sessions"),
                AgentKind::Claude => ("claude", "projects"),
                _ => continue,
            };
            sources.push(HistorySource {
                provider: provider.to_string(),
                agent_profile: Some(profile.id.clone()),
                root: config_dir.join(child),
            });
        }
        if let Some(root) = crate::codex_sessions_root()
            && !sources
                .iter()
                .any(|source| source.provider == "codex" && source.root == root)
        {
            sources.push(HistorySource {
                provider: "codex".to_string(),
                agent_profile: None,
                root,
            });
        }
        if let Some(root) = crate::claude_projects_root()
            && !sources
                .iter()
                .any(|source| source.provider == "claude" && source.root == root)
        {
            sources.push(HistorySource {
                provider: "claude".to_string(),
                agent_profile: None,
                root,
            });
        }
        sources
    }

    fn dsh_history_sources(&self) -> Vec<DshHistorySource> {
        self.entries
            .iter()
            .filter_map(|profile| {
                profile
                    .dsh_history
                    .as_ref()
                    .map(|history| DshHistorySource {
                        agent_profile: profile.id.clone(),
                        command: history.command.clone(),
                        root: history.root.clone(),
                        compression: history.compression.clone(),
                    })
            })
            .collect()
    }

    fn ready_summaries(&self) -> Vec<AgentLaunchProfileSummary> {
        let (readiness, completed) = &*self.readiness_probe;
        let mut state = readiness
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.refreshing {
            let generation = state.generation;
            #[cfg(test)]
            SIGNAL_AGENT_LAUNCH_PROFILE_READINESS_WAITER.with(|signal| {
                if let Some(signal) = signal.borrow_mut().take() {
                    signal
                        .send(())
                        .expect("readiness waiter observer should remain connected");
                }
            });
            while state.refreshing && state.generation == generation {
                state = completed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            return state.summaries.clone();
        }
        state.refreshing = true;
        drop(state);

        let refresh = AgentLaunchProfileReadinessRefresh {
            readiness,
            completed,
            active: true,
        };
        let summaries = self.probe_ready_summaries();
        refresh.finish(&summaries);
        summaries
    }

    fn probe_ready_summaries(&self) -> Vec<AgentLaunchProfileSummary> {
        #[cfg(test)]
        BLOCK_AGENT_LAUNCH_PROFILE_READINESS_PROBE.with(|block| {
            if let Some((claimed, release)) = block.borrow_mut().take() {
                claimed
                    .send(())
                    .expect("readiness owner observer should remain connected");
                release
                    .recv()
                    .expect("readiness owner release should remain connected");
            }
        });
        #[cfg(test)]
        assert!(
            !PANIC_AGENT_LAUNCH_PROFILE_READINESS_PROBE
                .with(|panic_next| panic_next.replace(false)),
            "injected launch profile readiness panic"
        );
        std::thread::scope(|scope| {
            self.entries
                .iter()
                .map(|profile| scope.spawn(move || profile.is_ready().then(|| profile.summary())))
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|probe| probe.join().ok().flatten())
                .collect()
        })
    }
}

impl Default for AgentLaunchProfiles {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            readiness_probe: Arc::new((
                StdMutex::new(AgentLaunchProfileReadiness::default()),
                std::sync::Condvar::new(),
            )),
        }
    }
}

impl AgentLaunchProfile {
    fn summary(&self) -> AgentLaunchProfileSummary {
        AgentLaunchProfileSummary {
            id: self.id.clone(),
            label: self.label.clone(),
            agent: self.agent.as_str().to_string(),
            provider_resume_import_supported: self.provider_config_dir.is_some()
                && matches!(self.agent, AgentKind::Codex | AgentKind::Claude),
            history_resume_supported: self.provider_resume_import_supported()
                || self.dsh_history.as_ref().is_some_and(|history| {
                    history.resume == Some(DshHistoryResumeCapability::ExactId)
                }),
        }
    }

    fn provider_resume_import_supported(&self) -> bool {
        self.provider_config_dir.is_some()
            && matches!(self.agent, AgentKind::Codex | AgentKind::Claude)
    }

    fn is_ready(&self) -> bool {
        let Ok(metadata) = std::fs::metadata(&self.agent_bin) else {
            return false;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
        if self
            .provider_config_dir
            .as_ref()
            .is_some_and(|path| !path.is_dir())
        {
            return false;
        }
        if self.readiness_args.is_empty() {
            return true;
        }
        let mut command = ProcessCommand::new(&self.agent_bin);
        command.args(&self.readiness_args);
        run_agent_profile_readiness(command, AGENT_LAUNCH_PROFILE_READINESS_TIMEOUT)
    }
}

fn run_agent_profile_readiness(mut command: ProcessCommand, timeout: Duration) -> bool {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    let started_at = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if started_at.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) | Err(_) => {
                // SAFETY: readiness commands are leaders of dedicated process groups.
                unsafe {
                    let _ = libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Validate a launch-profile array exactly as `AGENT_SESSION_LAUNCH_PROFILES`
/// is parsed at startup, returning the accepted ids in order.
pub(crate) fn validate_launch_profiles_json(raw: &str) -> Result<Vec<String>, CliError> {
    let profiles = AgentLaunchProfiles::from_json(raw)?;
    reject_retired_dsh_launch_profiles(&profiles)?;
    Ok(profiles
        .entries
        .into_iter()
        .map(|profile| profile.id)
        .collect())
}

fn reject_retired_dsh_launch_profiles(profiles: &AgentLaunchProfiles) -> Result<(), CliError> {
    if profiles
        .entries
        .iter()
        .any(|profile| profile.agent == AgentKind::Dsh)
    {
        return Err(invalid_agent_launch_profiles(
            "DSH launch profiles are retired; remove the profile and configure a supported agent",
        ));
    }
    Ok(())
}

fn valid_agent_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || (index > 0 && byte == b'-')
        })
}

fn invalid_agent_launch_profiles(message: &str) -> CliError {
    CliError::usage("invalid-agent-launch-profiles", message, None)
}

async fn bind_listener_and_start_delete_tombstone_cleanup(
    context: &CliContext,
    bind: SocketAddr,
) -> io::Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let cleanup_context = context.clone();
    std::thread::spawn(move || {
        let _ = cleanup_session_delete_tombstones(
            &cleanup_context,
            SESSION_DELETE_TOMBSTONE_CLEANUP_LIMIT,
        );
    });
    Ok(listener)
}

const SERVE_OWNER_LOCK: &str = "serve.lock";
// Long enough for a supervised restart whose predecessor is still exiting.
const SERVE_OWNER_GRACE: Duration = Duration::from_secs(2);

/// Exclusive, daemon-lifetime ownership of one canonical state root. Every
/// serve on a root shares its sessions, registry lock, and published local
/// endpoint, so a second one contends with and redirects the first
/// (sympoies/nils-cli#1856). The flock is released when the process exits, and
/// the descriptor is close-on-exec so spawned panes never inherit it.
struct ServeOwnership {
    _lock: fs::File,
}

enum ServeOwnershipError {
    Owned { root: PathBuf, holder: String },
    Unavailable(String),
}

fn acquire_serve_ownership(
    context: &CliContext,
    bind: SocketAddr,
) -> Result<ServeOwnership, ServeOwnershipError> {
    use std::io::{Seek, Write};
    use std::os::fd::AsRawFd;

    let unavailable = |what: &str, error: &dyn fmt::Display| {
        ServeOwnershipError::Unavailable(format!("failed to {what}: {error}"))
    };
    fs::create_dir_all(&context.state_dir)
        .map_err(|error| unavailable("create the state directory", &error))?;
    // Symlinked spellings of one root resolve to the same lock inode.
    let root = fs::canonicalize(&context.state_dir)
        .map_err(|error| unavailable("resolve the state directory", &error))?;
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(SERVE_OWNER_LOCK))
        .map_err(|error| unavailable("open the serve ownership lock", &error))?;
    let started = Instant::now();
    loop {
        // SAFETY: flock is called with a valid, owned file descriptor.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EWOULDBLOCK) {
            return Err(unavailable("lock the state directory", &error));
        }
        if started.elapsed() >= SERVE_OWNER_GRACE {
            let mut holder = String::new();
            let _ = lock.read_to_string(&mut holder);
            return Err(ServeOwnershipError::Owned {
                root,
                holder: holder.trim().to_string(),
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let owner = format!("pid {} on {bind}\n", std::process::id());
    lock.set_len(0)
        .and_then(|()| lock.rewind())
        .and_then(|()| lock.write_all(owner.as_bytes()))
        .map_err(|error| unavailable("record the serve owner", &error))?;
    Ok(ServeOwnership { _lock: lock })
}

/// `EX_TEMPFAIL`: the running binary was replaced or removed on disk, so the
/// supervisor should restart serve on the installed one (sympoies/nils-cli#1817).
const SERVE_BINARY_REPLACED_EXIT: i32 = 75;
const SERVE_BINARY_WATCH_ENV: &str = "AGENT_SESSION_SERVE_EXIT_ON_BINARY_CHANGE";
const SERVE_BINARY_WATCH_INTERVAL: Duration = Duration::from_secs(1);
const SERVE_BINARY_REPLACED_DRAIN: Duration = Duration::from_secs(10);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ServeStop {
    Interrupted,
    BinaryReplaced,
}

/// The installed file this serve was started from, by device and inode, and
/// the path it was invoked through.
struct ServeBinaryIdentity {
    invocation: PathBuf,
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl ServeBinaryIdentity {
    fn capture() -> io::Result<Self> {
        Self::from_invocation(std::path::absolute(std::env::current_exe()?)?)
    }

    fn from_invocation(invocation: PathBuf) -> io::Result<Self> {
        use std::os::unix::fs::MetadataExt;

        // macOS reports the invocation path, which is a link for a Homebrew
        // `bin/` install; watch the linked release file itself.
        let path = fs::canonicalize(&invocation)?;
        let metadata = fs::metadata(&path)?;
        Ok(Self {
            invocation,
            path,
            dev: metadata.dev(),
            ino: metadata.ino(),
        })
    }

    fn changed(&self) -> bool {
        use std::os::unix::fs::MetadataExt;

        // An upgrade that keeps the old release installed only repoints the
        // invocation link at the new one.
        if fs::canonicalize(&self.invocation).is_ok_and(|target| target != self.path) {
            return true;
        }
        match fs::metadata(&self.path) {
            Ok(metadata) => metadata.dev() != self.dev || metadata.ino() != self.ino,
            Err(error) => error.kind() == io::ErrorKind::NotFound,
        }
    }
}

fn serve_binary_identity() -> Option<ServeBinaryIdentity> {
    // Without the transient tmux scope, a supervisor cleaning up the exited
    // service cgroup would take every live session with it.
    if !crate::env_truthy("AGENT_SESSION_TMUX_SCOPE")
        || non_empty_env(SERVE_BINARY_WATCH_ENV).is_some_and(|value| !crate::is_truthy_flag(&value))
    {
        return None;
    }
    match ServeBinaryIdentity::capture() {
        Ok(identity) => Some(identity),
        Err(error) => {
            eprintln!("warning: serve-binary-watch-unavailable: {error}");
            None
        }
    }
}

async fn serve_binary_replaced(identity: Option<ServeBinaryIdentity>) {
    let Some(identity) = identity else {
        return std::future::pending().await;
    };
    let mut interval = tokio::time::interval(SERVE_BINARY_WATCH_INTERVAL);
    loop {
        interval.tick().await;
        if identity.changed() {
            eprintln!(
                "warning: serve-binary-replaced: {} changed on disk; draining for up to {}s, \
                 then exiting {SERVE_BINARY_REPLACED_EXIT} for a supervisor restart",
                identity.path.display(),
                SERVE_BINARY_REPLACED_DRAIN.as_secs()
            );
            return;
        }
    }
}

async fn serve_stop_signal(
    identity: Option<ServeBinaryIdentity>,
    stop: watch::Sender<Option<ServeStop>>,
) {
    let reason = tokio::select! {
        _ = tokio::signal::ctrl_c() => ServeStop::Interrupted,
        () = serve_binary_replaced(identity) => ServeStop::BinaryReplaced,
    };
    let _ = stop.send(Some(reason));
}

/// Bounds the drain after a binary replacement: long-lived attach and activity
/// streams would otherwise hold graceful shutdown open indefinitely.
async fn serve_binary_replaced_drain_deadline(mut stop: watch::Receiver<Option<ServeStop>>) {
    if stop
        .wait_for(|reason| *reason == Some(ServeStop::BinaryReplaced))
        .await
        .is_err()
    {
        return std::future::pending().await;
    }
    tokio::time::sleep(SERVE_BINARY_REPLACED_DRAIN).await;
    eprintln!("warning: serve-binary-replaced: drain deadline elapsed; closing open connections");
}

pub fn run_serve(context: &CliContext, args: cli::ServeArgs) -> i32 {
    // First, while the process is still single-threaded: a check exits here,
    // and an applied config is visible to every environment read below.
    if let Some(config) = args.config.as_deref()
        && let std::ops::ControlFlow::Break(code) =
            crate::serve_config::apply_for_serve(config, args.check, args.format)
    {
        return code;
    }
    // Owner-run CLI commands (`account`, `resume`) do not inherit this
    // environment; record the effective brokers where they can adopt them.
    if let Err(err) = crate::account_broker::publish_serve_brokers(&context.state_dir) {
        eprintln!(
            "warning: account brokers could not be recorded for CLI commands: {}",
            err.kind()
        );
    }
    let bind: SocketAddr = match args.bind.parse() {
        Ok(addr) => addr,
        Err(err) => {
            eprintln!("error: invalid --bind {}: {err}", args.bind);
            return exit::USAGE;
        }
    };
    if !bind.ip().is_loopback() && !args.allow_non_loopback {
        eprintln!(
            "error: refusing to bind a non-loopback address ({bind}); this endpoint drives a \
             remote shell. Keep it tailnet-only behind `tailscale serve`, or pass \
             --allow-non-loopback to override deliberately."
        );
        return exit::USAGE;
    }
    if !bind.ip().is_loopback() {
        eprintln!(
            "warning: binding a non-loopback address ({bind}) exposes a remote shell; \
             keep it tailnet-only behind `tailscale serve` and off the public internet"
        );
    }

    let token = match resolve_serve_token(&args) {
        Ok(token) => token,
        Err(err) => {
            eprintln!("error: {err}");
            return exit::USAGE;
        }
    };
    if token.is_none() {
        eprintln!(
            "warning: no --token / --token-stdin / AGENT_SESSION_TOKEN set; authenticated endpoints are disabled"
        );
    }

    let max_attachment_bytes = match resolve_max_attachment_bytes() {
        Ok(value) => value,
        Err(message) => {
            eprintln!("error: {message}");
            return exit::USAGE;
        }
    };

    let machine = crate::board::machine_identity(args.machine.clone(), context);
    let board = crate::board::serve_enabled(args.board);

    let federation = match crate::coordination::remote::Config::from_environment(&machine) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("error: {}", error.message());
            return exit::USAGE;
        }
    };
    if let Some(config) = &federation
        && token
            .as_deref()
            .is_none_or(|value| value == config.token || value == config.ingress_token)
    {
        eprintln!("error: federation requires a distinct operator token");
        return exit::USAGE;
    }
    let launch_profiles = match AgentLaunchProfiles::from_environment() {
        Ok(profiles) => profiles,
        Err(err) => {
            eprintln!("error: {}", err.message());
            return exit::USAGE;
        }
    };

    // Held until the process exits, before any recovery, fencing, endpoint
    // write, or background loop touches the shared state root.
    let _ownership = match acquire_serve_ownership(context, bind) {
        Ok(ownership) => ownership,
        Err(ServeOwnershipError::Owned { root, holder }) => {
            eprintln!(
                "error: serve-state-root-owned: {} is already served by {}; give a development \
                 or test serve its own --state-dir or AGENT_SESSION_STATE_DIR",
                root.display(),
                if holder.is_empty() {
                    "another serve"
                } else {
                    holder.as_str()
                }
            );
            return exit::UNAVAILABLE;
        }
        Err(ServeOwnershipError::Unavailable(message)) => {
            eprintln!("error: {message}");
            return exit::RUNTIME;
        }
    };

    let binary_identity = serve_binary_identity();
    let tmux_bin = resolve_tmux_bin(args.tmux_bin.as_deref());
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("error: failed to build async runtime: {err}");
            return exit::RUNTIME;
        }
    };

    runtime.block_on(async move {
        if let Err(err) = crate::recover_interrupted_tmux_terminations(context) {
            eprintln!(
                "error: failed to recover interrupted session termination: {}",
                err.code()
            );
            return exit::RUNTIME;
        }
        if let Err(err) = fence_codex_controls_before_listen(context, &tmux_bin) {
            eprintln!(
                "error: failed to fence Codex account controls: {}",
                err.code()
            );
            return exit::RUNTIME;
        }
        let session_collector = default_session_collector();
        let activity_broker = ActivityBroker::start(
            context.clone(),
            machine.clone(),
            tmux_bin.clone(),
            session_collector.clone(),
        )
        .await;
        let history_catalog = Arc::new(
            HistoryCatalog::new(
                launch_profiles.history_sources(),
                provider_history::archive_root(&context.state_dir),
                provider_history::star_root(&context.state_dir),
            )
            .with_session_state_dir(context.state_dir.clone())
            .with_dsh_sources(launch_profiles.dsh_history_sources()),
        );
        let state = Arc::new(ServeState {
            context: context.clone(),
            machine,
            token,
            board,
            federation,
            mailbox_services_file: args.mailbox_services_file.clone().or_else(|| {
                crate::non_empty_env("AGENT_SESSION_MAILBOX_SERVICES_FILE").map(PathBuf::from)
            }),
            mailbox_service_workers: Arc::new(tokio::sync::Semaphore::new(16)),
            max_attachment_bytes,
            tmux_bin,
            attach_brokers: AttachBrokerRegistry::default(),
            provider_prompt_discovery: Arc::new(ProviderPromptDiscoveryRegistry::default()),
            activity_broker,
            codex_controls: Arc::new(StdMutex::new(HashMap::new())),
            account_switches: AccountSwitchRegistry::default(),
            session_collector,
            launch_profiles,
            history_catalog,
            retitle: Arc::new(crate::retitle::RetitleService::from_environment()),
            retitle_v3_manual_waiters: Arc::new(StdMutex::new(HashMap::new())),
            retitle_v3_continuations: Arc::new(StdMutex::new(HashSet::new())),
            managed_history_titles: Arc::new(StdMutex::new(ManagedHistoryTitleCache::default())),
            coordination_wait_workers: Arc::new(tokio::sync::Semaphore::new(
                COORDINATION_WAIT_WORKER_LIMIT,
            )),
            coordination_notification_wake: Arc::new(tokio::sync::Notify::new()),
            remote_outbox_wake: Arc::new(tokio::sync::Notify::new()),
            provider_usage: Arc::new(crate::usage::UsageService::from_environment()),
            board_programs: Arc::new(crate::board::ProgramCache::from_environment()),
        });
        let listener = match bind_listener_and_start_delete_tombstone_cleanup(context, bind).await {
            Ok(listener) => listener,
            Err(err) => {
                eprintln!("error: failed to bind {bind}: {err}");
                return exit::RUNTIME;
            }
        };
        eprintln!(
            "agent-session serve listening on http://{bind} (machine={})",
            state.machine
        );
        if let Err(error) = crate::coordination::remote::write_endpoint(
            &state.context,
            listener.local_addr().expect("bound address"),
        ) {
            eprintln!("error: {}", error.message());
            return exit::RUNTIME;
        }
        let app = router(state.clone());
        let federation_task = tokio::spawn(remote_outbox::run(state.clone()));
        let codex_control_task = tokio::spawn(account_drain::run(state.clone()));
        let auto_resume_task = tokio::spawn(auto_resume_loop(state.clone()));
        let auth_incident_task = tokio::spawn(auth_incident_loop(state.clone()));
        let auto_retitle_task = tokio::spawn(auto_retitle_loop(state.clone()));
        let retitle_v3_recovery_task =
            tokio::spawn(recover_retitle_v3_operations_once(state.clone()));
        let coordination_notification_task =
            tokio::spawn(coordination_notification_loop(state.clone()));
        let (stop_tx, stop_rx) = watch::channel(None);
        // Peer addresses let the activity hook ingress admit loopback only.
        let serve = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(serve_stop_signal(binary_identity, stop_tx));
        let result = tokio::select! {
            result = serve.into_future() => result,
            () = serve_binary_replaced_drain_deadline(stop_rx.clone()) => Ok(()),
        };
        let binary_replaced = *stop_rx.borrow() == Some(ServeStop::BinaryReplaced);
        federation_task.abort();
        let _ = federation_task.await;
        auth_incident_task.abort();
        let _ = auth_incident_task.await;
        auto_resume_task.abort();
        let _ = auto_resume_task.await;
        auto_retitle_task.abort();
        let _ = auto_retitle_task.await;
        retitle_v3_recovery_task.abort();
        let _ = retitle_v3_recovery_task.await;
        coordination_notification_task.abort();
        let _ = coordination_notification_task.await;
        codex_control_task.abort();
        let _ = codex_control_task.await;
        state.attach_brokers.shutdown_all().await;
        match result {
            Ok(()) if binary_replaced => SERVE_BINARY_REPLACED_EXIT,
            Ok(()) => exit::SUCCESS,
            Err(err) => {
                eprintln!("error: serve failed: {err}");
                exit::RUNTIME
            }
        }
    })
}

fn router(state: Arc<ServeState>) -> Router {
    Router::new()
        .route(crate::coordination::audit::ROUTE, get(mail_audit_handler))
        .merge(console_service::routes())
        .route(
            "/coordination/messages/receive/v1",
            post(remote_receive_handler),
        )
        .route(
            "/sessions/{id}/messages/remote/v1",
            post(remote_submit_handler),
        )
        .route(
            "/sessions/{id}/messages/peers/v1",
            get(remote_peers_handler),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/delivery/v1",
            get(remote_delivery_handler),
        )
        .route(
            "/shells/{owner}",
            get(shell::status).post(shell::open).delete(shell::close),
        )
        .route("/shells/{owner}/attach", get(shell::attach))
        .route("/healthz", get(healthz))
        .route("/board/v1", get(board_snapshot_handler))
        .route("/board/programs/v1", get(board_programs_handler))
        .route("/board/closed/v1", get(board_closed_handler))
        .route("/sessions/{id}/board/v1", get(board_relay_handler))
        .route(
            "/sessions/{id}/console-start/v1",
            post(console_start_handler),
        )
        .route("/sessions", get(list_handler).post(create::create_handler))
        .merge(resume_start::routes())
        .route("/history/sessions", get(history_list_handler))
        .merge(history_images::routes())
        .route("/history/sessions/{id}/star", post(history_star_handler))
        .route(
            "/history/sessions/{id}/resume",
            post(history_resume_handler),
        )
        .route("/codex/accounts", get(codex_accounts_handler))
        .route("/claude/accounts", get(claude_accounts_handler))
        .route("/retitle/readiness", get(retitle_readiness_handler))
        .route("/clipboard/unwrap/v1", post(clipboard_unwrap_handler))
        .merge(session_info::routes())
        .route(
            "/sessions/{id}/display-metadata",
            post(display_metadata_handler),
        )
        .route("/retitle/v3/readiness", get(retitle_v3_readiness_handler))
        .route(
            "/sessions/{id}/retitle-v3/readiness",
            get(session_retitle_v3_readiness_handler),
        )
        .route(
            "/sessions/{id}/retitle-v3",
            post(session_retitle_v3_handler),
        )
        .route(
            "/sessions/{id}/retitle-v3/operations",
            get(session_retitle_v3_receipts_handler),
        )
        .route(
            "/sessions/{id}/retitle-v3/operations/{operation_hash}",
            get(session_retitle_v3_operation_handler),
        )
        .route("/activity/events", get(activity_events_handler))
        .route("/usage", get(usage_handler))
        .route("/usage/v1", get(usage_v1_handler))
        .route("/codex/reset/v1", post(codex_reset_v1_handler))
        .route("/claude/reset/v1", post(claude_reset_v1_handler))
        .route("/workdirs", get(workdirs_handler))
        .route("/repos/remote-url", get(repo_remote_url_handler))
        .route("/sessions/{id}/glance", get(glance_handler))
        .route(
            "/sessions/{id}/work-context/v1",
            get(coordination_context_show_handler),
        )
        .route(
            "/sessions/{id}/work-context/check/v1",
            post(coordination_context_check_handler),
        )
        .route(
            "/coordination/work-context/check/v1",
            post(coordination_registry_context_check_handler),
        )
        .route(
            "/sessions/{id}/work-context/claim/v1",
            post(coordination_context_claim_handler),
        )
        .route(
            "/sessions/{id}/work-context/renew/v1",
            post(coordination_context_renew_handler),
        )
        .route(
            "/sessions/{id}/work-context/release/v1",
            post(coordination_context_release_handler),
        )
        .route(
            "/sessions/{id}/work-context/admit/v1",
            post(coordination_context_admit_handler),
        )
        .route(
            "/sessions/{id}/work-context/complete/v1",
            post(coordination_context_complete_handler),
        )
        .route(
            "/sessions/{id}/work-context/reconcile/v1",
            post(coordination_context_reconcile_handler),
        )
        .route("/sessions/{id}/broker/v1", get(coordination_broker_handler))
        .route(
            "/sessions/{id}/broker/adopt/v1",
            post(coordination_broker_adopt_handler),
        )
        .route(
            "/sessions/{id}/broker/adopt/v2",
            post(coordination_broker_adopt_handler),
        )
        .route(
            "/sessions/{id}/broker/reconcile/v1",
            post(coordination_broker_reconcile_handler),
        )
        .route(
            "/sessions/{id}/broker/reconcile/v2",
            post(coordination_broker_reconcile_handler),
        )
        .route(
            "/sessions/{id}/operations/{lease_id}/operator-reconcile/v1",
            post(coordination_operator_reconcile_handler),
        )
        .route(
            "/sessions/{id}/activity/provider-turn/operator-reconcile/v1",
            post(coordination_operator_provider_turn_reconcile_handler),
        )
        .route(
            "/sessions/{id}/messages/v1",
            get(coordination_inbox_handler).post(coordination_send_handler),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/v1",
            get(coordination_message_show_handler),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/ack/v1",
            post(coordination_ack_handler),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/reply/v1",
            post(coordination_reply_handler),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/wait/v1",
            get(coordination_wait_handler),
        )
        .route("/sessions/{id}/buffer", get(buffer_handler))
        .route("/sessions/{id}/send", post(send_handler))
        .route("/sessions/{id}/prompt", post(structured_prompt_handler))
        .route("/sessions/{id}/prompt/v2", post(prompt_receipts::submit))
        .route(
            "/sessions/{id}/prompt/v2/receipts/{request_id}",
            get(prompt_receipts::lookup),
        )
        .route("/sessions/{id}/resume", post(resume_handler))
        .route("/sessions/{id}/archive", post(archive_handler))
        .route("/sessions/{id}/maintenance", get(maintenance_handler))
        .route(
            "/sessions/{id}/maintenance/actions",
            post(maintenance_action_handler),
        )
        .route(
            "/sessions/{id}/account",
            axum::routing::put(session_account_handler),
        )
        .route(
            "/sessions/{id}/auto-resume",
            get(auto_resume_status_handler)
                .put(auto_resume_set_handler)
                .delete(auto_resume_cancel_handler),
        )
        .route(
            "/sessions/{id}/attachments",
            post(upload_attachment_handler),
        )
        .route("/sessions/{id}/attach", get(attach_handler))
        .route(
            "/sessions/{id}",
            patch(update_session_handler).delete(delete_handler),
        )
        .merge(crate::activity_ingress::router(
            state.context.clone(),
            state.machine.clone(),
        ))
        .with_state(state)
}

#[derive(Debug)]
enum ServeTokenError {
    EmptyStdin,
    MultipleStdinTokens,
    StdinTooLarge,
    ReadStdin(io::Error),
}

impl fmt::Display for ServeTokenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyStdin => write!(f, "--token-stdin received an empty token"),
            Self::MultipleStdinTokens => write!(f, "--token-stdin expects exactly one token"),
            Self::StdinTooLarge => write!(f, "--token-stdin input exceeds 8192 bytes"),
            Self::ReadStdin(err) => write!(f, "failed to read --token-stdin input: {err}"),
        }
    }
}

fn resolve_serve_token(args: &cli::ServeArgs) -> Result<Option<String>, ServeTokenError> {
    if args.token_stdin {
        return read_token_from_stdin(io::stdin().lock()).map(Some);
    }
    Ok(sanitize_token(args.token.clone()).or_else(|| non_empty_env("AGENT_SESSION_TOKEN")))
}

fn resolve_max_attachment_bytes() -> Result<u64, String> {
    let Some(raw) = non_empty_env(MAX_ATTACHMENT_BYTES_ENV) else {
        return Ok(DEFAULT_MAX_ATTACHMENT_BYTES);
    };
    let value = raw.parse::<u64>().map_err(|_| {
        format!(
            "{MAX_ATTACHMENT_BYTES_ENV} must be an integer from 1 to {MAX_CONFIGURED_ATTACHMENT_BYTES}"
        )
    })?;
    if value == 0 || value > MAX_CONFIGURED_ATTACHMENT_BYTES {
        return Err(format!(
            "{MAX_ATTACHMENT_BYTES_ENV} must be an integer from 1 to {MAX_CONFIGURED_ATTACHMENT_BYTES}"
        ));
    }
    Ok(value)
}

fn read_token_from_stdin<R: Read>(reader: R) -> Result<String, ServeTokenError> {
    let mut input = String::new();
    let mut limited = reader.take(MAX_STDIN_TOKEN_BYTES + 1);
    limited
        .read_to_string(&mut input)
        .map_err(ServeTokenError::ReadStdin)?;
    if input.len() as u64 > MAX_STDIN_TOKEN_BYTES {
        return Err(ServeTokenError::StdinTooLarge);
    }

    let token = input.trim();
    if token.is_empty() {
        return Err(ServeTokenError::EmptyStdin);
    }
    if token.contains(['\n', '\r']) {
        return Err(ServeTokenError::MultipleStdinTokens);
    }
    Ok(token.to_string())
}

/// Treat an empty/whitespace explicit `--token` as "no token" (fail closed),
/// matching the env-var sanitization in `non_empty_env`.
fn sanitize_token(token: Option<String>) -> Option<String> {
    token.filter(|value| !value.trim().is_empty())
}

// --- activity stream ---------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
struct ActivityStreamSession {
    id: String,
    turn_state: Option<crate::activity::StreamTurnState>,
}

#[derive(Clone, Debug, Serialize)]
struct ActivityStreamEvent {
    schema_version: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    stream_id: String,
    sequence: u64,
    machine: String,
    observed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sessions: Option<Vec<ActivityStreamSession>>,
}

impl ActivityStreamEvent {
    fn sse_id(&self) -> String {
        format!("{}:{}", self.stream_id, self.sequence)
    }
}

#[derive(Debug)]
struct ActivityStreamFrame {
    kind: &'static str,
    sequence: u64,
    #[cfg(test)]
    event: ActivityStreamEvent,
    sse: Bytes,
    wire_bytes: usize,
}

#[derive(Default)]
struct ActivityFrameEncoder {
    #[cfg(test)]
    count: AtomicUsize,
}

impl ActivityFrameEncoder {
    fn encode(&self, event: ActivityStreamEvent) -> Arc<ActivityStreamFrame> {
        let payload = serde_json::to_vec(&event).expect("activity stream event serializes");
        #[cfg(test)]
        self.count.fetch_add(1, Ordering::SeqCst);
        let id = event.sse_id();
        let mut sse = Vec::with_capacity(id.len() + event.kind.len() + payload.len() + 24);
        sse.extend_from_slice(b"id: ");
        sse.extend_from_slice(id.as_bytes());
        sse.extend_from_slice(b"\nevent: ");
        sse.extend_from_slice(event.kind.as_bytes());
        sse.extend_from_slice(b"\ndata: ");
        sse.extend_from_slice(&payload);
        sse.extend_from_slice(b"\n\n");
        let wire_bytes = sse.len();
        Arc::new(ActivityStreamFrame {
            kind: event.kind,
            sequence: event.sequence,
            #[cfg(test)]
            event,
            sse: Bytes::from(sse),
            wire_bytes,
        })
    }

    #[cfg(test)]
    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

impl ActivityStreamFrame {
    fn replay_bytes(&self) -> usize {
        self.wire_bytes
    }
}

#[cfg(test)]
impl std::ops::Deref for ActivityStreamFrame {
    type Target = ActivityStreamEvent;

    fn deref(&self) -> &Self::Target {
        &self.event
    }
}

struct ActivityEventState {
    history: VecDeque<Arc<ActivityStreamFrame>>,
    history_bytes: usize,
    latest_sessions: Vec<ActivityStreamSession>,
    latest_observed_at: String,
    latest_snapshot_oversized: bool,
    cached_snapshot: Option<Arc<ActivityStreamFrame>>,
    cached_reset: Option<Arc<ActivityStreamFrame>>,
}

struct ActivityEventLog {
    stream_id: String,
    machine: String,
    encoder: ActivityFrameEncoder,
    sequence: AtomicU64,
    state: StdMutex<ActivityEventState>,
    sender: broadcast::Sender<Arc<ActivityStreamFrame>>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
}

impl ActivityEventLog {
    #[cfg(test)]
    fn new(machine: String, sessions: Vec<ActivityStreamSession>) -> Arc<Self> {
        let (lifecycle, _) = watch::channel(ActivityBrokerLifecycle::Ready);
        Self::new_with_lifecycle(machine, sessions, lifecycle)
    }

    fn new_with_lifecycle(
        machine: String,
        sessions: Vec<ActivityStreamSession>,
        lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    ) -> Arc<Self> {
        let stream_id = uuid::Uuid::new_v4().to_string();
        let observed_at = activity_observed_at();
        let encoder = ActivityFrameEncoder::default();
        let initial = encoder.encode(ActivityStreamEvent {
            schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
            kind: "snapshot",
            stream_id: stream_id.clone(),
            sequence: 1,
            machine: machine.clone(),
            observed_at: observed_at.clone(),
            reason: None,
            sessions: Some(sessions.clone()),
        });
        let latest_snapshot_oversized =
            initial.replay_bytes() > ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY;
        let initial = if latest_snapshot_oversized {
            encoder.encode(ActivityStreamEvent {
                schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
                kind: "reset",
                stream_id: stream_id.clone(),
                sequence: 1,
                machine: machine.clone(),
                observed_at: observed_at.clone(),
                reason: Some(ACTIVITY_STREAM_OVERSIZED_REASON),
                sessions: None,
            })
        } else {
            initial
        };
        let mut history = VecDeque::with_capacity(ACTIVITY_STREAM_REPLAY_CAPACITY);
        let initial_bytes = initial.replay_bytes();
        let history_bytes = if initial_bytes <= ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY {
            history.push_back(initial.clone());
            initial_bytes
        } else {
            0
        };
        let (sender, _) = broadcast::channel(ACTIVITY_STREAM_BROADCAST_CAPACITY);
        Arc::new(Self {
            stream_id,
            machine,
            encoder,
            sequence: AtomicU64::new(1),
            state: StdMutex::new(ActivityEventState {
                history,
                history_bytes,
                latest_sessions: sessions,
                latest_observed_at: observed_at,
                latest_snapshot_oversized,
                cached_snapshot: Some(initial.clone()),
                cached_reset: (initial.kind == "reset").then(|| initial.clone()),
            }),
            sender,
            lifecycle,
        })
    }

    fn publish_heartbeat(&self) {
        let mut state = self.state.lock().expect("activity event state lock");
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let event = self.encoder.encode(ActivityStreamEvent {
            schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
            kind: "heartbeat",
            stream_id: self.stream_id.clone(),
            sequence,
            machine: self.machine.clone(),
            observed_at: activity_observed_at(),
            reason: None,
            sessions: None,
        });
        state.cached_snapshot = None;
        state.cached_reset = None;
        self.retain_and_broadcast_locked(&mut state, event);
    }

    fn retain_and_broadcast_locked(
        &self,
        state: &mut ActivityEventState,
        event: Arc<ActivityStreamFrame>,
    ) {
        state.history_bytes = state.history_bytes.saturating_add(event.replay_bytes());
        state.history.push_back(event.clone());
        while state.history.len() > ACTIVITY_STREAM_REPLAY_CAPACITY
            || state.history_bytes > ACTIVITY_STREAM_REPLAY_BYTE_CAPACITY
        {
            if let Some(evicted) = state.history.pop_front() {
                state.history_bytes = state.history_bytes.saturating_sub(evicted.replay_bytes());
            }
        }
        // A bounded broadcast is deliberately lossy. A lagged subscriber gets
        // a full reset and polling remains the convergence path.
        let _ = self.sender.send(event);
    }

    fn subscribe(self: &Arc<Self>, last_event_id: Option<&str>) -> ActivitySubscription {
        // Subscribe before taking the replay snapshot. Events racing with the
        // snapshot may be present in both sources; the sequence filter removes
        // those duplicates without losing an event.
        let receiver = self.sender.subscribe();
        let mut state = self.state.lock().expect("activity event state lock");
        let current_sequence = self.sequence.load(Ordering::SeqCst);
        let pending = match last_event_id.and_then(parse_activity_event_id) {
            None if last_event_id.is_none() => VecDeque::from([state
                .cached_snapshot
                .clone()
                .filter(|frame| frame.sequence == current_sequence)
                .unwrap_or_else(|| self.publish_full_event_locked(&mut state, "snapshot"))]),
            Some((stream_id, sequence))
                if stream_id == self.stream_id && sequence <= current_sequence =>
            {
                let replay: VecDeque<_> = state
                    .history
                    .iter()
                    .filter(|event| event.sequence > sequence)
                    .cloned()
                    .collect();
                let contiguous = if sequence == current_sequence {
                    replay.is_empty()
                } else {
                    replay
                        .front()
                        .is_some_and(|event| event.sequence == sequence.saturating_add(1))
                        && replay
                            .back()
                            .is_some_and(|event| event.sequence == current_sequence)
                        && replay
                            .iter()
                            .zip(replay.iter().skip(1))
                            .all(|(left, right)| right.sequence == left.sequence.saturating_add(1))
                };
                if contiguous {
                    replay
                } else {
                    VecDeque::from([self.publish_full_event_locked(&mut state, "reset")])
                }
            }
            _ => VecDeque::from([self.publish_full_event_locked(&mut state, "reset")]),
        };
        ActivitySubscription {
            log: self.clone(),
            pending,
            receiver,
            lifecycle: self.lifecycle.subscribe(),
            last_sequence: last_event_id
                .and_then(parse_activity_event_id)
                .filter(|(stream_id, sequence)| {
                    stream_id == &self.stream_id && *sequence <= current_sequence
                })
                .map_or(0, |(_, sequence)| sequence),
            reconciliation_sent: false,
            _subscriber_permit: None,
        }
    }

    fn reset_event(&self) -> Arc<ActivityStreamFrame> {
        let mut state = self.state.lock().expect("activity event state lock");
        let current_sequence = self.sequence.load(Ordering::SeqCst);
        state
            .cached_reset
            .clone()
            .filter(|frame| frame.sequence == current_sequence)
            .unwrap_or_else(|| self.publish_full_event_locked(&mut state, "reset"))
    }

    fn degraded_reset_event(&self) -> Arc<ActivityStreamFrame> {
        let state = self.state.lock().expect("activity event state lock");
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        self.uncached_full_event_locked(&state, sequence, "reset")
    }

    fn publish_full_event_locked(
        &self,
        state: &mut ActivityEventState,
        kind: &'static str,
    ) -> Arc<ActivityStreamFrame> {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        state.cached_snapshot = None;
        state.cached_reset = None;
        let frame = self.uncached_full_event_locked(state, sequence, kind);
        if kind == "snapshot" {
            state.cached_snapshot = Some(frame.clone());
        } else {
            state.cached_reset = Some(frame.clone());
        }
        if state.latest_snapshot_oversized {
            state.cached_snapshot = Some(frame.clone());
            state.cached_reset = Some(frame.clone());
        }
        self.retain_and_broadcast_locked(state, frame.clone());
        frame
    }

    fn uncached_full_event_locked(
        &self,
        state: &ActivityEventState,
        sequence: u64,
        kind: &'static str,
    ) -> Arc<ActivityStreamFrame> {
        if state.latest_snapshot_oversized {
            return self.content_free_reset_frame(sequence, &state.latest_observed_at);
        }
        self.encoder.encode(ActivityStreamEvent {
            schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
            kind,
            stream_id: self.stream_id.clone(),
            sequence,
            machine: self.machine.clone(),
            observed_at: state.latest_observed_at.clone(),
            reason: None,
            sessions: Some(state.latest_sessions.clone()),
        })
    }

    fn content_free_reset_frame(
        &self,
        sequence: u64,
        observed_at: &str,
    ) -> Arc<ActivityStreamFrame> {
        self.encoder.encode(ActivityStreamEvent {
            schema_version: ACTIVITY_STREAM_EVENT_SCHEMA_VERSION,
            kind: "reset",
            stream_id: self.stream_id.clone(),
            sequence,
            machine: self.machine.clone(),
            observed_at: observed_at.to_string(),
            reason: Some(ACTIVITY_STREAM_OVERSIZED_REASON),
            sessions: None,
        })
    }
}

struct ActivitySubscription {
    log: Arc<ActivityEventLog>,
    pending: VecDeque<Arc<ActivityStreamFrame>>,
    receiver: broadcast::Receiver<Arc<ActivityStreamFrame>>,
    lifecycle: watch::Receiver<ActivityBrokerLifecycle>,
    last_sequence: u64,
    reconciliation_sent: bool,
    _subscriber_permit: Option<OwnedSemaphorePermit>,
}

impl ActivitySubscription {
    fn reconcile_or_stop(&mut self) -> Option<Arc<ActivityStreamFrame>> {
        if self.reconciliation_sent {
            None
        } else {
            self.reconciliation_sent = true;
            Some(self.log.degraded_reset_event())
        }
    }

    async fn next_event(&mut self) -> Option<Arc<ActivityStreamFrame>> {
        loop {
            if *self.lifecycle.borrow() != ActivityBrokerLifecycle::Ready {
                return self.reconcile_or_stop();
            }
            if let Some(event) = self.pending.pop_front() {
                if event.sequence > self.last_sequence || event.kind == "reset" {
                    self.last_sequence = event.sequence;
                    return Some(event);
                }
                continue;
            }
            tokio::select! {
                biased;
                lifecycle = self.lifecycle.changed() => {
                    if lifecycle.is_err()
                        || *self.lifecycle.borrow() != ActivityBrokerLifecycle::Ready
                    {
                        return self.reconcile_or_stop();
                    }
                }
                received = self.receiver.recv() => match received {
                    Ok(event) if event.sequence > self.last_sequence => {
                        self.last_sequence = event.sequence;
                        return Some(event);
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let event = self.log.reset_event();
                        self.last_sequence = event.sequence;
                        return Some(event);
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        return self.reconcile_or_stop();
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityBrokerLifecycle {
    Starting,
    Ready,
    Degraded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivitySubscribeError {
    Unavailable,
    Capacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ActivityChange {
    Refresh,
    Rescan,
    RootLost,
}

type ActivityWatchRearm = Arc<dyn Fn() -> bool + Send + Sync>;

#[derive(Default)]
struct ActivityChangeLoopControls {
    refresh_started: Option<mpsc::UnboundedSender<tokio::time::Instant>>,
    watch_rearm: Option<ActivityWatchRearm>,
}

fn create_activity_watcher(
    change_tx: mpsc::Sender<ActivityChange>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    sessions_root: PathBuf,
) -> Option<RecommendedWatcher> {
    let callback_root = sessions_root.clone();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<NotifyEvent>| {
        activity_watch_callback(result, &change_tx, &lifecycle, &callback_root);
    })
    .ok()?;
    watcher
        .watch(&sessions_root, RecursiveMode::Recursive)
        .ok()?;
    Some(watcher)
}

struct ActivityBroker {
    log: Arc<ActivityEventLog>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    subscribers: Arc<Semaphore>,
    _watcher: Arc<StdMutex<Option<RecommendedWatcher>>>,
    /// Handles for the change and heartbeat loops, retained only so a caller can
    /// wait for them to stop. A served broker lives for the whole process and
    /// never uses this; see [`ActivityBroker::shutdown`].
    loops: StdMutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl ActivityBroker {
    async fn start(
        context: CliContext,
        machine: String,
        tmux_bin: PathBuf,
        session_collector: SessionCollector,
    ) -> Arc<Self> {
        let (lifecycle, _) = watch::channel(ActivityBrokerLifecycle::Starting);
        let (change_tx, change_rx) = mpsc::channel(1);
        let sessions_root = context.state_dir.join("sessions");
        let watcher = std::fs::create_dir_all(&sessions_root).ok().and_then(|()| {
            create_activity_watcher(change_tx.clone(), lifecycle.clone(), sessions_root.clone())
        });
        if watcher.is_none() {
            eprintln!(
                "warning: activity filesystem notifications unavailable; session polling will reconcile"
            );
        }
        let initial_context = context.clone();
        let initial_tmux = tmux_bin.clone();
        let initial_collector = session_collector.clone();
        let (initial, snapshot_available) = match tokio::task::spawn_blocking(move || {
            collect_activity_stream_sessions(&initial_collector, &initial_context, &initial_tmux)
        })
        .await
        {
            Ok(Ok(sessions)) => (sessions, true),
            Ok(Err(_)) | Err(_) => {
                eprintln!(
                    "warning: initial activity stream snapshot unavailable; session polling will reconcile"
                );
                (Vec::new(), false)
            }
        };
        let log = ActivityEventLog::new_with_lifecycle(machine, initial, lifecycle.clone());
        let ready = watcher.is_some()
            && snapshot_available
            && *lifecycle.borrow() != ActivityBrokerLifecycle::Degraded;
        lifecycle.send_replace(if ready {
            ActivityBrokerLifecycle::Ready
        } else {
            ActivityBrokerLifecycle::Degraded
        });
        let watcher = Arc::new(StdMutex::new(watcher));
        let broker = Arc::new(Self {
            log,
            lifecycle,
            subscribers: Arc::new(Semaphore::new(ACTIVITY_STREAM_SUBSCRIBER_CAPACITY)),
            _watcher: watcher.clone(),
            loops: StdMutex::new(Vec::new()),
        });
        if ready {
            let rearm_watcher = watcher;
            let rearm_tx = change_tx;
            let rearm_lifecycle = broker.lifecycle.clone();
            let rearm_root = sessions_root;
            let watch_rearm: ActivityWatchRearm = Arc::new(move || {
                if std::fs::create_dir_all(&rearm_root).is_err() {
                    return false;
                }
                let Some(rearmed) = create_activity_watcher(
                    rearm_tx.clone(),
                    rearm_lifecycle.clone(),
                    rearm_root.clone(),
                ) else {
                    return false;
                };
                *rearm_watcher.lock().expect("activity watcher lock") = Some(rearmed);
                true
            });
            let change_loop = tokio::spawn(activity_change_loop_inner(
                broker.log.clone(),
                broker.lifecycle.clone(),
                session_collector,
                context,
                tmux_bin,
                change_rx,
                ActivityChangeLoopControls {
                    refresh_started: None,
                    watch_rearm: Some(watch_rearm),
                },
            ));
            let heartbeat_loop = tokio::spawn(activity_heartbeat_loop(
                broker.log.clone(),
                broker.lifecycle.clone(),
            ));
            broker
                .loops
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .extend([change_loop, heartbeat_loop]);
        }
        broker
    }

    /// Stops the background loops and waits for them to actually be gone.
    ///
    /// The change loop owns a watch-rearm closure that calls `create_dir_all` on
    /// the sessions root whenever the root disappears — correct for a served
    /// broker, but fatal for a fixture: removing the `TempDir` *is* a root loss,
    /// so a still-running loop re-creates the directory right after teardown and
    /// leaks it. `abort()` alone does not close that window because it only
    /// schedules cancellation; the `await` is what makes this a barrier.
    #[cfg(test)]
    async fn shutdown(&self) {
        let handles: Vec<_> = self
            .loops
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain(..)
            .collect();
        for handle in handles {
            handle.abort();
            let _ = handle.await;
        }
        // Drop the watcher so no queued notification can re-arm it afterwards.
        *self
            ._watcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }

    #[cfg(test)]
    fn for_test(machine: &str) -> Arc<Self> {
        Self::for_test_with_subscriber_limit(machine, ACTIVITY_STREAM_SUBSCRIBER_CAPACITY)
    }

    #[cfg(test)]
    fn for_test_with_subscriber_limit(machine: &str, subscriber_limit: usize) -> Arc<Self> {
        let watcher = notify::recommended_watcher(|_: notify::Result<NotifyEvent>| {})
            .expect("test filesystem watcher");
        let log = ActivityEventLog::new(machine.to_string(), Vec::new());
        let lifecycle = log.lifecycle.clone();
        Arc::new(Self {
            log,
            lifecycle,
            subscribers: Arc::new(Semaphore::new(subscriber_limit)),
            _watcher: Arc::new(StdMutex::new(Some(watcher))),
            loops: StdMutex::new(Vec::new()),
        })
    }

    #[cfg(test)]
    fn for_test_with_session_collector(
        machine: &str,
        context: &CliContext,
        tmux_bin: &Path,
        session_collector: SessionCollector,
    ) -> Arc<Self> {
        let sessions = collect_activity_stream_sessions(&session_collector, context, tmux_bin)
            .expect("test activity snapshot source");
        let watcher = notify::recommended_watcher(|_: notify::Result<NotifyEvent>| {})
            .expect("test filesystem watcher");
        let log = ActivityEventLog::new(machine.to_string(), sessions);
        let lifecycle = log.lifecycle.clone();
        Arc::new(Self {
            log,
            lifecycle,
            subscribers: Arc::new(Semaphore::new(ACTIVITY_STREAM_SUBSCRIBER_CAPACITY)),
            _watcher: Arc::new(StdMutex::new(Some(watcher))),
            loops: StdMutex::new(Vec::new()),
        })
    }

    fn subscribe(
        &self,
        last_event_id: Option<&str>,
    ) -> Result<ActivitySubscription, ActivitySubscribeError> {
        let lifecycle = self.lifecycle.subscribe();
        if *lifecycle.borrow() != ActivityBrokerLifecycle::Ready {
            return Err(ActivitySubscribeError::Unavailable);
        }
        let permit = self
            .subscribers
            .clone()
            .try_acquire_owned()
            .map_err(|_| ActivitySubscribeError::Capacity)?;
        if *lifecycle.borrow() != ActivityBrokerLifecycle::Ready {
            return Err(ActivitySubscribeError::Unavailable);
        }
        let mut subscription = self.log.subscribe(last_event_id);
        subscription.lifecycle = lifecycle;
        subscription._subscriber_permit = Some(permit);
        Ok(subscription)
    }
}

fn activity_observed_at() -> String {
    jiff::Timestamp::now().to_string()
}

fn parse_activity_event_id(raw: &str) -> Option<(String, u64)> {
    let (stream_id, sequence) = raw.rsplit_once(':')?;
    if stream_id.is_empty() {
        return None;
    }
    Some((stream_id.to_string(), sequence.parse().ok()?))
}

fn degrade_activity_broker(lifecycle: &watch::Sender<ActivityBrokerLifecycle>, warning: &str) {
    if lifecycle.send_replace(ActivityBrokerLifecycle::Degraded)
        != ActivityBrokerLifecycle::Degraded
    {
        eprintln!("warning: {warning}; session polling will reconcile");
    }
}

fn activity_watch_callback(
    result: notify::Result<NotifyEvent>,
    change_tx: &mpsc::Sender<ActivityChange>,
    lifecycle: &watch::Sender<ActivityBrokerLifecycle>,
    sessions_root: &Path,
) {
    match result {
        Ok(event) if activity_notify_event_root_lost(&event, sessions_root) => {
            if change_tx.try_send(ActivityChange::RootLost).is_err() {
                degrade_activity_broker(
                    lifecycle,
                    "activity sessions root invalidation could not be queued; activity stream disabled",
                );
            }
        }
        Ok(event) if event.need_rescan() => match change_tx.try_send(ActivityChange::Rescan) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
            Err(mpsc::error::TrySendError::Closed(_)) => degrade_activity_broker(
                lifecycle,
                "activity rescan could not be queued; activity stream disabled",
            ),
        },
        Ok(event) if activity_notify_event_relevant(&event) => {
            match change_tx.try_send(ActivityChange::Refresh) {
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                Err(mpsc::error::TrySendError::Closed(_)) => degrade_activity_broker(
                    lifecycle,
                    "activity refresh could not be queued; activity stream disabled",
                ),
            }
        }
        Ok(_) => {}
        Err(_) => degrade_activity_broker(
            lifecycle,
            "activity filesystem notification failed; activity stream disabled",
        ),
    }
}

fn activity_notify_event_root_lost(event: &NotifyEvent, sessions_root: &Path) -> bool {
    matches!(
        event.kind,
        EventKind::Remove(_) | EventKind::Modify(notify::event::ModifyKind::Name(_))
    ) && event.paths.iter().any(|path| path == sessions_root)
}

fn collect_activity_stream_sessions(
    session_collector: &SessionCollector,
    context: &CliContext,
    tmux_bin: &Path,
) -> Result<Vec<ActivityStreamSession>, CliError> {
    session_collector(context, tmux_bin).map(|sessions| {
        sessions
            .into_iter()
            .map(|session| ActivityStreamSession {
                id: session.id,
                turn_state: session
                    .turn_state
                    .as_ref()
                    .map(crate::activity::stream_projection),
            })
            .collect()
    })
}

#[cfg(test)]
async fn activity_change_loop(
    log: Arc<ActivityEventLog>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    session_collector: SessionCollector,
    context: CliContext,
    tmux_bin: PathBuf,
    changes: mpsc::Receiver<ActivityChange>,
) {
    activity_change_loop_inner(
        log,
        lifecycle,
        session_collector,
        context,
        tmux_bin,
        changes,
        ActivityChangeLoopControls::default(),
    )
    .await;
}

async fn activity_change_loop_inner(
    log: Arc<ActivityEventLog>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
    session_collector: SessionCollector,
    context: CliContext,
    tmux_bin: PathBuf,
    mut changes: mpsc::Receiver<ActivityChange>,
    controls: ActivityChangeLoopControls,
) {
    let mut lifecycle_rx = lifecycle.subscribe();
    let mut last_refresh_started_at = None;
    let mut shadow_poll = activity_stream::shadow_refresh_interval();
    shadow_poll.tick().await;
    loop {
        let completion_deadline =
            claude_completion::activity_completion_refresh_deadline(&log, &context).await;
        let completion_refresh = async {
            match completion_deadline {
                Some(deadline) => tokio::time::sleep_until(deadline).await,
                None => std::future::pending::<()>().await,
            }
        };
        let change = tokio::select! {
            biased;
            changed = lifecycle_rx.changed() => {
                if changed.is_err()
                    || *lifecycle_rx.borrow() != ActivityBrokerLifecycle::Ready
                {
                    return;
                }
                continue;
            }
            change = changes.recv() => change,
            _ = shadow_poll.tick() => Some(ActivityChange::Refresh),
            _ = completion_refresh => Some(ActivityChange::Refresh),
        };
        let Some(change) = change else {
            degrade_activity_broker(
                &lifecycle,
                "activity filesystem notification channel closed; activity stream disabled",
            );
            return;
        };
        if change == ActivityChange::RootLost
            && !controls.watch_rearm.as_ref().is_some_and(|rearm| rearm())
        {
            degrade_activity_broker(
                &lifecycle,
                "activity sessions root watch could not be re-armed; activity stream disabled",
            );
            return;
        }
        let batch_started_at = tokio::time::Instant::now();
        let minimum_start_at = last_refresh_started_at
            .map(|started_at| started_at + ACTIVITY_STREAM_MAX_REFRESH_CADENCE)
            .unwrap_or(batch_started_at);
        let quiet = tokio::time::sleep_until(
            (batch_started_at + ACTIVITY_STREAM_DEBOUNCE).max(minimum_start_at),
        );
        let cadence = tokio::time::sleep_until(if minimum_start_at > batch_started_at {
            minimum_start_at
        } else {
            batch_started_at + ACTIVITY_STREAM_MAX_REFRESH_CADENCE
        });
        tokio::pin!(quiet);
        tokio::pin!(cadence);
        loop {
            tokio::select! {
                biased;
                changed = lifecycle_rx.changed() => {
                    if changed.is_err()
                        || *lifecycle_rx.borrow() != ActivityBrokerLifecycle::Ready
                    {
                        return;
                    }
                }
                _ = &mut cadence => break,
                _ = &mut quiet => break,
                change = changes.recv() => match change {
                    None => {
                        degrade_activity_broker(
                            &lifecycle,
                            "activity filesystem notification channel closed; activity stream disabled",
                        );
                        return;
                    }
                    Some(change) => {
                    if change == ActivityChange::RootLost
                        && !controls
                            .watch_rearm
                            .as_ref()
                            .is_some_and(|rearm| rearm())
                    {
                        degrade_activity_broker(
                            &lifecycle,
                            "activity sessions root watch could not be re-armed; activity stream disabled",
                        );
                        return;
                    }
                    quiet
                        .as_mut()
                        .reset(
                            (tokio::time::Instant::now() + ACTIVITY_STREAM_DEBOUNCE)
                                .max(minimum_start_at),
                        );
                    }
                }
            }
        }
        let started_at = tokio::time::Instant::now();
        last_refresh_started_at = Some(started_at);
        if let Some(observer) = &controls.refresh_started {
            let _ = observer.send(started_at);
        }
        if !claude_completion::activity_refresh_snapshot(
            log.clone(),
            lifecycle.clone(),
            session_collector.clone(),
            context.clone(),
            tmux_bin.clone(),
        )
        .await
        {
            return;
        }
    }
}

fn activity_publish_heartbeat_if_ready(
    log: &ActivityEventLog,
    lifecycle: &watch::Sender<ActivityBrokerLifecycle>,
) -> bool {
    if *lifecycle.borrow() != ActivityBrokerLifecycle::Ready {
        return false;
    }
    log.publish_heartbeat();
    true
}

async fn activity_heartbeat_loop(
    log: Arc<ActivityEventLog>,
    lifecycle: watch::Sender<ActivityBrokerLifecycle>,
) {
    let mut interval = tokio::time::interval(ACTIVITY_STREAM_HEARTBEAT_INTERVAL);
    let mut lifecycle_rx = lifecycle.subscribe();
    if *lifecycle_rx.borrow() != ActivityBrokerLifecycle::Ready {
        return;
    }
    interval.tick().await;
    loop {
        tokio::select! {
            biased;
            changed = lifecycle_rx.changed() => {
                if changed.is_err()
                    || *lifecycle_rx.borrow() != ActivityBrokerLifecycle::Ready
                {
                    return;
                }
            }
            _ = interval.tick() => {
                if !activity_publish_heartbeat_if_ready(&log, &lifecycle) {
                    return;
                }
            }
        }
    }
}

// --- response helpers ---------------------------------------------------------

// Every serve response uses one wire contract, `cli.agent-session.serve.v1`,
// distinct from the CLI stdout envelopes: the `data` shape ({machine, ...}) is
// serve-specific, so it must NOT reuse the CLI's per-command schema versions.
fn serve_schema() -> String {
    schema_version_for(BINARY, "serve", 1)
}

pub(crate) fn envelope_ok(data: Value) -> Response {
    envelope_status(StatusCode::OK, data)
}

fn envelope_status(status: StatusCode, data: Value) -> Response {
    let session_id = data
        .pointer("/session/id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut response = (
        status,
        Json(json!({
            "schema_version": serve_schema(),
            "ok": true,
            "data": data,
        })),
    )
        .into_response();
    if let Some(id) = session_id {
        response
            .extensions_mut()
            .insert(crate::lifecycle::ResponseSessionId(id));
    }
    response
}

pub(crate) fn status_json(status: StatusCode, code: &str, message: &str) -> Response {
    let mut response = (
        status,
        Json(json!({
            "schema_version": serve_schema(),
            "ok": false,
            "error": { "code": code, "message": message },
        })),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(CliError::runtime(code, message, None));
    response
}

pub(crate) fn join_err() -> Response {
    envelope_err(CliError::runtime(
        "serve-task-failed",
        "internal task failed",
        None,
    ))
}

#[derive(Debug, Serialize)]
struct UsageReport {
    schema_version: String,
    ok: bool,
    providers: Vec<UsageProvider>,
}

#[derive(Debug, Serialize)]
struct UsageProvider {
    id: String,
    label: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<ProviderUsageReason>,
    windows: Vec<UsageWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<UsageProviderError>,
    #[serde(skip)]
    fresh_authoritative: bool,
}

#[derive(Debug, Serialize)]
struct UsageWindow {
    label: String,
    used_percent: i64,
    remaining_percent: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_at_epoch: Option<i64>,
}

#[derive(Debug, Serialize)]
struct UsageProviderError {
    code: String,
    message: String,
}

struct UsageHelperOutput {
    status_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

async fn usage_handler(State(state): State<Arc<ServeState>>) -> Response {
    let timeout = usage_timeout();
    let codex = tokio::task::spawn_blocking(move || collect_codex_usage(timeout));
    let claude = tokio::task::spawn_blocking(move || collect_claude_usage(timeout));
    let (codex, claude) = tokio::join!(codex, claude);

    let providers = vec![
        codex.unwrap_or_else(|_| provider_internal_error("codex", "Codex")),
        claude.unwrap_or_else(|_| provider_internal_error("claude", "Claude")),
    ];
    let ok = providers.iter().all(|provider| provider.ok);
    envelope_ok(json!({
        "machine": state.machine,
        "usage": UsageReport {
            schema_version: USAGE_SCHEMA_VERSION.to_string(),
            ok,
            providers,
        },
    }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UsageV1Query {
    refresh: Option<String>,
}

/// `GET /usage/v1[?refresh=1]`; the cached provider snapshot lives in `usage.rs`.
async fn usage_v1_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    query: Result<Query<UsageV1Query>, QueryRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let force = match query.as_ref().map(|Query(query)| query.refresh.as_deref()) {
        Ok(None | Some("0")) => false,
        Ok(Some("1")) => true,
        _ => {
            return status_json(
                StatusCode::BAD_REQUEST,
                "invalid-query",
                "the only query parameter is refresh=0 or refresh=1",
            );
        }
    };
    let usage = state.provider_usage.snapshot(force).await;
    envelope_ok(json!({ "machine": state.machine, "usage": usage }))
}

/// `POST /codex/reset/v1`: a raw versioned result on success (`usage.rs`).
async fn codex_reset_v1_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    match state
        .provider_usage
        .codex_reset(&state.machine, &body)
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => status_json(error.status, error.code, error.message),
    }
}

/// `POST /claude/reset/v1`: a raw versioned result on success (`usage.rs`).
/// A failed CLI run adds `error.details` with only daemon-validated codes.
async fn claude_reset_v1_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    match state
        .provider_usage
        .claude_reset(&state.machine, &body)
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) => match error.details {
            None => status_json(error.status, error.code, error.message),
            Some(details) => (
                error.status,
                Json(json!({
                    "schema_version": serve_schema(),
                    "ok": false,
                    "error": { "code": error.code, "message": error.message, "details": details },
                })),
            )
                .into_response(),
        },
    }
}

fn usage_timeout() -> Duration {
    std::env::var("AGENT_SESSION_USAGE_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|millis| *millis > 0)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(DEFAULT_USAGE_TIMEOUT_MS))
}

fn collect_codex_usage(timeout: Duration) -> UsageProvider {
    match run_usage_helper(
        "codex-cli",
        &[
            "diag",
            "rate-limits",
            "--all",
            "--format",
            "json",
            "--no-refresh-auth",
        ],
        &[],
        timeout,
    ) {
        Ok(output) => normalize_codex_usage(output),
        Err(message) => provider_error("codex", "Codex", "helper-spawn-failed", message),
    }
}

fn collect_claude_usage(timeout: Duration) -> UsageProvider {
    let explicit_timeout = non_empty_env("CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS");
    let timeout_seconds =
        claude_inner_timeout_seconds(timeout, explicit_timeout.as_deref()).to_string();
    match run_usage_helper(
        "claude-cli",
        &["usage", "--format", "json", "--source", "auto"],
        &[(
            "CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS",
            timeout_seconds.as_str(),
        )],
        timeout,
    ) {
        Ok(output) => normalize_claude_usage(output),
        Err(message) => provider_error("claude", "Claude", "helper-spawn-failed", message),
    }
}

/// Multi-account Codex rate-limit source, one entry per account keyed by
/// nickname. Used to key a `claude` launch profile's auto-resume off its
/// authoritative Codex account instead of native Claude usage.
const CODEX_ACCOUNT_USAGE_ARGS: &[&str] = &[
    "diag",
    "rate-limits",
    "--all",
    "--format",
    "json",
    "--no-refresh-auth",
];

fn non_authoritative_usage_snapshot() -> UsageSnapshot {
    UsageSnapshot {
        authoritative: false,
        has_exhausted_windows: false,
        exhausted_reset_epochs: Vec::new(),
        soonest_reset_epoch: None,
    }
}

/// Build a per-account [`UsageSnapshot`] for a `claude` launch
/// profile backed by a Codex account, from `codex-cli diag rate-limits --all`
/// output FILTERED to `account`'s nickname.
///
/// Fail safe: any parse failure, a missing nickname, a non-ok entry, or an
/// entry with no windows degrades to a non-authoritative snapshot so the
/// auto-resume loop retries rather than acting on the wrong quota. It never
/// falls back to native Claude usage.
fn codex_account_usage_snapshot(output: &UsageHelperOutput, account: &str) -> UsageSnapshot {
    if output.timed_out {
        return non_authoritative_usage_snapshot();
    }
    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => return non_authoritative_usage_snapshot(),
    };
    let Some(entry) = value
        .get("results")
        .and_then(Value::as_array)
        .and_then(|results| {
            results
                .iter()
                .find(|result| result.get("name").and_then(Value::as_str) == Some(account))
        })
    else {
        // Nickname missing from the diag output.
        return non_authoritative_usage_snapshot();
    };
    // Mirror the Codex path's ok classification (see `map_codex_diag_result`
    // in the infra usage reader): success requires both `ok` and `status`.
    let ok = entry.get("ok").and_then(Value::as_bool) == Some(true)
        && entry.get("status").and_then(Value::as_str) == Some("ok");
    if !ok {
        return non_authoritative_usage_snapshot();
    }
    let mut windows = windows_from_value(entry, None);
    if windows.is_empty()
        && let Some(summary) = entry.get("summary")
    {
        windows = windows_from_codex_summary(summary);
    }
    if windows.is_empty() {
        // A successful read with no windows cannot confirm the quota state;
        // degrade rather than resume against an unknown quota.
        return non_authoritative_usage_snapshot();
    }
    UsageSnapshot {
        authoritative: true,
        has_exhausted_windows: windows.iter().any(|window| window.used_percent >= 100),
        exhausted_reset_epochs: windows
            .iter()
            .filter(|window| window.used_percent >= 100)
            .filter_map(|window| window.reset_at_epoch)
            .collect(),
        soonest_reset_epoch: windows
            .iter()
            .filter_map(|window| window.reset_at_epoch)
            .min(),
    }
}

fn claude_inner_timeout_seconds(timeout: Duration, explicit: Option<&str>) -> u64 {
    let max_inner_seconds = timeout
        .checked_sub(Duration::from_secs(CLAUDE_USAGE_CLEANUP_SLACK_SECONDS))
        .map(|remaining| remaining.as_secs())
        .unwrap_or(0)
        .max(1);
    explicit
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(max_inner_seconds)
        .min(max_inner_seconds)
}

fn run_usage_helper(
    program: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    timeout: Duration,
) -> Result<UsageHelperOutput, String> {
    let mut child = ProcessCommand::new(program)
        .args(args)
        .envs(envs.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|err| format!("failed to start {program}: {err}"))?;

    let start = Instant::now();
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.code().unwrap_or(1)),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    timed_out = true;
                    unsafe {
                        let pgid = -(child.id() as libc::pid_t);
                        let _ = libc::kill(pgid, libc::SIGKILL);
                    }
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => {
                let _ = child.kill();
                return Err(format!("failed to wait for {program}: {err}"));
            }
        }
    };

    let mut stdout = Vec::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_end(&mut stdout);
    }
    let mut stderr = Vec::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_end(&mut stderr);
    }

    Ok(UsageHelperOutput {
        status_code: status,
        stdout,
        stderr,
        timed_out,
    })
}

fn normalize_codex_usage(output: UsageHelperOutput) -> UsageProvider {
    if output.timed_out {
        return provider_error(
            "codex",
            "Codex",
            "helper-timeout",
            "codex usage helper timed out".to_string(),
        );
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => {
            return provider_error(
                "codex",
                "Codex",
                "helper-invalid-json",
                helper_failure_message("codex usage unavailable", &output),
            );
        }
    };

    let reason_code = reason_from_helper_json(&value);
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.get("result").map(|result| vec![result.clone()]))
        .unwrap_or_default();

    let mut windows = Vec::new();
    for result in results.iter().filter(|result| {
        result
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| result.get("status").and_then(Value::as_str) == Some("ok"))
    }) {
        windows.extend(windows_from_value(result, None));
        if windows.is_empty()
            && let Some(summary) = result.get("summary")
        {
            windows.extend(windows_from_codex_summary(summary));
        }
    }

    if !windows.is_empty() {
        return UsageProvider {
            id: "codex".to_string(),
            label: "Codex".to_string(),
            ok: true,
            source: Some("codex-cli".to_string()),
            reason_code,
            windows,
            error: None,
            fresh_authoritative: false,
        };
    }

    let (code, message) =
        error_from_helper_json(&value, output.status_code, "codex usage unavailable");
    provider_error_with_reason("codex", "Codex", &code, message, reason_code)
}

fn normalize_claude_usage(output: UsageHelperOutput) -> UsageProvider {
    if output.timed_out {
        return provider_error(
            "claude",
            "Claude",
            "helper-timeout",
            "claude usage helper timed out".to_string(),
        );
    }

    let value: Value = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(_) => {
            return provider_error(
                "claude",
                "Claude",
                "helper-invalid-json",
                helper_failure_message("claude usage unavailable", &output),
            );
        }
    };

    let result = value.get("result").unwrap_or(&value);
    let reason_code = reason_from_helper_json(&value);
    let fresh_authoritative = output.status_code == Some(0)
        && value.get("ok").and_then(Value::as_bool) == Some(true)
        && result
            .get("source")
            .and_then(Value::as_str)
            .is_some_and(|source| matches!(source, "oauth" | "cli"))
        && result.get("stale").and_then(Value::as_bool) == Some(false)
        && reason_code.is_none();
    let reference_epoch = i64_field(result, &["updated_at", "updatedAt"]);
    let windows = windows_from_value(result, reference_epoch);
    if !windows.is_empty() {
        return UsageProvider {
            id: "claude".to_string(),
            label: "Claude".to_string(),
            ok: true,
            source: Some("claude-cli".to_string()),
            reason_code,
            windows,
            error: None,
            fresh_authoritative,
        };
    }

    let (code, message) =
        error_from_helper_json(&value, output.status_code, "claude usage unavailable");
    provider_error_with_reason("claude", "Claude", &code, message, reason_code)
}

fn windows_from_value(value: &Value, reference_epoch: Option<i64>) -> Vec<UsageWindow> {
    value
        .get("windows")
        .and_then(Value::as_array)
        .map(|windows| {
            windows
                .iter()
                .filter_map(|window| usage_window_from_value(window, reference_epoch))
                .collect()
        })
        .unwrap_or_default()
}

fn usage_window_from_value(value: &Value, reference_epoch: Option<i64>) -> Option<UsageWindow> {
    let label = value.get("label").and_then(Value::as_str)?.to_string();
    let used_percent = i64_field(value, &["used_percent"])?;
    let remaining_percent = i64_field(value, &["remaining_percent"])
        .unwrap_or_else(|| (100 - used_percent).clamp(0, 100));
    let reset_at = reset_at_text_field(value, RESET_AT_KEYS);
    let reset_at_epoch = epoch_field(value, RESET_AT_EPOCH_KEYS, reference_epoch);
    Some(UsageWindow {
        label,
        used_percent,
        remaining_percent,
        reset_at,
        reset_at_epoch,
    })
}

fn windows_from_codex_summary(summary: &Value) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    if let Some(remaining) = i64_field(summary, &["non_weekly_remaining"]) {
        windows.push(UsageWindow {
            label: summary
                .get("non_weekly_label")
                .and_then(Value::as_str)
                .unwrap_or("Non-weekly")
                .to_string(),
            used_percent: (100 - remaining).clamp(0, 100),
            remaining_percent: remaining,
            reset_at: None,
            reset_at_epoch: i64_field(
                summary,
                &["non_weekly_reset_epoch", "non_weekly_reset_at_epoch"],
            ),
        });
    }
    if let Some(remaining) = i64_field(summary, &["weekly_remaining"]) {
        windows.push(UsageWindow {
            label: "Weekly".to_string(),
            used_percent: (100 - remaining).clamp(0, 100),
            remaining_percent: remaining,
            reset_at: None,
            reset_at_epoch: i64_field(summary, &["weekly_reset_epoch", "weekly_reset_at_epoch"]),
        });
    }
    windows
}

fn reset_at_text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|raw| !raw.is_empty())
            .filter(|raw| safe_reset_at_text(raw))
            .map(ToString::to_string)
    })
}

fn safe_reset_at_text(raw: &str) -> bool {
    if raw.len() > 256 || raw.chars().any(char::is_control) {
        return false;
    }
    let lower = raw.to_ascii_lowercase();
    if lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("authorization")
        || lower.contains("bearer")
        || lower.contains("sk-")
        || lower.contains("account_id")
        || lower.contains("account-id")
        || lower.contains("acct_")
        || raw.contains('@')
    {
        return false;
    }
    !(raw.starts_with('/')
        || raw.starts_with("~/")
        || raw.starts_with("$HOME/")
        || raw.contains("/home/")
        || raw.contains("/Users/"))
}

fn epoch_field(value: &Value, keys: &[&str], reference_epoch: Option<i64>) -> Option<i64> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|value| epoch_seconds_from_value(value, reference_epoch))
    })
}

fn epoch_seconds_from_value(value: &Value, reference_epoch: Option<i64>) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().and_then(epoch_seconds_from_f64))
            .map(normalize_epoch_seconds),
        Value::String(raw) => reset_epoch_seconds_from_str(raw, reference_epoch),
        _ => None,
    }
}

fn i64_field(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_f64().map(|n| n.round() as i64))
        })
    })
}

fn error_from_helper_json(
    value: &Value,
    status_code: Option<i32>,
    fallback: &str,
) -> (String, String) {
    let code = value
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            if status_code.unwrap_or(0) == 0 {
                "usage-unavailable"
            } else {
                "helper-failed"
            }
        })
        .to_string();
    let message = value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or(fallback);
    (code, sanitize_helper_message(message))
}

fn reason_from_helper_json(value: &Value) -> Option<ProviderUsageReason> {
    fn direct(value: &Value) -> Option<ProviderUsageReason> {
        let direct = value
            .get("reason_code")
            .and_then(Value::as_str)
            .and_then(ProviderUsageReason::from_code);
        let details = value
            .get("error")
            .and_then(|error| error.get("details"))
            .and_then(|details| details.get("reason_code"))
            .and_then(Value::as_str)
            .and_then(ProviderUsageReason::from_code);
        match (direct, details) {
            (Some(first), Some(second)) => Some(prefer_reason(first, second)),
            (Some(reason), None) | (None, Some(reason)) => Some(reason),
            (None, None) => None,
        }
    }

    let mut reason = direct(value);
    let mut merge = |candidate: Option<ProviderUsageReason>| {
        if let Some(candidate) = candidate {
            reason = Some(
                reason
                    .map(|current| prefer_reason(current, candidate))
                    .unwrap_or(candidate),
            );
        }
    };
    merge(value.get("result").and_then(direct));
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        for result in results {
            merge(direct(result));
        }
    }
    reason
}

fn helper_failure_message(prefix: &str, output: &UsageHelperOutput) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = stderr
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or(prefix);
    sanitize_helper_message(message)
}

fn provider_internal_error(id: &str, label: &str) -> UsageProvider {
    provider_error(
        id,
        label,
        "serve-task-failed",
        "usage provider task failed".to_string(),
    )
}

fn provider_error(id: &str, label: &str, code: &str, message: String) -> UsageProvider {
    let reason_code = match code {
        "helper-timeout" => Some(ProviderUsageReason::Timeout),
        "helper-spawn-failed" | "helper-invalid-json" | "serve-task-failed" => {
            Some(ProviderUsageReason::ServiceUnavailable)
        }
        _ => None,
    };
    provider_error_with_reason(id, label, code, message, reason_code)
}

fn provider_error_with_reason(
    id: &str,
    label: &str,
    code: &str,
    message: String,
    reason_code: Option<ProviderUsageReason>,
) -> UsageProvider {
    UsageProvider {
        id: id.to_string(),
        label: label.to_string(),
        ok: false,
        source: None,
        reason_code,
        windows: Vec::new(),
        error: Some(UsageProviderError {
            code: code.to_string(),
            message: sanitize_helper_message(&message),
        }),
        fresh_authoritative: false,
    }
}

fn sanitize_helper_message(message: &str) -> String {
    let normalized = message.replace(['\n', '\r', '\t'], " ");
    let mut cleaned = Vec::new();
    let mut redact_next = false;
    for token in normalized.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        let should_redact = redact_next
            || lower.contains("access_token")
            || lower.contains("refresh_token")
            || lower.contains("authorization")
            || lower.contains("bearer")
            || lower.contains("sk-")
            || lower.contains("account_id")
            || lower.contains("account-id")
            || lower.contains("acct_")
            || token.contains('@');
        redact_next = lower.contains("bearer") || lower.contains("authorization:");

        if should_redact {
            cleaned.push("[redacted]".to_string());
        } else if token.starts_with('/')
            || token.starts_with("~/")
            || token.starts_with("$HOME/")
            || token.contains("/home/")
            || token.contains("/Users/")
        {
            cleaned.push("[path]".to_string());
        } else {
            cleaned.push(token.to_string());
        }
        if cleaned.len() >= 24 {
            break;
        }
    }
    if cleaned.is_empty() {
        "usage unavailable".to_string()
    } else {
        cleaned.join(" ")
    }
}

/// XOR-accumulating byte comparison so a correct-length wrong token is compared
/// in constant time (no early return on the first differing byte). The length
/// check is intentionally NOT constant-time — the token length is not treated as
/// a secret; the token itself is the high-entropy secret. Never echoed anywhere.
pub(crate) fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Enforce the bearer token on authenticated endpoints. Returns `Some(denial)`
/// to reject (401), or `Some(503)` when the daemon has no token configured
/// (fail closed); returns `None` when the request is authorized.
fn deny_unauthorized(state: &ServeState, headers: &HeaderMap) -> Option<Response> {
    let Some(expected) = state.token.as_deref() else {
        return Some(status_json(
            StatusCode::SERVICE_UNAVAILABLE,
            "token-not-configured",
            "server has no token configured; authenticated endpoints are disabled",
        ));
    };
    let provided = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    match provided {
        Some(token) if constant_time_eq(token, expected) => None,
        _ => Some(status_json(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "missing or invalid bearer token",
        )),
    }
}

// --- request bodies -----------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GlanceQuery {
    tail: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaintenanceQuery {
    operation: MaintenanceOperation,
    /// Maintenance contract the client understands. Absent means v1, so an
    /// existing Console keeps receiving a valid v1 preview; an unrecognized
    /// version fails the query instead of degrading silently.
    #[serde(default)]
    schema_version: maintenance::MaintenanceContract,
}

#[derive(Debug, Default)]
enum CreateTitleInput {
    #[default]
    Missing,
    Null,
    Value(String),
}

impl CreateTitleInput {
    fn is_supplied(&self) -> bool {
        !matches!(self, Self::Missing)
    }

    fn into_value(self) -> Option<String> {
        match self {
            Self::Missing | Self::Null => None,
            Self::Value(value) => Some(value),
        }
    }
}

fn deserialize_create_title<'de, D>(deserializer: D) -> Result<CreateTitleInput, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(match Option::<String>::deserialize(deserializer)? {
        Some(value) => CreateTitleInput::Value(value),
        None => CreateTitleInput::Null,
    })
}

#[derive(Debug, Deserialize)]
struct SendBody {
    text: Option<String>,
    #[serde(default)]
    keys: Vec<String>,
    /// Opt in to typing literal text while the agent is blocked on an approval
    /// or question. Keys are always admitted, so answering never needs this.
    #[serde(default)]
    allow_blocked: bool,
}

#[derive(Debug, Deserialize)]
struct StructuredPromptBody {
    text: String,
    expected_session_incarnation: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AutoResumeBody {
    enabled: bool,
    #[serde(default)]
    recovery_policy: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AccountSwitchBody {
    account: String,
    expected_session_incarnation: Option<String>,
    #[serde(default)]
    supports_unbound_account_queue: bool,
}

#[derive(Debug, Deserialize)]
struct AttachmentQuery {
    filename: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkdirQuery {
    q: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    git_only: bool,
    #[serde(default)]
    exclude_worktrees: bool,
}

#[derive(Debug, Deserialize)]
struct RepoRemoteUrlQuery {
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HistoryListQuery {
    q: Option<String>,
    provider: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveBody {
    expected_session_incarnation: String,
    /// Star the archived session as it lands in history. Older clients omit it.
    #[serde(default)]
    starred: bool,
    /// Archive even though the session still has children on this machine.
    #[serde(default)]
    orphan_children: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteQuery {
    #[serde(default)]
    orphan_children: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryStarBody {
    starred: bool,
}

// --- handlers -----------------------------------------------------------------

/// Registry-epoch slack used only to screen protected-session candidates.
///
/// Deliberately wider than the sidecar's own freshness window so a broker whose
/// registry epoch lags its sidecar write is still authenticated rather than
/// silently dropped. Freshness authority stays with the sidecar.
const HEARTBEAT_CANDIDATE_WINDOW_SECONDS: i64 = 300;

/// Whether the control-plane runtime can actually serve, independent of whether
/// this HTTP handler answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeAvailability {
    Available,
    Degraded,
    Unavailable,
}

impl RuntimeAvailability {
    fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Degraded => "degraded",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Projected control-plane health for one machine.
struct HealthProjection {
    availability: RuntimeAvailability,
    executable_state: &'static str,
    coordination: &'static str,
    reasons: Vec<&'static str>,
    protected_sessions: usize,
}

impl HealthProjection {
    /// `machine offline`, `runtime unavailable`, and `session protected` are
    /// three different operator situations with three different responses, and
    /// collapsing them into one "offline" state is what made Agent Console
    /// unable to say anything useful during the `sympoies/nils-cli#1409`
    /// incident. Machine reachability is already answered by receiving this
    /// response at all, so this projection covers the other two.
    fn collect(state: &ServeState) -> Self {
        let mut reasons = Vec::new();
        let mut availability = RuntimeAvailability::Available;

        let executable_state = nils_common::runtime_compat::executable_state(std::process::id());
        if executable_state == nils_common::runtime_compat::ExecutableState::Replaced {
            // The daemon is answering from an executable that no longer exists,
            // so a restart is required before it can be trusted to serve.
            availability = RuntimeAvailability::Unavailable;
            reasons.push("runtime-executable-replaced");
        }

        if !state.context.state_dir.is_dir() {
            availability = RuntimeAvailability::Unavailable;
            reasons.push("state-dir-unavailable");
        }

        let (coordination, protected_sessions) =
            match nils_common::coordination_projection::load(&state.context.state_dir) {
                Ok(Some(registry)) => {
                    let now = jiff::Timestamp::now().as_second();
                    // A session whose broker is ready with a fresh heartbeat is
                    // protected: its runtime must not be restarted out from
                    // under an active claim or an uncertain operation.
                    //
                    // The registry's own heartbeat epoch is a cheap pre-filter
                    // only. It is not freshness authority — the sidecar is — but
                    // it can only ever over-report a stale broker as a
                    // candidate, so screening on it first bounds this open
                    // loopback route to one private read per plausibly-live
                    // broker instead of one per record.
                    let protected = registry
                        .brokers
                        .values()
                        .filter(|broker| {
                            broker.state == "ready"
                                && now.saturating_sub(broker.heartbeat_epoch)
                                    <= HEARTBEAT_CANDIDATE_WINDOW_SECONDS
                        })
                        .filter(|broker| {
                            nils_common::coordination_projection::heartbeat_fresh(
                                &state.context.state_dir,
                                &broker.session_id,
                                &broker.incarnation,
                                now,
                            )
                        })
                        .count();
                    ("available", protected)
                }
                Ok(None) => ("absent", 0),
                Err(error) => {
                    availability = availability.max_degraded();
                    let code = match error {
                        nils_common::coordination_projection::ReadError::Unavailable => {
                            "coordination-unavailable"
                        }
                        nils_common::coordination_projection::ReadError::Untrusted => {
                            "coordination-untrusted"
                        }
                        nils_common::coordination_projection::ReadError::Invalid => {
                            "coordination-invalid"
                        }
                        nils_common::coordination_projection::ReadError::Incompatible => {
                            "runtime-version-skew"
                        }
                    };
                    reasons.push(code);
                    (code, 0)
                }
            };

        Self {
            availability,
            executable_state: executable_state.as_str(),
            coordination,
            reasons,
            protected_sessions,
        }
    }

    fn health(&self) -> &'static str {
        match self.availability {
            RuntimeAvailability::Available => "healthy",
            RuntimeAvailability::Degraded => "degraded",
            RuntimeAvailability::Unavailable => "critical",
        }
    }
}

impl RuntimeAvailability {
    /// Degrade without ever upgrading an already-unavailable runtime.
    fn max_degraded(self) -> Self {
        match self {
            Self::Unavailable => Self::Unavailable,
            _ => Self::Degraded,
        }
    }
}

async fn healthz(State(state): State<Arc<ServeState>>) -> Response {
    let projection = HealthProjection::collect(&state);
    // `status` keeps its historical meaning — this handler answered — so an
    // existing consumer is unaffected. The distinguishable states are additive.
    envelope_ok(json!({
        "status": "ok",
        "machine": state.machine,
        "health": projection.health(),
        "runtime": {
            "state": projection.availability.as_str(),
            "binary_version": nils_build_info::long_version(env!("CARGO_PKG_VERSION")),
            "executable_state": projection.executable_state,
            "coordination": projection.coordination,
            "reasons": projection.reasons,
        },
        "sessions": {
            "protected": projection.protected_sessions,
        },
    }))
}

async fn retitle_readiness_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let service = state.retitle.clone();
    match tokio::task::spawn_blocking(move || service.readiness()).await {
        Ok(readiness) => envelope_ok(json!({
            "machine": state.machine,
            "retitle": readiness,
        })),
        Err(_) => join_err(),
    }
}

async fn clipboard_unwrap_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let selection = body.ok().and_then(|Json(value)| {
        value
            .get("selection")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let Some(selection) = selection else {
        return envelope_err(CliError::usage(
            "invalid-clipboard-selection",
            "clipboard selection is invalid",
            None,
        ));
    };
    if !crate::retitle::valid_clipboard_selection(&selection) {
        return envelope_err(CliError::usage(
            "invalid-clipboard-selection",
            "clipboard selection is invalid",
            None,
        ));
    }
    let permit = match state.retitle.acquire().await {
        Ok(permit) => permit,
        Err(error) => return envelope_err(error),
    };
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancel_on_disconnect = ClipboardCancellation(cancelled.clone());
    match tokio::task::spawn_blocking(move || {
        crate::retitle::infer_clipboard(&permit, &selection, &cancelled)
    })
    .await
    {
        Ok(Ok(output)) => envelope_ok(json!({"machine": state.machine, "output": output})),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

struct ClipboardCancellation(Arc<AtomicBool>);

impl Drop for ClipboardCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

async fn display_metadata_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<crate::display_metadata::Update>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "display metadata request body is invalid",
                None,
            ));
        }
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        crate::display_metadata::update(&context, &id, body)
            .map(|record| crate::session_view(&context, &record, None, Some(&tmux)))
    })
    .await
    {
        Ok(Ok(session)) => envelope_ok(json!({"machine":state.machine,"session":session})),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn retitle_v3_readiness_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let readiness = crate::retitle_v3::global_readiness(state.retitle.is_available());
    envelope_ok(json!({
        "machine": state.machine,
        "retitle": readiness,
    }))
}

async fn session_retitle_v3_readiness_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let catalog = state.history_catalog.clone();
    let provider_available = state.retitle.is_available();
    match tokio::task::spawn_blocking(move || {
        crate::retitle_v3::session_readiness(&context, &catalog, &id, provider_available)
    })
    .await
    {
        Ok(Ok(readiness)) => envelope_ok(json!({
            "machine": state.machine,
            "retitle": readiness,
        })),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn session_retitle_v3_receipts_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || crate::retitle_v3::operation_receipts(&context, &id))
        .await
    {
        Ok(Ok(receipts)) => envelope_ok(json!({
            "machine": state.machine,
            "retitle": receipts,
        })),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn session_retitle_v3_operation_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, operation_hash)): AxPath<(String, String)>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    if !crate::retitle_v3::valid_operation_hash(&operation_hash) {
        return envelope_err(crate::retitle_v3::invalid_operation_hash_error());
    }
    let context = state.context.clone();
    let id_for_lookup = id.clone();
    let operation_for_lookup = operation_hash.clone();
    match tokio::task::spawn_blocking(move || {
        crate::retitle_v3::operation_response(&context, &id_for_lookup, &operation_for_lookup)
    })
    .await
    {
        Ok(Ok(Some(retitle))) => {
            if retitle.status == "accepted" {
                spawn_retitle_v3_continuation(state.clone(), id, operation_hash);
            }
            envelope_ok(json!({
                "machine": state.machine,
                "retitle": retitle,
            }))
        }
        Ok(Ok(None)) => envelope_err(crate::retitle_v3::operation_not_found()),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn session_retitle_v3_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<crate::retitle_v3::RetitleV3Request>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(request) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "retitle v3 request body is invalid",
                None,
            ));
        }
    };
    let manual = request.trigger == "manual";
    let result = execute_retitle_v3(state.clone(), id.clone(), request).await;
    if manual {
        spawn_selected_retitle_v3_continuation(state.clone(), id).await;
    }
    match result {
        Ok((status, retitle)) => envelope_status(
            status,
            json!({
                "machine": state.machine,
                "retitle": retitle,
            }),
        ),
        Err(error) => envelope_err(error),
    }
}

struct RetitleV3ManualPriorityGuard {
    waiters: Arc<StdMutex<HashMap<String, usize>>>,
    id: String,
}

impl RetitleV3ManualPriorityGuard {
    fn register(state: &ServeState, id: &str, trigger: &str) -> Option<Self> {
        if trigger != "manual" {
            return None;
        }
        let mut waiters = state
            .retitle_v3_manual_waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *waiters.entry(id.to_string()).or_default() += 1;
        drop(waiters);
        Some(Self {
            waiters: state.retitle_v3_manual_waiters.clone(),
            id: id.to_string(),
        })
    }
}

impl Drop for RetitleV3ManualPriorityGuard {
    fn drop(&mut self) {
        let mut waiters = self
            .waiters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(count) = waiters.get_mut(&self.id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                waiters.remove(&self.id);
            }
        }
    }
}

fn retitle_v3_manual_waiting(state: &ServeState, id: &str) -> bool {
    state
        .retitle_v3_manual_waiters
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .is_some_and(|count| *count > 0)
}

async fn execute_retitle_v3(
    state: Arc<ServeState>,
    id: String,
    request: crate::retitle_v3::RetitleV3Request,
) -> Result<(StatusCode, crate::retitle_v3::RetitleV3Response), CliError> {
    let _manual_priority = RetitleV3ManualPriorityGuard::register(&state, &id, &request.trigger);
    if request.trigger == "automatic" {
        for _ in 0..100 {
            if !retitle_v3_manual_waiting(&state, &id) {
                break;
            }
            tokio::task::yield_now().await;
        }
    }
    let operation_hash = crate::retitle_v3::request_operation_hash(&id, &request);
    let replay = {
        let _session_guard = state.retitle.lock_session(&id).await;
        let context = state.context.clone();
        let id_for_replay = id.clone();
        let request_for_replay = request.clone();
        tokio::task::spawn_blocking(move || {
            crate::retitle_v3::operation_response_for_request(
                &context,
                &id_for_replay,
                &request_for_replay,
            )
        })
        .await
        .map_err(|_| retitle_worker_failed())??
    };
    if let Some(response) = replay {
        if response.status == "terminal" {
            return Ok((StatusCode::OK, response));
        }
        if response.readiness == crate::retitle_v3::MemoryReadiness::Ready {
            return evaluate_retitle_v3(state, id, operation_hash).await;
        }
        spawn_retitle_v3_continuation(state.clone(), id, operation_hash);
        return Ok((StatusCode::ACCEPTED, response));
    }
    let refreshed = {
        let _session_guard = state.retitle.lock_session(&id).await;
        let context = state.context.clone();
        let catalog = state.history_catalog.clone();
        let id_for_refresh = id.clone();
        let request_for_refresh = request.clone();
        tokio::task::spawn_blocking(move || {
            crate::retitle_v3::refresh_once(
                &context,
                &catalog,
                &id_for_refresh,
                &request_for_refresh,
            )
        })
        .await
        .map_err(|_| retitle_worker_failed())??
    };
    if request.trigger == "manual"
        && refreshed.readiness == crate::retitle_v3::MemoryReadiness::Ready
    {
        return evaluate_retitle_v3(state, id, operation_hash).await;
    }
    if refreshed.status == "accepted" {
        spawn_retitle_v3_continuation(state.clone(), id, operation_hash);
        return Ok((StatusCode::ACCEPTED, refreshed));
    }
    Ok((StatusCode::OK, refreshed))
}

async fn continue_retitle_v3_operation(
    state: Arc<ServeState>,
    id: String,
    operation_hash: String,
) -> RetitleV3ContinuationOutcome {
    let mut last_revision = None;
    let mut retry_delay = Duration::from_secs(1);
    for _ in 0..8 {
        if retitle_v3_manual_waiting(&state, &id) {
            let context = state.context.clone();
            let id_for_trigger = id.clone();
            let operation_for_trigger = operation_hash.clone();
            let trigger = tokio::task::spawn_blocking(move || {
                crate::retitle_v3::operation_trigger(
                    &context,
                    &id_for_trigger,
                    &operation_for_trigger,
                )
            })
            .await;
            if matches!(trigger, Ok(Ok(Some(trigger))) if trigger == "automatic") {
                return RetitleV3ContinuationOutcome::Stopped;
            }
        }
        let _session_guard = state.retitle.lock_session(&id).await;
        let context = state.context.clone();
        let catalog = state.history_catalog.clone();
        let id_for_refresh = id.clone();
        let operation_for_refresh = operation_hash.clone();
        let refreshed = tokio::task::spawn_blocking(move || {
            crate::retitle_v3::refresh_operation_once(
                &context,
                &catalog,
                &id_for_refresh,
                &operation_for_refresh,
            )
        })
        .await;
        drop(_session_guard);
        match refreshed {
            Ok(Ok(response)) if response.status == "terminal" => {
                return RetitleV3ContinuationOutcome::Completed;
            }
            Ok(Ok(response)) if response.readiness == crate::retitle_v3::MemoryReadiness::Ready => {
                match evaluate_retitle_v3(state.clone(), id.clone(), operation_hash.clone()).await {
                    Ok((_, response)) if response.status == "terminal" => {
                        return RetitleV3ContinuationOutcome::Completed;
                    }
                    Ok(_) => {
                        tokio::time::sleep(retry_delay).await;
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                    }
                    Err(error)
                        if crate::retitle::is_retryable_automatic_error_code(error.code()) =>
                    {
                        tokio::time::sleep(retry_delay).await;
                        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                    }
                    Err(_) => return RetitleV3ContinuationOutcome::Stopped,
                }
            }
            Ok(Ok(response)) if response.status == "accepted" => {
                if last_revision == Some(response.memory_revision) {
                    tokio::time::sleep(retry_delay).await;
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                } else {
                    last_revision = Some(response.memory_revision);
                    retry_delay = Duration::from_secs(1);
                    tokio::task::yield_now().await;
                }
            }
            Ok(Ok(_)) => return RetitleV3ContinuationOutcome::Stopped,
            Ok(Err(error)) if crate::retitle::is_retryable_automatic_error_code(error.code()) => {
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
            }
            Ok(Err(_)) => return RetitleV3ContinuationOutcome::Stopped,
            Err(_) => {
                tokio::time::sleep(retry_delay).await;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
            }
        }
    }
    RetitleV3ContinuationOutcome::Retryable
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetitleV3ContinuationOutcome {
    Completed,
    Retryable,
    Stopped,
}

impl RetitleV3ContinuationOutcome {
    fn follow_up_delay(self) -> Option<Duration> {
        match self {
            Self::Completed => Some(Duration::ZERO),
            Self::Retryable => Some(Duration::from_secs(30)),
            Self::Stopped => None,
        }
    }
}

struct RetitleV3ContinuationGuard {
    running: Arc<StdMutex<HashSet<String>>>,
    operation_hash: String,
}

impl RetitleV3ContinuationGuard {
    fn register(running: Arc<StdMutex<HashSet<String>>>, operation_hash: &str) -> Option<Self> {
        let mut operations = running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !operations.insert(operation_hash.to_string()) {
            return None;
        }
        drop(operations);
        Some(Self {
            running,
            operation_hash: operation_hash.to_string(),
        })
    }

    fn is_registered(running: &StdMutex<HashSet<String>>, operation_hash: &str) -> bool {
        running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(operation_hash)
    }
}

impl Drop for RetitleV3ContinuationGuard {
    fn drop(&mut self) {
        self.running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.operation_hash);
    }
}

async fn wait_for_retitle_v3_follow_up(
    guard: RetitleV3ContinuationGuard,
    outcome: RetitleV3ContinuationOutcome,
) -> bool {
    let Some(delay) = outcome.follow_up_delay() else {
        drop(guard);
        return false;
    };
    if !delay.is_zero() {
        tokio::time::sleep(delay).await;
    }
    drop(guard);
    true
}

fn spawn_retitle_v3_continuation(state: Arc<ServeState>, id: String, operation_hash: String) {
    let Some(guard) = RetitleV3ContinuationGuard::register(
        state.retitle_v3_continuations.clone(),
        &operation_hash,
    ) else {
        return;
    };
    tokio::spawn(async move {
        let outcome =
            continue_retitle_v3_operation(state.clone(), id.clone(), operation_hash).await;
        // Retryable pending work keeps its single-flight ownership throughout
        // the bounded pause. Completed work advances immediately; permanent
        // failures release ownership without starting another task.
        if wait_for_retitle_v3_follow_up(guard, outcome).await {
            spawn_selected_retitle_v3_continuation(state, id).await;
        }
    });
}

async fn spawn_selected_retitle_v3_continuation(state: Arc<ServeState>, id: String) {
    let context = state.context.clone();
    let id_for_lookup = id.clone();
    let operation_hash = tokio::task::spawn_blocking(move || {
        load_session_record(&context, &id_for_lookup)
            .ok()
            .and_then(|record| crate::retitle_v3::selected_pending_operation_hash(&record))
    })
    .await
    .ok()
    .flatten();
    if let Some(operation_hash) = operation_hash {
        spawn_retitle_v3_continuation(state, id, operation_hash);
    }
}

async fn recover_retitle_v3_operations_once(state: Arc<ServeState>) {
    // Every accepted POST/GET schedules its continuation directly. A daemon
    // restart needs one cold adoption pass; a perpetual one-second catalog
    // scan would make idle cost proportional to the entire session fleet.
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let collect = state.session_collector.clone();
    let Ok(Ok(sessions)) = tokio::task::spawn_blocking(move || collect(&context, &tmux)).await
    else {
        return;
    };
    for session in sessions {
        spawn_selected_retitle_v3_continuation(state.clone(), session.id).await;
    }
}

async fn evaluate_retitle_v3(
    state: Arc<ServeState>,
    id: String,
    operation_hash: String,
) -> Result<(StatusCode, crate::retitle_v3::RetitleV3Response), CliError> {
    let local_context = state.context.clone();
    let local_catalog = state.history_catalog.clone();
    let local_id = id.clone();
    let local_operation = operation_hash.clone();
    match tokio::task::spawn_blocking(move || {
        crate::retitle_v3::complete_manual_locally(
            &local_context,
            &local_catalog,
            &local_id,
            &local_operation,
        )
    })
    .await
    .map_err(|_| retitle_worker_failed())?
    {
        Ok(Some(response)) => {
            invalidate_managed_history_titles(&state.managed_history_titles);
            return Ok((StatusCode::OK, response));
        }
        Ok(None) => {}
        Err(error) if error.code() == "retitle-v3-operation-queued" => {
            let context = state.context.clone();
            let queued_id = id.clone();
            let queued_operation = operation_hash.clone();
            let response = tokio::task::spawn_blocking(move || {
                crate::retitle_v3::operation_response(&context, &queued_id, &queued_operation)
            })
            .await
            .map_err(|_| retitle_worker_failed())??
            .ok_or_else(retitle_worker_failed)?;
            return Ok((StatusCode::ACCEPTED, response));
        }
        Err(error) => return Err(error),
    }
    let context = state.context.clone();
    let id_for_context = id.clone();
    let operation_for_context = operation_hash.clone();
    let claim_token = uuid::Uuid::new_v4().simple().to_string();
    let claimed = tokio::task::spawn_blocking(move || {
        crate::retitle_v3::claim_provider_inference(
            &context,
            &id_for_context,
            &operation_for_context,
            &claim_token,
        )
    })
    .await
    .map_err(|_| retitle_worker_failed())??;
    let inference = match claimed {
        crate::retitle_v3::ProviderClaim::Claimed(inference) => inference,
        crate::retitle_v3::ProviderClaim::Waiting(response) => {
            return Ok((StatusCode::ACCEPTED, response));
        }
        crate::retitle_v3::ProviderClaim::RecoveredTerminal(response) => {
            return Ok((StatusCode::OK, response));
        }
    };
    if inference.trigger == "automatic" && retitle_v3_manual_waiting(&state, &id) {
        let context = state.context.clone();
        let id_for_response = id.clone();
        let inference_for_release = inference.clone();
        let response = tokio::task::spawn_blocking(move || {
            crate::retitle_v3::release_unused_automatic_claim(
                &context,
                &id_for_response,
                &inference_for_release,
            )
        })
        .await
        .map_err(|_| retitle_worker_failed())??;
        return Ok((StatusCode::ACCEPTED, response));
    }
    let permit = match state.retitle.acquire().await {
        Ok(permit) => permit,
        Err(error) => {
            let failure_class = crate::retitle::provider_failure_class(&error);
            let response = complete_retitle_v3_failure(
                &state,
                &id,
                &inference,
                &failure_class,
                "provider_admission",
                &[],
            )
            .await?;
            return Ok((StatusCode::OK, response));
        }
    };
    let input = inference.input.clone();
    let existing = inference.existing.clone();
    let trigger = inference.trigger.clone();
    let evaluated = match tokio::task::spawn_blocking(move || {
        crate::retitle::infer_semantic_memory_observed(&permit, &input, existing.as_ref(), &trigger)
    })
    .await
    {
        Ok(evaluated) => evaluated,
        Err(_) => {
            let response = complete_retitle_v3_failure(
                &state,
                &id,
                &inference,
                "worker_failed",
                "provider_worker",
                &[],
            )
            .await?;
            return Ok((StatusCode::OK, response));
        }
    };
    match evaluated {
        Ok(observed) => {
            let context = state.context.clone();
            let catalog = state.history_catalog.clone();
            let id_for_commit = id.clone();
            let inference_for_commit = inference.clone();
            let provider_attempts = observed.providers.clone();
            let committed = tokio::task::spawn_blocking(move || {
                crate::retitle_v3::commit_inference(
                    &context,
                    &catalog,
                    &id_for_commit,
                    &inference_for_commit,
                    observed.inferred.state,
                    &observed.providers,
                )
            })
            .await
            .map_err(|_| retitle_worker_failed())?;
            match committed {
                Ok(committed) => {
                    invalidate_managed_history_titles(&state.managed_history_titles);
                    Ok((StatusCode::OK, committed))
                }
                Err(error)
                    if matches!(
                        error.code(),
                        "retitle-v3-state-conflict"
                            | "retitle-v3-turn-conflict"
                            | "retitle-v3-history-conflict"
                            | "retitle-v3-history-stale"
                    ) =>
                {
                    let context = state.context.clone();
                    let id_for_conflict = id.clone();
                    let failure_class = error.code().to_string();
                    let terminal = tokio::task::spawn_blocking(move || {
                        crate::retitle_v3::complete_inference_conflict(
                            &context,
                            &id_for_conflict,
                            &inference,
                            &failure_class,
                            &provider_attempts,
                        )
                    })
                    .await
                    .map_err(|_| retitle_worker_failed())??;
                    Ok((StatusCode::OK, terminal))
                }
                Err(error) => Err(error),
            }
        }
        Err(failure) => {
            let failure_class = crate::retitle::provider_failure_class(&failure.error);
            let failure_stage = observed_provider_failure_stage(&failure.providers).to_string();
            let cached = complete_retitle_v3_failure(
                &state,
                &id,
                &inference,
                &failure_class,
                &failure_stage,
                &failure.providers,
            )
            .await?;
            let _ = failure.error;
            Ok((StatusCode::OK, cached))
        }
    }
}

async fn complete_retitle_v3_failure(
    state: &ServeState,
    id: &str,
    inference: &crate::retitle_v3::InferenceContext,
    failure_class: &str,
    failure_stage: &str,
    provider_attempts: &[crate::retitle::ProviderAttemptObservation],
) -> Result<crate::retitle_v3::RetitleV3Response, CliError> {
    let context = state.context.clone();
    let catalog = state.history_catalog.clone();
    let completion_id = id.to_string();
    let conflict_id = id.to_string();
    let inference_for_completion = inference.clone();
    let failure_class = failure_class.to_string();
    let failure_stage = failure_stage.to_string();
    let attempts_for_completion = provider_attempts.to_vec();
    let attempts_for_conflict = provider_attempts.to_vec();
    let completion = tokio::task::spawn_blocking(move || {
        crate::retitle_v3::complete_provider_failure(
            &context,
            &catalog,
            &completion_id,
            &inference_for_completion,
            &failure_class,
            &failure_stage,
            &attempts_for_completion,
        )
    })
    .await
    .map_err(|_| retitle_worker_failed())?;
    match completion {
        Ok(response) => Ok(response),
        Err(error)
            if matches!(
                error.code(),
                "retitle-v3-state-conflict"
                    | "retitle-v3-turn-conflict"
                    | "retitle-v3-history-conflict"
                    | "retitle-v3-history-stale"
            ) =>
        {
            let context = state.context.clone();
            let inference = inference.clone();
            let failure_class = error.code().to_string();
            tokio::task::spawn_blocking(move || {
                crate::retitle_v3::complete_inference_conflict(
                    &context,
                    &conflict_id,
                    &inference,
                    &failure_class,
                    &attempts_for_conflict,
                )
            })
            .await
            .map_err(|_| retitle_worker_failed())?
        }
        Err(error) => Err(error),
    }
}

async fn execute_retitle(
    state: Arc<ServeState>,
    id: String,
    request: crate::retitle::RetitleRequest,
) -> Result<Value, CliError> {
    let _session_guard = state.retitle.lock_session(&id).await;
    let context = state.context.clone();
    let request_for_admission = request.clone();
    let id_for_admission = id.clone();
    let admission = tokio::task::spawn_blocking(move || {
        crate::retitle::admit(&context, &id_for_admission, &request_for_admission)
    })
    .await
    .map_err(|_| retitle_worker_failed())?
    .map_err(sanitize_retitle_error)?;

    if let crate::retitle::Admission::Replay(
        record,
        diagnostic_code,
        coverage,
        replayed_provider_kind,
        replayed_observation,
    ) = admission
    {
        return Ok(retitle_projection(
            &record,
            &request,
            RetitleResponseProjection {
                provider_kind: replayed_provider_kind.as_deref().unwrap_or_else(|| {
                    state
                        .retitle
                        .configured_provider_kind()
                        .unwrap_or("command")
                }),
                outcome: "replayed",
                changed: false,
                diagnostic_code,
                coverage,
                attempt: replayed_observation.as_ref(),
            },
        ));
    }
    let crate::retitle::Admission::Evaluate(observed) = admission else {
        unreachable!();
    };
    match execute_retitle_evaluation(state.clone(), &id, &request, observed).await {
        Ok(result) => Ok(result),
        Err(failure) => {
            let _ = record_retitle_failure(&state, &id, &request, &failure).await;
            Err(sanitize_retitle_error(failure.error))
        }
    }
}

struct RetitleEvaluationFailure {
    error: CliError,
    failure_stage: &'static str,
    context: Option<crate::retitle::RetitleContextObservation>,
    providers: Vec<crate::retitle::ProviderAttemptObservation>,
    started: Instant,
}

impl RetitleEvaluationFailure {
    fn new(error: CliError, failure_stage: &'static str, started: Instant) -> Self {
        Self {
            error,
            failure_stage: stable_retitle_failure_stage(failure_stage),
            context: None,
            providers: Vec::new(),
            started,
        }
    }
}

fn stable_retitle_failure_stage(stage: &str) -> &'static str {
    match stage {
        "queue" => "queue",
        "context_worker" => "context_worker",
        "context_read" => "context_read",
        "provider_admission" => "provider_admission",
        "provider_worker" => "provider_worker",
        "provider_setup" => "provider_setup",
        "provider_call" => "provider_call",
        "provider_response" => "provider_response",
        "provider_parse" => "provider_parse",
        "provider_budget" => "provider_budget",
        "provider" => "provider",
        "commit_worker" => "commit_worker",
        "commit" => "commit",
        _ => "unknown",
    }
}

async fn execute_retitle_evaluation(
    state: Arc<ServeState>,
    id: &str,
    request: &crate::retitle::RetitleRequest,
    observed: SessionRecord,
) -> Result<Value, Box<RetitleEvaluationFailure>> {
    let started = Instant::now();
    let permit = state
        .retitle
        .acquire()
        .await
        .map_err(|error| Box::new(RetitleEvaluationFailure::new(error, "queue", started)))?;
    let history_catalog = state.history_catalog.clone();
    let trigger = request.trigger;
    let observed_for_context = observed.clone();
    let context_result = tokio::task::spawn_blocking(move || {
        crate::retitle::build_context(&history_catalog, &observed_for_context, trigger, &permit)
            .map(|title_context| (permit, title_context))
    })
    .await
    .map_err(|_| {
        Box::new(RetitleEvaluationFailure::new(
            retitle_worker_failed(),
            "context_worker",
            started,
        ))
    })?;
    let (permit, title_context) = match context_result {
        Ok(value) => value,
        Err(error) => {
            let mut failure = RetitleEvaluationFailure::new(error, "context_read", started);
            failure.context = Some(crate::retitle::RetitleContextObservation {
                source: "provider_transcript".to_string(),
                freshness: "unavailable".to_string(),
                complete: false,
                truncated: false,
                turn_count: 0,
                context_chars: 0,
                provider_input_chars: 0,
            });
            return Err(Box::new(failure));
        }
    };
    let coverage = title_context.coverage_view();
    let context_observation = title_context.observation();
    let existing = observed.title_state.clone();
    let decision_result = tokio::task::spawn_blocking(move || {
        crate::retitle::infer_title_state_observed(&permit, &title_context, existing.as_ref())
    })
    .await
    .map_err(|_| {
        let mut failure =
            RetitleEvaluationFailure::new(retitle_worker_failed(), "provider_worker", started);
        failure.context = Some(context_observation.clone());
        Box::new(failure)
    })?;
    let inference = match decision_result {
        Ok(value) => value,
        Err(observed_error) => {
            let failure_stage = observed_provider_failure_stage(&observed_error.providers);
            return Err(Box::new(RetitleEvaluationFailure {
                error: observed_error.error,
                failure_stage,
                context: Some(context_observation),
                providers: observed_error.providers,
                started,
            }));
        }
    };
    let inferred = inference.inferred;
    let provider_attempts = inference.providers;
    let provider_kind = inferred.provider_kind;
    let decision = inferred.state;
    let context = state.context.clone();
    let request_for_commit = request.clone();
    let id_for_commit = id.to_string();
    let coverage_for_commit = coverage.clone();
    let evidence_for_commit = crate::retitle::RetitleAttemptEvidence {
        context: Some(context_observation.clone()),
        providers: provider_attempts.clone(),
        elapsed: Duration::ZERO,
        started: Some(started),
    };
    let committed = tokio::task::spawn_blocking(move || {
        crate::retitle::commit(
            &context,
            &id_for_commit,
            &request_for_commit,
            decision,
            &coverage_for_commit,
            provider_kind,
            &evidence_for_commit,
        )
    })
    .await
    .map_err(|_| {
        let mut failure =
            RetitleEvaluationFailure::new(retitle_worker_failed(), "commit_worker", started);
        failure.context = Some(context_observation.clone());
        failure.providers = provider_attempts.clone();
        Box::new(failure)
    })?
    .map_err(|error| {
        Box::new(RetitleEvaluationFailure {
            error,
            failure_stage: "commit",
            context: Some(context_observation),
            providers: provider_attempts,
            started,
        })
    })?;
    if committed.changed {
        invalidate_managed_history_titles(&state.managed_history_titles);
    }
    if let Some(observation) = committed.observation.as_ref() {
        emit_retitle_observation(observation);
    }
    Ok(retitle_projection(
        &committed.record,
        request,
        RetitleResponseProjection {
            provider_kind,
            outcome: if committed.changed {
                "committed"
            } else {
                "unchanged"
            },
            changed: committed.changed,
            diagnostic_code: if committed.changed {
                "committed"
            } else {
                "no_change"
            },
            coverage,
            attempt: committed.observation.as_ref(),
        },
    ))
}

fn observed_provider_failure_stage(
    providers: &[crate::retitle::ProviderAttemptObservation],
) -> &'static str {
    providers
        .last()
        .and_then(|attempt| attempt.failure_stage.as_deref())
        .map(stable_retitle_failure_stage)
        .filter(|stage| *stage != "unknown")
        .unwrap_or("provider")
}

#[derive(Clone)]
struct AutomaticRetitleCandidate {
    id: String,
    tracking_key: String,
    request: crate::retitle_v3::RetitleV3Request,
}

#[derive(Default)]
struct AutomaticRetitleTracker {
    entries: HashMap<String, AutomaticRetitleAttempt>,
}

struct AutomaticRetitleAttempt {
    attempts: u8,
    retry_after: Instant,
    in_flight: bool,
    terminal: bool,
}

fn automatic_retitle_scanner_error_is_retryable(code: &str) -> bool {
    code == "retitle-v3-history-unavailable"
        || crate::retitle::is_retryable_automatic_error_code(code)
}

impl AutomaticRetitleTracker {
    fn begin(&mut self, key: &str, now: Instant) -> bool {
        match self.entries.entry(key.to_string()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(AutomaticRetitleAttempt {
                    attempts: 1,
                    retry_after: now,
                    in_flight: true,
                    terminal: false,
                });
                true
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                let attempt = entry.get_mut();
                if attempt.terminal || attempt.in_flight || now < attempt.retry_after {
                    return false;
                }
                attempt.attempts = attempt.attempts.saturating_add(1);
                attempt.in_flight = true;
                true
            }
        }
    }

    fn finish(&mut self, key: &str, result: Result<&str, &CliError>, now: Instant) {
        let Some(attempt) = self.entries.get_mut(key) else {
            return;
        };
        attempt.in_flight = false;
        match result {
            Ok("terminal") => attempt.terminal = true,
            Ok(_) => {
                // An accepted durable operation is still pending. Polling it
                // must not consume the bounded provider-failure retry budget.
                attempt.attempts = attempt.attempts.saturating_sub(1);
                attempt.retry_after = now + Duration::from_secs(1);
            }
            Err(error)
                if automatic_retitle_scanner_error_is_retryable(error.code())
                    && attempt.attempts < crate::retitle::MAX_AUTOMATIC_ATTEMPTS =>
            {
                let shift = u32::from(attempt.attempts.saturating_sub(1));
                attempt.retry_after = now + Duration::from_secs(1_u64 << shift);
            }
            Err(_) => attempt.terminal = true,
        }
    }

    fn prune(&mut self) {
        if self.entries.len() <= 4096 {
            return;
        }
        self.entries
            .retain(|_, attempt| !attempt.terminal || attempt.in_flight);
        if self.entries.len() > 4096 {
            self.entries.clear();
        }
    }
}

fn automatic_retitle_candidate(session: &SessionView) -> Option<AutomaticRetitleCandidate> {
    if session.title_mode == crate::display_metadata::TitleMode::Pinned
        || session.status != "running"
        || session.provider_resume.is_none()
    {
        return None;
    }
    let incarnation = session.session_incarnation.as_deref()?;
    let state = session.turn_state.as_ref()?;
    let (turn_id, trigger) = if let Some(turn_id) = state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
    {
        let trigger = if session.title_revision == 0 && session.title.is_none() {
            crate::retitle::RetitleTrigger::Initial
        } else {
            crate::retitle::RetitleTrigger::Prompt
        };
        (turn_id, trigger)
    } else {
        (
            state
                .last_turn
                .as_ref()
                .and_then(|turn| turn.provider_turn_id.as_deref())?,
            crate::retitle::RetitleTrigger::CompletionRecovery,
        )
    };
    let tracking_identity = format!("{incarnation}\0{turn_id}\0{trigger:?}");
    let request_identity = format!(
        "{tracking_identity}\0{}\0{}",
        state.revision, session.title_revision
    );
    // The request key identifies the latest observation, while v3's automatic
    // operation fingerprint intentionally coalesces all revisions of this
    // incarnation/provider-turn pair into one durable operation.
    Some(AutomaticRetitleCandidate {
        id: session.id.clone(),
        tracking_key: format!(
            "auto-track-{}",
            crate::retitle::hash_identity(&tracking_identity).trim_start_matches("sha256:")
        ),
        request: crate::retitle_v3::RetitleV3Request {
            schema_version: crate::retitle_v3::REQUEST_SCHEMA.to_string(),
            trigger: "automatic".to_string(),
            idempotency_key: format!(
                "auto-{}",
                crate::retitle::hash_identity(&request_identity).trim_start_matches("sha256:")
            ),
            expected: crate::retitle_v3::RetitleV3FenceInput {
                session_incarnation: incarnation.to_string(),
                title_revision: session.title_revision,
                memory_revision: session.retitle_v3_memory_revision,
                activity_revision: Some(state.revision),
                provider_turn_id: Some(turn_id.to_string()),
            },
        },
    })
}

async fn auto_retitle_loop(state: Arc<ServeState>) {
    let mut tracker = AutomaticRetitleTracker::default();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        if state.retitle.configured_provider_kind().is_none() {
            continue;
        }
        let context = state.context.clone();
        let tmux = state.tmux_bin.clone();
        let collect = state.session_collector.clone();
        let Ok(Ok(sessions)) = tokio::task::spawn_blocking(move || collect(&context, &tmux)).await
        else {
            continue;
        };
        let mut tasks = JoinSet::new();
        let mut pending_keys = HashSet::new();
        let now = Instant::now();
        for candidate in sessions
            .iter()
            .filter_map(automatic_retitle_candidate)
            .collect::<Vec<_>>()
        {
            let operation_hash =
                crate::retitle_v3::request_operation_hash(&candidate.id, &candidate.request);
            if RetitleV3ContinuationGuard::is_registered(
                &state.retitle_v3_continuations,
                &operation_hash,
            ) {
                continue;
            }
            let key = candidate.tracking_key.clone();
            if !tracker.begin(&key, now) {
                continue;
            }
            pending_keys.insert(key.clone());
            let state = state.clone();
            tasks.spawn(async move {
                let result = execute_retitle_v3(state, candidate.id, candidate.request).await;
                (key, result)
            });
        }
        while let Some(result) = tasks.join_next().await {
            if let Ok((key, result)) = result {
                pending_keys.remove(&key);
                let outcome = match result.as_ref() {
                    Ok((_, response)) => Ok(response.status.as_str()),
                    Err(error) => Err(error),
                };
                tracker.finish(&key, outcome, Instant::now());
            }
        }
        let worker_failed = retitle_worker_failed();
        for key in pending_keys {
            tracker.finish(&key, Err(&worker_failed), Instant::now());
        }
        tracker.prune();
    }
}

async fn record_retitle_failure(
    state: &ServeState,
    id: &str,
    request: &crate::retitle::RetitleRequest,
    failure: &RetitleEvaluationFailure,
) -> Option<crate::retitle::RetitleAttemptObservation> {
    let context = state.context.clone();
    let id = id.to_string();
    let request = request.clone();
    let code = if crate::retitle::is_public_error_code(failure.error.code()) {
        failure.error.code()
    } else {
        "retitle-worker-failed"
    }
    .to_string();
    let failure_stage = failure.failure_stage.to_string();
    let context_observation = failure.context.clone();
    let providers = failure.providers.clone();
    let started = failure.started;
    let observation = tokio::task::spawn_blocking(move || {
        let evidence = crate::retitle::RetitleAttemptEvidence {
            context: context_observation,
            providers,
            elapsed: Duration::ZERO,
            started: Some(started),
        };
        crate::retitle::fail_attempt(&context, &id, &request, &code, &failure_stage, &evidence)
    })
    .await
    .ok()
    .flatten();
    if let Some(observation) = observation.as_ref() {
        emit_retitle_observation(observation);
    }
    observation
}

fn emit_retitle_observation(observation: &crate::retitle::RetitleAttemptObservation) {
    eprintln!("{}", retitle_observation_log(observation));
}

fn retitle_observation_log(observation: &crate::retitle::RetitleAttemptObservation) -> Value {
    json!({
        "event": "agent_session_retitle_attempt",
        "retitle": observation,
    })
}

struct RetitleResponseProjection<'a> {
    provider_kind: &'a str,
    outcome: &'static str,
    changed: bool,
    diagnostic_code: &'static str,
    coverage: crate::retitle::CoverageView,
    attempt: Option<&'a crate::retitle::RetitleAttemptObservation>,
}

fn retitle_projection(
    record: &SessionRecord,
    request: &crate::retitle::RetitleRequest,
    projection: RetitleResponseProjection<'_>,
) -> Value {
    let mut response = json!({
        "schema_version": crate::retitle::RESPONSE_SCHEMA,
        "outcome": projection.outcome,
        "changed": projection.changed,
        "trigger": request.trigger,
        "provider_kind": projection.provider_kind,
        "processed_turn_id_hash": request.expected_provider_turn_id.as_deref().map(crate::retitle::hash_identity),
        "diagnostic_code": projection.diagnostic_code,
        "coverage": projection.coverage,
        "session": {
            "title": record.title,
            "title_state": crate::effective_session_title_state(record),
            "title_revision": record.title_revision,
            "session_incarnation": crate::coordination::incarnation(record).ok(),
        },
    });
    if let Some(attempt) = projection.attempt {
        response
            .as_object_mut()
            .expect("retitle response object")
            .insert("attempt".to_string(), json!(attempt));
    }
    response
}

fn retitle_worker_failed() -> CliError {
    crate::retitle::retitle_error(
        "retitle-worker-failed",
        "title worker failed",
        true,
        "retry",
        "retry_request",
    )
}

fn sanitize_retitle_error(error: CliError) -> CliError {
    if crate::retitle::is_public_error_code(error.code()) {
        error
    } else {
        retitle_worker_failed()
    }
}

/// `GET /board/v1` (`docs/specs/session-board-v1.md`); the projection lives in
/// `board.rs`.
async fn board_snapshot_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if !state.board {
        return status_json(
            StatusCode::NOT_FOUND,
            crate::board::DISABLED_CODE,
            crate::board::DISABLED_MESSAGE,
        );
    }
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let machine = state.machine.clone();
    let federation = state.federation.is_some();
    match tokio::task::spawn_blocking(move || {
        crate::board::snapshot(&context, &tmux, &machine, federation)
    })
    .await
    {
        Ok(Ok(board)) => envelope_ok(json!({ "machine": state.machine, "board": board })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

/// `GET /board/programs/v1`: the program lanes of this machine's sessions;
/// the cache lives in `board/programs.rs`.
async fn board_programs_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if !state.board {
        return status_json(
            StatusCode::NOT_FOUND,
            crate::board::DISABLED_CODE,
            crate::board::DISABLED_MESSAGE,
        );
    }
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let machine = state.machine.clone();
    let programs = state.board_programs.clone();
    match tokio::task::spawn_blocking(move || programs.snapshot(&context, &machine)).await {
        Ok(Ok(programs)) => {
            envelope_ok(json!({ "machine": state.machine, "board_programs": programs }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardClosedQuery {
    since: Option<String>,
}

/// `GET /board/closed/v1?since=<cursor>`; the ledger lives in `board/ledger.rs`.
async fn board_closed_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    query: Result<Query<BoardClosedQuery>, QueryRejection>,
) -> Response {
    if !state.board {
        return status_json(
            StatusCode::NOT_FOUND,
            crate::board::DISABLED_CODE,
            crate::board::DISABLED_MESSAGE,
        );
    }
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return envelope_err(crate::board::cursor_invalid());
    };
    let context = state.context.clone();
    let machine = state.machine.clone();
    match tokio::task::spawn_blocking(move || {
        crate::board::closed(&context, query.since.as_deref(), &machine)
    })
    .await
    {
        Ok(Ok(closed)) => envelope_ok(json!({ "machine": state.machine, "board_closed": closed })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

/// `GET /sessions/{id}/board/v1`: a managed session's board query, forwarded
/// to the aggregator by `board/relay.rs`.
async fn board_relay_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    axum::extract::RawQuery(query): axum::extract::RawQuery,
) -> Response {
    if !state.board {
        return status_json(
            StatusCode::NOT_FOUND,
            crate::board::DISABLED_CODE,
            crate::board::DISABLED_MESSAGE,
        );
    }
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let context = state.context.clone();
    let federation = state.federation.clone();
    match tokio::task::spawn_blocking(move || {
        crate::board::relay_route(&context, federation.as_ref(), &id, &token, query.as_deref())
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

/// `POST /sessions/{id}/console-start/v1`: start or reconcile an owned child.
async fn console_start_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<Value>, JsonRejection>,
) -> Response {
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let Ok(Json(body)) = body else {
        return status_json(
            StatusCode::BAD_REQUEST,
            "console-start-invalid",
            "the console start body must be a JSON object",
        );
    };
    let context = state.context.clone();
    let federation = state.federation.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::console_start::relay_route(
            &context,
            federation.as_ref(),
            &id,
            &token,
            &body,
        )
    })
    .await
    {
        Ok(Ok(value)) => (StatusCode::CREATED, Json(value)).into_response(),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn list_handler(State(state): State<Arc<ServeState>>) -> Response {
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let collect = state.session_collector.clone();
    let launch_profiles = state.launch_profiles.clone();
    match tokio::task::spawn_blocking(move || {
        collect(&context, &tmux).map(|mut sessions| {
            // List projection must observe removals and readiness failures on
            // this request. Probe once per configured profile, then reuse that
            // request-local snapshot for every stopped session below.
            let agent_profiles = launch_profiles.ready_summaries();
            for session in &mut sessions {
                if !session.resumable || session.status != "stopped" {
                    continue;
                }
                let blocked = if session
                    .agent_profile
                    .as_deref()
                    .is_some_and(|profile_id| launch_profiles.get(profile_id).is_none())
                {
                    session
                        .agent_profile
                        .as_deref()
                        .map(unavailable_launch_profile)
                } else {
                    match &session.profile_resume_context {
                        Ok(Some(durable)) => {
                            match validate_durable_launch_profile_context(durable, &launch_profiles)
                            {
                                Ok(Some(profile))
                                    if !agent_profiles
                                        .iter()
                                        .any(|ready| ready.id == profile.id) =>
                                {
                                    Some(unavailable_launch_profile(&profile.id))
                                }
                                Ok(_) => None,
                                Err(err) => Some(err),
                            }
                        }
                        Ok(None) => None,
                        Err(err) => Some(err.clone()),
                    }
                };
                let Some(err) = blocked else { continue };
                session.resumable = false;
                session.resume_blocked_reason = Some(err.code().to_string());
            }
            (sessions, agent_profiles)
        })
    })
    .await
    {
        Ok(Ok((mut sessions, agent_profiles))) => {
            enrich_last_prompts(&state, &mut sessions).await;
            envelope_ok(json!({
                "machine": state.machine,
                "observed_at": activity_observed_at(),
                "sessions": sessions,
                "agent_profiles": agent_profiles,
                "coordination": {"remote_messaging_supported":state.federation.is_some()},
                "capabilities": {
                    "profile_resume_import": true,
                    "managed_resume_command": false,
                    "resume_blocked_reason": true,
                    "last_prompt": true,
                    "last_message": true,
                    "history": true,
                    "archive": true,
                    "group_archive": true,
                    "history_star": true,
                    "managed_account_handoff": crate::codex_app_server::MANAGED_ACCOUNT_HANDOFF_CAPABILITY,
                    "claude_account_switch": crate::claude_account::broker_is_configured(),
                    "codex_account_switch": crate::codex_account::broker_is_configured(),
                    "session_retitle_v2": true,
                    "session_retitle_v3": true,
                    "session_retitle_v3_receipts": true,
                },
            }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn history_list_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    Query(query): Query<HistoryListQuery>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let catalog = state.history_catalog.clone();
    let title_cache = state.managed_history_titles.clone();
    let context = state.context.clone();
    let launch_profiles = state.launch_profiles.clone();
    let machine = state.machine.clone();
    let response_machine = machine.clone();
    match tokio::task::spawn_blocking(move || {
        let titles = managed_history_titles(&context, &title_cache);
        let mut page = catalog.list_with_titles(
            &machine,
            query.q.as_deref(),
            query.provider.as_deref(),
            query.cursor.as_deref(),
            query.limit.unwrap_or(50),
            &titles,
        )?;
        let ready_profiles = launch_profiles.ready_summaries();
        for session in &mut page.sessions {
            if session.provider == AgentKind::Dsh.as_str()
                && let Some(profile_id) = session.agent_profile.as_deref()
            {
                let supported = ready_profiles
                    .iter()
                    .any(|profile| profile.id == profile_id && profile.history_resume_supported);
                session.resumable = supported;
            }
        }
        Ok::<_, HistoryError>(page)
    })
    .await
    {
        Ok(Ok(page)) => envelope_ok(json!({
            "machine": response_machine,
            "observed_at": activity_observed_at(),
            "sessions": page.sessions,
            "next_cursor": page.next_cursor,
            "truncated": page.truncated,
            "capabilities": {
                "metadata_search": true,
                "full_text_search": false,
                "transcript_messages": true,
                "latest_message_paging": true,
                "archive": true,
                "star": true,
                "resume": true,
            },
        })),
        Ok(Err(error)) => history_error_response(error),
        Err(_) => join_err(),
    }
}

async fn history_resume_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response = history_resume_handler_inner(State(state), headers, AxPath(id.clone())).await;
    crate::lifecycle::response(&context, &id, "history-resume", before, response).await
}

async fn history_resume_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let tmux_bin = state.tmux_bin.clone();
    let catalog = state.history_catalog.clone();
    let profiles = state.launch_profiles.clone();
    let machine = state.machine.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("history-resume");
        let history = catalog.resolve_fresh(&id).map_err(history_resume_catalog_error)?;
        let expected_id = provider_history::stable_history_id(
            &history.provider,
            history.agent_profile.as_deref(),
            &history.provider_session_id,
        );
        if expected_id != id {
            return Err(CliError::data(
                "history-resume-identity-mismatch",
                "history identity changed before resume",
                None,
            ));
        }
        let restored = history_resume_identity(&context, &history)?;
        match history.provider.as_str() {
            "dsh" => Err(history_resume_not_supported(
                "DSH history resume is no longer supported",
            )),
            "codex" | "claude" => {
                let agent = AgentKind::from_name(&history.provider).expect("matched provider");
                let profile = history
                    .agent_profile
                    .as_deref()
                    .map(|profile_id| {
                        let profile = profiles
                            .get(profile_id)
                            .ok_or_else(|| history_resume_profile_unavailable(profile_id))?;
                        if !profile.provider_resume_import_supported() {
                            return Err(history_resume_not_supported(
                                "the history launch profile does not support provider resume import",
                            ));
                        }
                        if !profile.is_ready() {
                            return Err(history_resume_profile_unavailable(profile_id));
                        }
                        Ok(profile)
                    })
                    .transpose()?;
                start_provider_resume_session(
                    &context,
                    ProviderResumeImportArgs {
                        ordinary_start: None,
                        launch_env: crate::launch_env::LaunchEnv::new(),
                        title_mode: restored.title_mode,
                        agent,
                        provider_resume_id: history.provider_session_id,
                        title: history.title,
                        title_state: restored.title_state,
                        id: restored.id,
                        coordination_mode: cli::CoordinationMode::Advisory,
                        tmux_bin: Some(tmux_bin),
                        agent_bin: profile.map(|profile| profile.agent_bin.clone()),
                        agent_profile: profile.map(|profile| profile.id.clone()),
                        provider_config_dir: profile
                            .and_then(|profile| profile.provider_config_dir.clone()),
                        profile_auto_resume_supported: profile
                            .map(|profile| profile.auto_resume_supported),
                        profile_graceful_shutdown: profile
                            .and_then(|profile| profile.graceful_shutdown)
                            .map(|mode| mode.as_str().to_string()),
                        codex_usage_account: profile
                            .and_then(|profile| profile.codex_usage_account.clone()),
                        agent_args: Vec::new(),
                        initial_lineage: create_body_lineage(&machine, None, None, None)?,
                        format: nils_common::cli_contract::OutputFormat::Json,
                    },
                )
            }
            _ => Err(history_resume_not_supported(
                "the history provider does not support daemon-owned resume",
            )),
        }
    })
    .await;
    match result {
        Ok(Ok(view)) => envelope_ok(json!({ "machine": state.machine, "session": view.result })),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

/// Managed identity a history resume restores.
struct RestoredHistoryIdentity {
    title_mode: crate::display_metadata::TitleMode,
    /// The archived session id, when the archive recorded one and it is free.
    id: Option<String>,
    title_state: Option<crate::SessionTitleState>,
}

/// Refuse a conversation a live managed session already owns, so resume never
/// gives one conversation two session ids, and restore the archived session id
/// so peers addressing it keep reaching the conversation.
fn history_resume_identity(
    context: &CliContext,
    history: &provider_history::HistorySession,
) -> Result<RestoredHistoryIdentity, CliError> {
    if let Some(live_id) = live_managed_session_for_history(context, &history.id) {
        return Err(CliError::data(
            "history-session-live",
            "a live session already owns this conversation",
            Some(json!({ "id": live_id })),
        ));
    }
    let id = history
        .archived_session_id
        .clone()
        .filter(|id| crate::validate_id(id).is_ok() && !crate::session_dir(context, id).exists());
    // A title edited after its structured state was captured no longer renders
    // from that state; restore the title alone rather than fail the resume.
    let title_state = history.archived_title_state.clone().filter(|state| {
        crate::canonicalize_structured_title_pair(
            history.title.clone(),
            history.title.is_some(),
            state.clone(),
        )
        .is_ok()
    });
    Ok(RestoredHistoryIdentity {
        id,
        title_state,
        title_mode: history.archived_title_mode,
    })
}

fn live_managed_session_for_history(context: &CliContext, history_id: &str) -> Option<String> {
    let entries = fs::read_dir(context.state_dir.join("sessions")).ok()?;
    entries.flatten().find_map(|entry| {
        let id = entry.file_name().to_str()?.to_string();
        if !managed_history_record_is_bounded(&entry.path()) {
            return None;
        }
        let record = load_session_record(context, &id).ok()?;
        let resume = record.provider_resume.as_ref()?;
        (provider_history::stable_history_id(
            &resume.provider,
            crate::session_agent_profile(&record),
            &resume.session_id,
        ) == history_id)
            .then_some(id)
    })
}

fn history_resume_catalog_error(error: HistoryError) -> CliError {
    match error {
        HistoryError::NotFound => CliError::data(
            "history-session-not-found",
            "history session was not found",
            None,
        ),
        HistoryError::InvalidCursor => CliError::data(
            "history-resume-identity-mismatch",
            "history identity is invalid",
            None,
        ),
        HistoryError::Io => {
            CliError::runtime("history-read-failed", "history could not be read", None)
        }
    }
}

fn history_resume_not_supported(message: &'static str) -> CliError {
    CliError::data("history-resume-not-supported", message, None)
}

fn history_resume_profile_unavailable(profile_id: &str) -> CliError {
    CliError::unavailable(
        "history-resume-profile-unavailable",
        "the history launch profile is unavailable on this machine",
        Some(json!({ "agent_profile": profile_id })),
    )
}

/// Toggle the star on one history session. The star is stored by history id, so
/// it survives archiving and is shared by every client reading this machine.
async fn history_star_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<HistoryStarBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "history star request requires starred",
                None,
            ));
        }
    };
    let catalog = state.history_catalog.clone();
    let starred_at = body.starred.then(activity_observed_at);
    let history_id = id.clone();
    match tokio::task::spawn_blocking(move || catalog.set_star(&history_id, starred_at.as_deref()))
        .await
    {
        Ok(Ok(starred_at)) => envelope_ok(json!({
            "machine": state.machine,
            "history_id": id,
            "starred_at": starred_at,
        })),
        Ok(Err(error)) => history_error_response(error),
        Err(_) => join_err(),
    }
}

fn managed_history_titles(
    context: &CliContext,
    cache: &StdMutex<ManagedHistoryTitleCache>,
) -> BTreeMap<String, String> {
    let generation = {
        let cache = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache
            .refreshed_at
            .is_some_and(|refreshed_at| refreshed_at.elapsed() < MANAGED_HISTORY_TITLE_CACHE_TTL)
        {
            return cache.titles.clone();
        }
        cache.generation
    };

    let titles = scan_managed_history_titles(
        context,
        MANAGED_HISTORY_TITLE_MAX_ENTRIES,
        MANAGED_HISTORY_TITLE_SCAN_MAX_DURATION,
    );
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if cache.generation == generation {
        cache.refreshed_at = Some(Instant::now());
        cache.titles = titles.clone();
    }
    titles
}

fn scan_managed_history_titles(
    context: &CliContext,
    max_entries: usize,
    max_duration: Duration,
) -> BTreeMap<String, String> {
    scan_managed_history_titles_with_stats(context, max_entries, max_duration).0
}

fn scan_managed_history_titles_with_stats(
    context: &CliContext,
    max_entries: usize,
    max_duration: Duration,
) -> (BTreeMap<String, String>, usize) {
    let root = context.state_dir.join("sessions");
    let Ok(entries) = fs::read_dir(root) else {
        return (BTreeMap::new(), 0);
    };
    let deadline = Instant::now() + max_duration;
    let mut visited = 0;
    let mut candidates: BTreeMap<String, Option<String>> = BTreeMap::new();
    for entry in entries.flatten().take(max_entries) {
        if Instant::now() >= deadline {
            break;
        }
        visited += 1;
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !managed_history_record_is_bounded(&entry.path()) {
            continue;
        }
        let Ok(record) = load_session_record(context, &id) else {
            continue;
        };
        let Some(resume) = record.provider_resume.as_ref() else {
            continue;
        };
        let Some(title) = record
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
        else {
            continue;
        };
        let history_id = provider_history::stable_history_id(
            &resume.provider,
            crate::session_agent_profile(&record),
            &resume.session_id,
        );
        match candidates.entry(history_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(title.to_string()));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if entry.get().as_deref() != Some(title) {
                    entry.insert(None);
                }
            }
        }
    }
    let titles = candidates
        .into_iter()
        .filter_map(|(history_id, title)| title.map(|title| (history_id, title)))
        .collect();
    (titles, visited)
}

fn managed_history_record_is_bounded(session_dir: &Path) -> bool {
    let record_path = session_dir.join("session.json");
    let Ok(record_metadata) = fs::symlink_metadata(record_path) else {
        return false;
    };
    if !record_metadata.file_type().is_file()
        || record_metadata.len() > MANAGED_HISTORY_TITLE_MAX_RECORD_BYTES
    {
        return false;
    }

    let sidecar_path = session_dir.join(crate::SESSION_RESUME_FILE);
    match fs::symlink_metadata(sidecar_path) {
        Ok(metadata) => {
            metadata.file_type().is_file()
                && metadata.len() <= MANAGED_HISTORY_TITLE_MAX_RECORD_BYTES
        }
        Err(error) => error.kind() == io::ErrorKind::NotFound,
    }
}

fn invalidate_managed_history_titles(cache: &StdMutex<ManagedHistoryTitleCache>) {
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.generation = cache.generation.wrapping_add(1);
    cache.refreshed_at = None;
    cache.titles.clear();
}

fn history_error_response(error: HistoryError) -> Response {
    match error {
        HistoryError::NotFound => status_json(
            StatusCode::NOT_FOUND,
            "history-session-not-found",
            "history session was not found",
        ),
        HistoryError::InvalidCursor => status_json(
            StatusCode::BAD_REQUEST,
            "invalid-history-cursor",
            "history cursor is invalid",
        ),
        HistoryError::Io => status_json(
            StatusCode::INTERNAL_SERVER_ERROR,
            "history-read-failed",
            "history could not be read",
        ),
    }
}

/// Best-effort enrichment: attach each running Codex/Claude session's most recent
/// user prompt to the list projection.
///
/// Cold recovery and bounded backlog catch-up run in the background. Eligible
/// sessions report `last_prompt_state` so a consumer can distinguish pending
/// catch-up from an authoritative empty or unavailable projection. The latest
/// caught-up value remains process-local and is returned in the response only;
/// it is never persisted by the daemon.
async fn enrich_last_prompts(state: &Arc<ServeState>, sessions: &mut [SessionView]) {
    for session in sessions.iter_mut() {
        if session.status == "stopped" || !matches!(session.agent.as_str(), "codex" | "claude") {
            continue;
        }
        let context = state.context.clone();
        let id = session.id.clone();
        let Ok(Ok(record)) =
            tokio::task::spawn_blocking(move || load_session_record(&context, &id)).await
        else {
            continue;
        };
        if let Some(projection) = state
            .provider_prompt_discovery
            .last_prompt_projection(&record)
            .await
        {
            session.previews = crate::provider_prompt::SessionPreviews {
                last_prompt_state: Some(projection.state),
                last_prompt_continuity: projection.continuity,
                last_prompt: projection.prompt,
                last_message: projection.message,
            };
        }
    }
}

async fn codex_accounts_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let agent_bin = crate::resolve_agent_bin(AgentKind::Codex, None);
    let state_dir = state.context.state_dir.clone();
    match tokio::task::spawn_blocking(move || {
        crate::codex_account::list_inventory().map(|inventory| {
            let readiness = codex_app_server::account_binding_readiness(&agent_bin, &state_dir);
            (inventory, readiness)
        })
    })
    .await
    {
        Ok(Ok((inventory, readiness))) => envelope_ok(json!({
            "machine": state.machine,
            "provider": "codex",
            "accounts": inventory.accounts,
            "selection_strategies": inventory.selection_strategies,
            "readiness": readiness,
        })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn claude_accounts_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    match tokio::task::spawn_blocking(crate::claude_account::list_accounts).await {
        Ok(Ok(inventory)) => envelope_ok(json!({
            "machine": state.machine,
            "provider": "claude",
            "accounts": inventory.accounts,
            "selection_strategies": inventory.selection_strategies,
        })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn activity_events_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok());
    let subscription = match state.activity_broker.subscribe(last_event_id) {
        Ok(subscription) => subscription,
        Err(ActivitySubscribeError::Unavailable) => {
            return status_json(
                StatusCode::SERVICE_UNAVAILABLE,
                "activity-stream-unavailable",
                "activity streaming is degraded or unavailable; use session polling",
            );
        }
        Err(ActivitySubscribeError::Capacity) => {
            return status_json(
                StatusCode::TOO_MANY_REQUESTS,
                "activity-stream-capacity",
                "activity stream subscriber capacity reached; retry with polling fallback",
            );
        }
    };
    let stream = futures_util::stream::unfold(subscription, |mut subscription| async move {
        subscription
            .next_event()
            .await
            .map(|event| (Ok::<_, Infallible>(event.sse.clone()), subscription))
    });
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        CONTENT_TYPE,
        "text/event-stream".parse().expect("static header"),
    );
    response.headers_mut().insert(
        "cache-control",
        "no-cache, no-transform".parse().expect("static header"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().expect("static header"));
    response
}

async fn glance_handler(
    State(state): State<Arc<ServeState>>,
    AxPath(id): AxPath<String>,
    Query(query): Query<GlanceQuery>,
) -> Response {
    let context = state.context.clone();
    let args = cli::GlanceArgs {
        id,
        tail: query.tail.unwrap_or(cli::DEFAULT_GLANCE_TAIL),
        tmux_bin: Some(state.tmux_bin.clone()),
        format: nils_common::cli_contract::OutputFormat::Json,
    };
    match tokio::task::spawn_blocking(move || glance_session(&context, args)).await {
        Ok(Ok(glance)) => envelope_ok(json!({ "machine": state.machine, "glance": glance })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

#[allow(clippy::result_large_err)]
fn coordination_authority(state: &ServeState, headers: &HeaderMap) -> Result<String, Response> {
    if let Some(response) = deny_unauthorized(state, headers) {
        return Err(response);
    }
    coordination_server::capability_from_headers(headers)
        .map(str::to_string)
        .map_err(envelope_err)
}

fn coordination_ok(state: &ServeState, value: Value) -> Response {
    envelope_ok(json!({ "machine": state.machine, "coordination": value }))
}

async fn coordination_context_show_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || coordination_server::show(&context, &id)).await {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_check_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::CheckBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let capability = coordination_server::capability_from_headers(&headers)
        .ok()
        .map(str::to_string);
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::check(&context, &id, capability.as_deref(), body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_registry_context_check_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Result<Json<coordination_server::CheckBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || coordination_server::check_candidate(&context, body))
        .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_claim_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::ClaimBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::claim(&context, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_renew_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::ClaimMutationBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    coordination_context_claim_mutation(state, headers, id, body, false).await
}

async fn coordination_context_release_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::ClaimMutationBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    coordination_context_claim_mutation(state, headers, id, body, true).await
}

async fn coordination_context_claim_mutation(
    state: Arc<ServeState>,
    headers: HeaderMap,
    id: String,
    body: coordination_server::ClaimMutationBody,
    release: bool,
) -> Response {
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::claim_mutation(&context, &id, &token, body, release)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_admit_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::AdmitBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::admit(&context, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_complete_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::CompleteBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::complete(&context, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_context_reconcile_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::ReconcileBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::reconcile(&context, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_broker_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || coordination_server::broker_status(&context, &id))
        .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_broker_adopt_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::BrokerRecoveryBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    coordination_broker_recovery_handler(state, headers, id, body, false).await
}

async fn coordination_broker_reconcile_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::BrokerRecoveryBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    coordination_broker_recovery_handler(state, headers, id, body, true).await
}

async fn coordination_broker_recovery_handler(
    state: Arc<ServeState>,
    headers: HeaderMap,
    id: String,
    body: coordination_server::BrokerRecoveryBody,
    reconcile: bool,
) -> Response {
    let capability = match coordination_authority(&state, &headers) {
        Ok(capability) => capability,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::broker_recover(&context, &id, capability.as_str(), body, reconcile)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_operator_reconcile_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, lease_id)): AxPath<(String, String)>,
    body: Result<Json<coordination_server::OperatorOperationReconcileBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::operator_reconcile_operation(&context, &id, &lease_id, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_operator_provider_turn_reconcile_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::OperatorProviderTurnReconcileBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::operator_reconcile_provider_turn(&context, &id, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_inbox_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    query: Result<Query<coordination_server::InboxQuery>, QueryRejection>,
) -> Response {
    let query = match coordination_query(query) {
        Ok(query) => query,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::inbox(&context, &id, &token, query)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_send_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<coordination_server::SendBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::send(&context, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => {
            state.coordination_notification_wake.notify_one();
            coordination_ok(&state, notification_controller_available(value))
        }
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

#[allow(clippy::result_large_err)]
fn coordination_json<T>(body: Result<Json<T>, JsonRejection>) -> Result<T, Response> {
    body.map(|Json(value)| value).map_err(|_| {
        envelope_err(CliError::usage(
            "invalid-json-body",
            "coordination request body is invalid",
            None,
        ))
    })
}

#[allow(clippy::result_large_err)]
fn coordination_query<T>(query: Result<Query<T>, QueryRejection>) -> Result<T, Response> {
    query.map(|Query(value)| value).map_err(|_| {
        envelope_err(CliError::usage(
            "invalid-query",
            "coordination request query is invalid",
            None,
        ))
    })
}

async fn coordination_message_show_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, message_id)): AxPath<(String, String)>,
) -> Response {
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::message_show(&context, &id, &message_id, &token)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_ack_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, message_id)): AxPath<(String, String)>,
    body: Result<Json<coordination_server::AckBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::ack(&context, &id, &message_id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn coordination_reply_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, message_id)): AxPath<(String, String)>,
    body: Result<Json<coordination_server::ReplyBody>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        coordination_server::reply(&context, &id, &message_id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => {
            state.coordination_notification_wake.notify_one();
            coordination_ok(&state, notification_controller_available(value))
        }
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

fn notification_controller_available(mut value: Value) -> Value {
    if let Some(controller_available) = value.pointer_mut("/notification/controller_available") {
        *controller_available = Value::Bool(true);
    }
    value
}

async fn coordination_wait_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, message_id)): AxPath<(String, String)>,
    body: Result<Query<coordination_server::WaitBody>, QueryRejection>,
) -> Response {
    let body = match coordination_query(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let token = match coordination_authority(&state, &headers) {
        Ok(token) => token,
        Err(response) => return response,
    };
    let permit = match state.coordination_wait_workers.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return status_json(
                StatusCode::TOO_MANY_REQUESTS,
                "rate-limited",
                "coordination wait worker limit reached",
            );
        }
    };
    let context = state.context.clone();
    let cancelled = Arc::new(AtomicBool::new(false));
    let _cancellation = CoordinationWaitCancellation(cancelled.clone());
    match tokio::task::spawn_blocking(move || {
        let _permit = permit;
        coordination_server::wait_with_cancellation(
            &context,
            &id,
            &message_id,
            &token,
            body,
            Some(cancelled.as_ref()),
        )
    })
    .await
    {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

struct CoordinationWaitCancellation(Arc<AtomicBool>);

impl Drop for CoordinationWaitCancellation {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

// Return the session server's tmux paste buffer (`tmux show-buffer`) — the text a
// mouse-selection-copying TUI (e.g. Claude Code) placed there. Read-only, open on
// loopback like `glance`; the browser edge uses it so a right-click Copy can reach
// the on-screen selection that a live TUI never exposes to the DOM.
async fn buffer_handler(
    State(state): State<Arc<ServeState>>,
    AxPath(id): AxPath<String>,
) -> Response {
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || session_clipboard_buffer(&context, &id, &tmux)).await
    {
        Ok(Ok(text)) => envelope_ok(json!({ "machine": state.machine, "text": text })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

fn resolve_initial_codex_account(
    agent: AgentKind,
    explicit_account: Option<String>,
) -> Result<(Option<String>, Option<String>), CliError> {
    if let Some(account) = explicit_account {
        // An explicit account must be valid and listed before anything
        // launches, as for Claude.
        crate::codex_account::ensure_listed(&account)?;
        return Ok((Some(account), Some("explicit".to_string())));
    }
    if agent != AgentKind::Codex || !crate::codex_account::broker_is_configured() {
        return Ok((None, None));
    }
    // Intentional: the automatic default is best effort. A broker that cannot
    // list its strategies leaves the new session unbound on the host login
    // instead of failing the create; only an explicit account fails closed.
    if !crate::codex_account::broker_advertises_selection_strategy("current_default")
        .unwrap_or(false)
    {
        return Ok((None, None));
    }
    let selected = crate::codex_account::select_account("current_default")?;
    Ok((
        Some(selected.account),
        Some("default_at_launch".to_string()),
    ))
}

async fn wait_for_codex_control(
    state: &ServeState,
    id: &str,
    launch_id: &str,
) -> Option<ControlHandle> {
    wait_for_codex_control_within(state, id, launch_id, STRUCTURED_PROMPT_CONTROL_WAIT).await
}

async fn wait_for_codex_control_within(
    state: &ServeState,
    id: &str,
    launch_id: &str,
    timeout: Duration,
) -> Option<ControlHandle> {
    let deadline = Instant::now() + timeout;
    loop {
        let handle = state.codex_controls.lock().ok().and_then(|controls| {
            controls
                .get(id)
                .filter(|entry| entry.launch_id == launch_id)
                .map(|entry| entry.handle.clone())
        });
        if handle.is_some() || Instant::now() >= deadline {
            return handle;
        }
        tokio::time::sleep(STRUCTURED_PROMPT_CONTROL_POLL).await;
    }
}

#[allow(clippy::result_large_err)]
async fn wait_for_account_binding(
    state: &ServeState,
    id: &str,
    launch_id: &str,
    requested_account: &str,
    requested_revision: u64,
) -> Result<crate::codex_account::CodexAccountView, Response> {
    let deadline = Instant::now() + CODEX_ACCOUNT_BINDING_WAIT;
    let mut poll_delay = STRUCTURED_PROMPT_CONTROL_POLL;
    loop {
        let context = state.context.clone();
        let load_id = id.to_string();
        let record = match tokio::task::spawn_blocking(move || {
            load_session_record(&context, &load_id)
        })
        .await
        {
            Ok(Ok(record)) => record,
            Ok(Err(err)) => return Err(envelope_err(err)),
            Err(_) => return Err(join_err()),
        };
        if record
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.launch_id != launch_id)
        {
            return Err(status_json(
                StatusCode::CONFLICT,
                "codex-account-runtime-changed",
                "Codex session runtime changed while binding its account",
            ));
        }
        let view = crate::codex_account::view_for_record(&record);
        let requested_binding = view.selected_account.as_deref() == Some(requested_account)
            && view.revision == requested_revision;
        match view.state {
            "bound"
                if requested_binding && view.applied_runtime_id.as_deref() == Some(launch_id) =>
            {
                return Ok(view);
            }
            "bound" | "pending" | "failed" if !requested_binding => {
                return Err(status_json(
                    StatusCode::CONFLICT,
                    "codex-account-binding-superseded",
                    "the requested Codex account binding was superseded",
                ));
            }
            "bound" => {
                return Err(status_json(
                    StatusCode::CONFLICT,
                    "codex-account-binding-superseded",
                    "the requested Codex account binding was applied to another runtime",
                ));
            }
            "failed" | "unsupported" => {
                return Err(status_json(
                    StatusCode::BAD_GATEWAY,
                    "codex-account-binding-failed",
                    "Codex account binding failed",
                ));
            }
            _ if Instant::now() >= deadline => {
                return Err(status_json(
                    StatusCode::GATEWAY_TIMEOUT,
                    "codex-account-binding-timeout",
                    "Codex account binding timed out",
                ));
            }
            _ => {
                tokio::time::sleep(poll_delay).await;
                poll_delay = (poll_delay * 2).min(Duration::from_millis(500));
            }
        }
    }
}

pub(crate) fn structured_prompt_incarnation_conflict(
    id: &str,
    expected_session_incarnation: &str,
    actual_session_incarnation: Option<&str>,
) -> CliError {
    CliError::data(
        "session-incarnation-conflict",
        "session runtime changed before prompt submission",
        Some(json!({
            "id": id,
            "expected_session_incarnation": expected_session_incarnation,
            "actual_session_incarnation": actual_session_incarnation,
        })),
    )
}

#[allow(clippy::result_large_err)]
async fn load_structured_prompt_record_locked(
    state: &ServeState,
    id: &str,
    expected_session_incarnation: Option<&str>,
) -> Result<SessionRecord, Response> {
    let context = state.context.clone();
    let id = id.to_string();
    let expected_session_incarnation = expected_session_incarnation.map(str::to_string);
    match tokio::task::spawn_blocking(move || {
        let _record_lock = crate::acquire_session_record_lock(&context, &id)?;
        let current = load_session_record(&context, &id)?;
        if let Some(expected) = expected_session_incarnation {
            let actual = current
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.as_str());
            if actual != Some(expected.as_str()) {
                return Err(structured_prompt_incarnation_conflict(
                    &current.id,
                    &expected,
                    actual,
                ));
            }
        }
        Ok::<_, CliError>(current)
    })
    .await
    {
        Ok(Ok(record)) => Ok(record),
        Ok(Err(err)) => Err(envelope_err(err)),
        Err(_) => Err(join_err()),
    }
}

#[allow(clippy::result_large_err)]
async fn wait_for_structured_prompt_control(
    state: &ServeState,
    id: &str,
    launch_id: &str,
    expected_session_incarnation: Option<&str>,
    timeout: Duration,
) -> Result<ControlHandle, Response> {
    if let Some(handle) = wait_for_codex_control_within(state, id, launch_id, timeout).await {
        return Ok(handle);
    }

    let unavailable_fence = expected_session_incarnation.unwrap_or(launch_id);
    load_structured_prompt_record_locked(state, id, Some(unavailable_fence)).await?;
    Err(status_json(
        StatusCode::CONFLICT,
        "structured-prompt-unavailable",
        "structured prompt control is not ready for this session",
    ))
}

async fn session_account_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Json(body): Json<AccountSwitchBody>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response =
        session_account_handler_inner(State(state), headers, AxPath(id.clone()), Json(body)).await;
    crate::lifecycle::response(&context, &id, "account-switch", before, response).await
}

async fn session_account_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Json(body): Json<AccountSwitchBody>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Some(expected_session_incarnation) = body
        .expected_session_incarnation
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= 128)
    else {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-session-incarnation",
            "expected_session_incarnation is required",
        );
    };
    let resolve_context = state.context.clone();
    let requested_id = id.clone();
    let resolved_record = match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
        load_session_record(&resolve_context, &requested_id)
    })
    .await
    {
        Ok(Ok(record)) => record,
        Ok(Err(err)) => return envelope_err(err),
        Err(_) => return join_err(),
    };
    let canonical_id = resolved_record.id;
    let _switch_guard = state.account_switches.lock(&canonical_id).await;
    let context = state.context.clone();
    let load_id = canonical_id.clone();
    let record =
        match tokio::task::spawn_blocking(move || load_session_record(&context, &load_id)).await {
            Ok(Ok(record)) => record,
            Ok(Err(err)) => return envelope_err(err),
            Err(_) => return join_err(),
        };
    if record.agent == AgentKind::Claude.as_str() {
        let expected_session_incarnation = expected_session_incarnation.to_string();
        return claude_account_switch_handler(
            &state,
            canonical_id,
            body.account,
            expected_session_incarnation,
        )
        .await;
    }
    let precheck_account = body.account.clone();
    let expected_session_incarnation = expected_session_incarnation.to_string();
    let launch_id = match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
        crate::session_account::codex_switch_precheck(
            &record,
            &expected_session_incarnation,
            &precheck_account,
        )
    })
    .await
    {
        Ok(Ok(launch_id)) => launch_id,
        Ok(Err(err)) => return envelope_err(err),
        Err(_) => return join_err(),
    };
    let Some(handle) = wait_for_codex_control(&state, &canonical_id, &launch_id).await else {
        return status_json(
            StatusCode::CONFLICT,
            "codex-account-control-unavailable",
            "Codex account control is not ready for this session",
        );
    };
    let begin_context = state.context.clone();
    let begin_id = canonical_id.clone();
    let begin_launch_id = launch_id.clone();
    let begin_account = body.account.clone();
    let supports_unbound_account_queue = body.supports_unbound_account_queue;
    let revision = match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
        crate::codex_account::begin_switch_binding(
            &begin_context,
            &begin_id,
            &begin_launch_id,
            &begin_account,
        )
    })
    .await
    {
        Ok(Ok(revision)) => revision,
        Ok(Err(err)) if err.code() == "codex-account-session-busy" => {
            // The turn is still working, so an immediate switch is unsafe.
            // Durably queue the account for the next prompt instead; the daemon
            // applies it at the idle boundary and any applied binding is left
            // unchanged until then.
            let queue_context = state.context.clone();
            let queue_id = canonical_id.clone();
            let queue_launch_id = launch_id.clone();
            let queue_account = body.account.clone();
            return match tokio::task::spawn_blocking(move || {
                let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
                crate::session_account::codex_queue_switch(
                    &queue_context,
                    &queue_id,
                    &queue_launch_id,
                    &queue_account,
                    supports_unbound_account_queue,
                )
            })
            .await
            {
                Ok(Ok(view)) => envelope_ok(json!({
                    "machine": state.machine,
                    "session_incarnation": launch_id,
                    "codex_account": view
                })),
                Ok(Err(err)) => envelope_err(err),
                Err(_) => join_err(),
            };
        }
        Ok(Err(err)) => return envelope_err(err),
        Err(_) => return join_err(),
    };
    match handle.bind_account(&body.account, revision).await {
        Ok(view) => envelope_ok(json!({
            "machine": state.machine,
            "session_incarnation": launch_id,
            "codex_account": view
        })),
        Err(_) => {
            let finish_context = state.context.clone();
            let finish_id = canonical_id.clone();
            let finish_launch_id = launch_id.clone();
            let finish_account = body.account.clone();
            let _ = tokio::task::spawn_blocking(move || {
                let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
                crate::codex_account::finish_binding(
                    &finish_context,
                    &finish_id,
                    &finish_launch_id,
                    &finish_account,
                    revision,
                    Err("apply_failed"),
                )
            })
            .await;
            status_json(
                StatusCode::BAD_GATEWAY,
                "codex-account-binding-failed",
                "Codex account binding failed",
            )
        }
    }
}

/// See [`crate::session_account::claude_switch_locked`].
async fn claude_account_switch_handler(
    state: &ServeState,
    id: String,
    account: String,
    expected_session_incarnation: String,
) -> Response {
    let context = state.context.clone();
    let tmux_bin = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("account-switch");
        crate::session_account::claude_switch_locked(
            &context,
            &id,
            &account,
            &expected_session_incarnation,
            &tmux_bin,
            true,
        )
    })
    .await
    {
        Ok(Ok((session_incarnation, view))) => envelope_ok(json!({
            "machine": state.machine,
            "session_incarnation": session_incarnation,
            "claude_account": view,
        })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn send_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Json(body): Json<SendBody>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    if body.keys.len() > MAX_SEND_KEYS {
        return envelope_err(CliError::usage(
            "too-many-keys",
            format!("send accepts at most {MAX_SEND_KEYS} keys per request"),
            Some(json!({ "limit": MAX_SEND_KEYS })),
        ));
    }
    let mut keys = Vec::with_capacity(MAX_SEND_KEYS);
    for name in &body.keys {
        match SpecialKey::from_name(name) {
            Some(key) => keys.push(key),
            None => {
                return envelope_err(CliError::usage(
                    "invalid-key",
                    format!("unknown key: {name}"),
                    None,
                ));
            }
        }
    }
    let context = state.context.clone();
    let args = crate::send::unguarded_args(
        id,
        body.text,
        keys,
        body.allow_blocked,
        state.tmux_bin.clone(),
    );
    match tokio::task::spawn_blocking(move || crate::send_to_session(&context, args)).await {
        Ok(Ok(sent)) => envelope_ok(json!({ "machine": state.machine, "sent": sent })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn structured_prompt_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Json(body): Json<StructuredPromptBody>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    structured_prompt::submit_structured_prompt_handler(
        state,
        id,
        body.text,
        body.expected_session_incarnation,
    )
    .await
}

pub(crate) mod prompt_receipts;

fn structured_prompt_has_unsafe_control(text: &str) -> bool {
    text.chars().any(|character| {
        character <= '\u{0009}'
            || ('\u{000b}'..='\u{001f}').contains(&character)
            || character == '\u{007f}'
    })
}

async fn auto_resume_status_handler(
    State(state): State<Arc<ServeState>>,
    AxPath(id): AxPath<String>,
) -> Response {
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        let record = load_session_record(&context, &id)?;
        Ok::<_, CliError>(auto_resume::view_for_record(&context, &record))
    })
    .await
    {
        Ok(Ok(view)) => envelope_ok(json!({ "machine": state.machine, "auto_resume": view })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn auto_resume_set_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<AutoResumeBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "request body must be JSON with a boolean enabled field",
                None,
            ));
        }
    };
    let context = state.context.clone();
    let now = jiff::Timestamp::now().to_string();
    match tokio::task::spawn_blocking(move || {
        auto_resume::set_enabled_with_policy(
            &context,
            &id,
            body.enabled,
            body.recovery_policy
                .as_deref()
                .unwrap_or(auto_resume::WAIT_FOR_RESET_POLICY),
            &now,
        )
    })
    .await
    {
        Ok(Ok(view)) => envelope_ok(json!({ "machine": state.machine, "auto_resume": view })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn auto_resume_cancel_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let now = jiff::Timestamp::now().to_string();
    match tokio::task::spawn_blocking(move || auto_resume::cancel(&context, &id, &now)).await {
        Ok(Ok(view)) => envelope_ok(json!({ "machine": state.machine, "auto_resume": view })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn auth_incident_loop(state: Arc<ServeState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(15));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let context = state.context.clone();
        let discovery_tmux = state.tmux_bin.clone();
        let records = tokio::task::spawn_blocking(move || {
            let mut records = Vec::new();
            let mut command = ProcessCommand::new(&discovery_tmux);
            command.args(["list-sessions", "-F", "#{session_name}"]);
            let live_tmux = crate::run_output_with_timeout_and_strict_cap(
                command,
                Duration::from_millis(250),
                64 * 1024,
            )
            .ok()
            .filter(|output| output.status.success())
            .map(|output| {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .map(str::to_string)
                    .collect::<HashSet<_>>()
            });
            if let Ok(entries) = fs::read_dir(context.state_dir.join("sessions")) {
                for entry in entries.flatten() {
                    if let Some(id) = entry.file_name().to_str()
                        && let Ok(record) = crate::load_session_record(&context, id)
                        && record.mode == "interactive"
                        && matches!(record.agent.as_str(), "claude" | "codex")
                        && record.runtime.is_some()
                    {
                        let terminal_available = live_tmux
                            .as_ref()
                            .map(|sessions| sessions.contains(&record.tmux_session));
                        records.push((record, terminal_available));
                    }
                }
            }
            records
        })
        .await;
        let Ok(records) = records else {
            continue;
        };
        // Bound concurrent sampling and mailbox requests; no provider text is
        // copied to the async scheduler or diagnostic output.
        let mut tasks = tokio::task::JoinSet::new();
        for (record, terminal_available) in records {
            if tasks.len() >= 4 {
                let _ = tasks.join_next().await;
            }
            let context = state.context.clone();
            let tmux = state.tmux_bin.clone();
            let machine = state.machine.clone();
            tasks.spawn_blocking(move || {
                if let Err(error) = crate::auth_incident::tick(
                    &context,
                    &record,
                    &tmux,
                    &machine,
                    terminal_available,
                ) {
                    eprintln!(
                        "warning: provider authentication observation degraded: {}",
                        error.code()
                    );
                }
            });
        }
        while tasks.join_next().await.is_some() {}
    }
}

async fn auto_resume_loop(state: Arc<ServeState>) {
    const POLL_INTERVAL: Duration = Duration::from_secs(15);
    loop {
        let now_epoch = jiff::Timestamp::now().as_second();
        let context = state.context.clone();
        let pending = match tokio::task::spawn_blocking(move || {
            auto_resume::pending_sessions(&context, now_epoch)
        })
        .await
        {
            Ok(Ok(pending)) => pending,
            Ok(Err(err)) => {
                eprintln!("warning: auto-resume discovery failed: {}", err.code());
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            Err(_) => {
                eprintln!("warning: auto-resume discovery worker failed");
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
        };

        for code in &pending.error_codes {
            eprintln!("warning: auto-resume session state skipped: {code}");
        }

        if !pending.recovery_ids.is_empty() {
            process_auto_resume_ids(
                state.clone(),
                pending.recovery_ids,
                UsageSnapshot {
                    authoritative: false,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                },
            )
            .await;
        }
        if !pending.capacity_ids.is_empty() {
            let partition = partition_auto_resume_ids(&state, pending.capacity_ids).await;
            process_codex_capacity_auto_resume_ids(state.clone(), partition.codex).await;
        }
        if !pending.usage_ids.is_empty() {
            let partition = partition_auto_resume_ids(&state, pending.usage_ids).await;
            if !partition.codex.is_empty() {
                process_codex_auto_resume_ids(state.clone(), partition.codex).await;
            }
            // `claude` launch profiles backed by a Codex account key auto-resume
            // off that account's rate limits, not native Claude usage.
            if !partition.codex_account_claude.is_empty() {
                process_codex_account_auto_resume_ids(
                    state.clone(),
                    partition.codex_account_claude,
                )
                .await;
            }
            if partition.claude.is_empty() {
                tokio::time::sleep(POLL_INTERVAL).await;
                continue;
            }
            let timeout = usage_timeout();
            let provider = tokio::task::spawn_blocking(move || collect_claude_usage(timeout))
                .await
                .unwrap_or_else(|_| provider_internal_error("claude", "Claude"));
            let usage = UsageSnapshot {
                authoritative: provider.fresh_authoritative,
                has_exhausted_windows: provider
                    .windows
                    .iter()
                    .any(|window| window.used_percent >= 100),
                exhausted_reset_epochs: provider
                    .windows
                    .iter()
                    .filter(|window| window.used_percent >= 100)
                    .filter_map(|window| window.reset_at_epoch)
                    .collect(),
                soonest_reset_epoch: provider
                    .windows
                    .iter()
                    .filter_map(|window| window.reset_at_epoch)
                    .min(),
            };
            process_auto_resume_ids(state.clone(), partition.claude, usage).await;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

async fn coordination_notification_loop(state: Arc<ServeState>) {
    const RECONCILE_INTERVAL: Duration = Duration::from_secs(1);
    loop {
        drain_coordination_notifications(state.clone()).await;
        tokio::select! {
            () = state.coordination_notification_wake.notified() => {}
            () = tokio::time::sleep(RECONCILE_INTERVAL) => {}
        }
    }
}

async fn drain_coordination_notifications(state: Arc<ServeState>) {
    reconcile_coordination_notifications(state.clone()).await;
    let context = state.context.clone();
    let candidates = match tokio::task::spawn_blocking(move || {
        crate::coordination::pending_notifications(&context)
    })
    .await
    {
        Ok(Ok(candidates)) => candidates,
        Ok(Err(err)) => {
            eprintln!(
                "warning: coordination notification discovery failed: {}",
                err.code()
            );
            return;
        }
        Err(_) => {
            eprintln!("warning: coordination notification discovery worker failed");
            return;
        }
    };
    let mut tasks = JoinSet::new();
    for candidate in candidates {
        while tasks.len() >= COORDINATION_NOTIFICATION_CONCURRENCY {
            let _ = tasks.join_next().await;
        }
        let dispatch_state = state.clone();
        tasks.spawn(async move {
            dispatch_coordination_notification(dispatch_state, candidate).await;
        });
    }
    while tasks.join_next().await.is_some() {}
}

async fn reconcile_coordination_notifications(state: Arc<ServeState>) {
    let context = state.context.clone();
    let candidates = match tokio::task::spawn_blocking(move || {
        crate::coordination::unresolved_notifications(&context)
    })
    .await
    {
        Ok(Ok(candidates)) => candidates,
        _ => return,
    };
    for candidate in candidates {
        let context = state.context.clone();
        let observed_candidate = candidate.clone();
        let observation = tokio::task::spawn_blocking(move || {
            crate::coordination::notification_prompt_observed_after(&context, &observed_candidate)
        })
        .await
        .ok()
        .flatten();
        let Some(observed) = observation else {
            continue;
        };
        let context = state.context.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if observed {
                crate::coordination::reconcile_notification_submitted(&context, &candidate)
            } else {
                crate::coordination::reconcile_notification_absent(&context, &candidate)
            }
        })
        .await;
    }
}

/// A recipient with no delivery path: a retained record whose agent this build
/// no longer knows (a retired `hermes` pane). Deferring would retry it until
/// the pane is replaced. DSH recipients are hook-delivered before this check.
fn coordination_notification_provider_unsupported(record: &crate::SessionRecord) -> bool {
    AgentKind::from_name(&record.agent).is_none()
}

async fn dispatch_coordination_notification(
    state: Arc<ServeState>,
    candidate: crate::coordination::NotificationCandidate,
) {
    let context = state.context.clone();
    let target_session_id = candidate.target_session_id.clone();
    let record = tokio::task::spawn_blocking(move || {
        let _record_lock = crate::acquire_session_record_lock(&context, &target_session_id)?;
        load_session_record(&context, &target_session_id)
    })
    .await;
    let record = match record {
        Ok(Ok(record)) => record,
        Ok(Err(_)) | Err(_) => {
            update_notification_undeliverable(&state, &candidate, "recipient-incarnation-replaced")
                .await;
            return;
        }
    };
    if record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        != Some(candidate.target_incarnation.as_str())
    {
        update_notification_undeliverable(&state, &candidate, "recipient-incarnation-replaced")
            .await;
        return;
    }
    if record.coordination_mode == cli::CoordinationMode::Off {
        update_notification_undeliverable(&state, &candidate, "coordination-disabled").await;
        return;
    }
    // DSH (an external `dsh` lane or a dsh launch-profile pane) has no serve
    // prompt route; its policy hook claims the reminder at a model-step
    // boundary through `message reminder`.
    if record.agent == AgentKind::Dsh.as_str() {
        update_notification_undeliverable(
            &state,
            &candidate,
            crate::coordination::NOTIFICATION_REASON_HOOK_DELIVERED,
        )
        .await;
        return;
    }
    if coordination_notification_provider_unsupported(&record) {
        update_notification_undeliverable(&state, &candidate, "provider-unsupported").await;
        return;
    }
    if record.agent == AgentKind::Claude.as_str() {
        dispatch_terminal_coordination_notification(state, record, candidate, AgentKind::Claude)
            .await;
        return;
    }
    let turn = activity::state_for_view(&state.context, &record);
    let waiting = turn
        .as_ref()
        .is_some_and(activity::codex_notification_maybe_idle);
    if codex_app_server::runtime_is_supported(&record)
        && let Some(active_turn_id) = turn.as_ref().and_then(|turn| {
            (matches!(
                turn.phase,
                activity::TurnPhase::Working | activity::TurnPhase::NeedsInput
            ) && turn.source.confidence == activity::Confidence::Authoritative)
                .then(|| {
                    turn.current_turn
                        .as_ref()
                        .and_then(|current| current.provider_turn_id.clone())
                })
                .flatten()
        })
    {
        dispatch_codex_in_turn_coordination_checkpoint(state, record, candidate, active_turn_id)
            .await;
        return;
    }
    if !waiting {
        update_notification_deferred(
            &state,
            &candidate,
            "recipient-working",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    }
    if !codex_app_server::runtime_is_supported(&record) {
        dispatch_terminal_coordination_notification(state, record, candidate, AgentKind::Codex)
            .await;
        return;
    }

    let prompt = crate::coordination::notification_prompt(&candidate);
    let response = structured_prompt::submit_structured_prompt_handler_with_fence(
        state.clone(),
        candidate.target_session_id.clone(),
        prompt,
        Some(candidate.target_incarnation.clone()),
        Some(CoordinationNotificationFence {
            candidate: candidate.clone(),
        }),
        false,
    )
    .await;
    let context = state.context.clone();
    if response.status().is_success() {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else if response.status() == StatusCode::INTERNAL_SERVER_ERROR {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &candidate,
                "submission-outcome-unknown",
            )
        })
        .await;
    } else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-not-ready",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
    }
}

async fn dispatch_codex_in_turn_coordination_checkpoint(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_turn_id: String,
) {
    let launch_id = candidate.target_incarnation.clone();
    let handle = match wait_for_structured_prompt_control(
        &state,
        &record.id,
        &launch_id,
        Some(&launch_id),
        STRUCTURED_PROMPT_CONTROL_WAIT,
    )
    .await
    {
        Ok(handle) => handle,
        Err(_) => {
            update_notification_deferred(
                &state,
                &candidate,
                "provider-not-ready",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
    };
    let prompt = crate::coordination::notification_prompt(&candidate);
    let lock_context = state.context.clone();
    let lock_id = record.id.clone();
    let lock_launch_id = launch_id.clone();
    let lock_turn_id = expected_turn_id.clone();
    let fence = candidate.clone();
    let locked = tokio::task::spawn_blocking(move || {
        let record_lock = crate::acquire_session_record_lock(&lock_context, &lock_id)?;
        let mut current = load_session_record(&lock_context, &lock_id)?;
        if current
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            != Some(lock_launch_id.as_str())
            || !codex_app_server::runtime_is_supported(&current)
        {
            return Err(structured_prompt_incarnation_conflict(
                &current.id,
                &lock_launch_id,
                current
                    .runtime
                    .as_ref()
                    .map(|runtime| runtime.launch_id.as_str()),
            ));
        }
        let eligible = activity::state_for_view(&lock_context, &current).is_some_and(|turn| {
            matches!(
                turn.phase,
                activity::TurnPhase::Working | activity::TurnPhase::NeedsInput
            ) && turn.source.confidence == activity::Confidence::Authoritative
                && turn
                    .current_turn
                    .as_ref()
                    .and_then(|active| active.provider_turn_id.as_deref())
                    == Some(lock_turn_id.as_str())
        });
        if !eligible {
            return Err(CliError::unavailable(
                "coordination-checkpoint-turn-changed",
                "the active turn changed before the coordination checkpoint",
                None,
            ));
        }
        crate::codex_account::authorize_terminal_input_locked(&lock_context, &mut current)?;
        crate::codex_account::ensure_terminal_input_allowed(&current)?;
        let mut quiescence = crate::coordination::lock_session_quiescence(
            &lock_context,
            &fence.target_session_id,
            &fence.target_incarnation,
        )?;
        if !quiescence.begin_notification_attempt(&fence)? {
            return Err(CliError::unavailable(
                "coordination-checkpoint-not-quiescent",
                "the recipient has an active claim or operation",
                None,
            ));
        }
        drop(record_lock);
        Ok::<_, CliError>(())
    })
    .await;
    match locked {
        Ok(Ok(())) => {}
        Ok(Err(err)) if err.code() == "coordination-checkpoint-not-quiescent" => {
            update_notification_deferred(
                &state,
                &candidate,
                "recipient-coordination-not-quiescent",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
        Ok(Err(_)) | Err(_) => {
            update_notification_deferred(
                &state,
                &candidate,
                "recipient-working",
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
    };

    let response = handle.steer_prompt(&prompt, &expected_turn_id).await;
    let context = state.context.clone();
    if response.as_deref() == Ok(expected_turn_id.as_str()) {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        })
        .await;
    } else {
        let _ = tokio::task::spawn_blocking(move || {
            crate::coordination::mark_notification_unknown(
                &context,
                &candidate,
                "submission-outcome-unknown",
            )
        })
        .await;
    }
}

async fn dispatch_terminal_coordination_notification(
    state: Arc<ServeState>,
    record: SessionRecord,
    candidate: crate::coordination::NotificationCandidate,
    expected_agent: AgentKind,
) {
    let Some(source) = ProviderPromptTail::resolve_source(&record) else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-observation-unavailable",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    };
    let Some(tail) = ProviderPromptTail::open_source_at_eof(source) else {
        update_notification_deferred(
            &state,
            &candidate,
            "provider-observation-unavailable",
            COORDINATION_NOTIFICATION_RETRY,
        )
        .await;
        return;
    };
    let prompt = crate::coordination::notification_prompt(&candidate);
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let start_candidate = candidate.clone();
    let start_prompt = prompt.clone();
    let started = tokio::task::spawn_blocking(move || {
        start_terminal_coordination_notification(
            &context,
            &tmux,
            &start_candidate,
            &start_prompt,
            tail,
            expected_agent,
        )
    })
    .await;
    let tail = match started {
        Ok(Ok(tail)) => tail,
        Ok(Err(TerminalNotificationStartError::Known(reason))) => {
            update_notification_deferred(
                &state,
                &candidate,
                reason,
                COORDINATION_NOTIFICATION_RETRY,
            )
            .await;
            return;
        }
        Ok(Err(TerminalNotificationStartError::UnknownReason(reason))) => {
            terminal_notification::mark_unknown_after_paste(&state, candidate, reason).await;
            return;
        }
        Ok(Err(TerminalNotificationStartError::Unknown)) | Err(_) => {
            terminal_notification::mark_unknown(&state, candidate, "submission-outcome-unknown")
                .await;
            return;
        }
    };

    let observed_prompt = prompt.clone();
    let observed = tokio::task::spawn_blocking(move || {
        observe_terminal_coordination_prompt(tail, &observed_prompt)
    })
    .await
    .unwrap_or(false);
    let context = state.context.clone();
    let _ = tokio::task::spawn_blocking(move || {
        if observed {
            crate::coordination::mark_notification_submitted(&context, &candidate)
        } else {
            crate::coordination::mark_notification_unknown(
                &context,
                &candidate,
                "submission-outcome-unknown",
            )
        }
    })
    .await;
}

mod terminal_notification;
use terminal_notification::{
    TerminalNotificationStartError, start_terminal_coordination_notification,
};

fn terminal_notification_waiting(
    context: &CliContext,
    record: &SessionRecord,
    expected_agent: AgentKind,
) -> bool {
    match expected_agent {
        AgentKind::Claude => activity::claude_notification_waiting(
            context,
            record,
            CLAUDE_NOTIFICATION_STOP_DEBOUNCE,
        ),
        AgentKind::Codex => activity::state_for_view(context, record).is_some_and(|turn| {
            turn.phase == activity::TurnPhase::Waiting
                && turn.source.confidence == activity::Confidence::Authoritative
                && turn
                    .last_turn
                    .as_ref()
                    .is_some_and(|last_turn| last_turn.outcome == "completed")
                && turn.current_turn.is_none()
        }),
        AgentKind::Dsh => false,
    }
}

fn tmux_session_attached(tmux_bin: &Path, tmux_session: &str) -> Result<bool, ()> {
    let mut command = ProcessCommand::new(tmux_bin);
    command
        .arg("display-message")
        .arg("-p")
        .arg("-t")
        .arg(format!("{tmux_session}:0.0"))
        .arg("#{session_attached}");
    let output = run_process_output_with_timeout(&mut command, ATTACH_TMUX_COMMAND_TIMEOUT)
        .map_err(|_| ())?;
    if !output.status.success() {
        return Err(());
    }
    let attached = String::from_utf8(output.stdout)
        .map_err(|_| ())?
        .trim()
        .parse::<u64>()
        .map_err(|_| ())?;
    Ok(attached != 0)
}

fn observe_terminal_coordination_prompt(mut tail: ProviderPromptTail, prompt: &str) -> bool {
    let deadline = Instant::now() + CLAUDE_NOTIFICATION_OBSERVATION_TIMEOUT;
    loop {
        match tail.poll_submitted_prompts() {
            Ok(prompts) if prompts.iter().any(|submitted| submitted == prompt) => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(CLAUDE_NOTIFICATION_OBSERVATION_POLL);
    }
}

async fn update_notification_deferred(
    state: &ServeState,
    candidate: &crate::coordination::NotificationCandidate,
    reason: &'static str,
    retry_after_seconds: i64,
) {
    let context = state.context.clone();
    let candidate = candidate.clone();
    let _ = tokio::task::spawn_blocking(move || {
        crate::coordination::defer_notification(&context, &candidate, reason, retry_after_seconds)
    })
    .await;
}

async fn update_notification_undeliverable(
    state: &ServeState,
    candidate: &crate::coordination::NotificationCandidate,
    reason: &'static str,
) {
    let context = state.context.clone();
    let candidate = candidate.clone();
    let _ = tokio::task::spawn_blocking(move || {
        crate::coordination::mark_notification_undeliverable(&context, &candidate, reason)
    })
    .await;
}

#[derive(Clone)]
struct CodexAutoResumeTarget {
    id: String,
    launch_id: String,
    binding: crate::codex_account::BindingSnapshot,
}

/// A `claude` auto-resume session whose launch profile declares an
/// authoritative Codex usage account. The loop keys its usage snapshot off the
/// Codex account's rate limits instead of native Claude usage.
#[derive(Clone)]
struct CodexAccountAutoResumeTarget {
    id: String,
    account: String,
}

#[derive(Default)]
struct AutoResumePartition {
    /// Codex sessions launched through the app-server v2 protocol.
    codex: Vec<CodexAutoResumeTarget>,
    /// `claude` sessions backed by an authoritative Codex usage account.
    codex_account_claude: Vec<CodexAccountAutoResumeTarget>,
    /// Everything else (native Claude, or sessions we could not classify).
    claude: Vec<String>,
}

async fn partition_auto_resume_ids(state: &ServeState, ids: Vec<String>) -> AutoResumePartition {
    let context = state.context.clone();
    tokio::task::spawn_blocking(move || {
        let mut partition = AutoResumePartition::default();
        for id in ids {
            let Ok(record) = load_session_record(&context, &id) else {
                partition.claude.push(id);
                continue;
            };
            if codex_app_server::runtime_is_supported(&record) {
                let binding = crate::codex_account::binding_snapshot(&record);
                match record.runtime {
                    Some(runtime) => partition.codex.push(CodexAutoResumeTarget {
                        id: record.id,
                        launch_id: runtime.launch_id,
                        binding,
                    }),
                    None => partition.claude.push(id),
                }
                continue;
            }
            match crate::session_codex_usage_account(&record) {
                Some(account) => {
                    partition
                        .codex_account_claude
                        .push(CodexAccountAutoResumeTarget {
                            id: record.id,
                            account,
                        });
                }
                None => partition.claude.push(id),
            }
        }
        partition
    })
    .await
    .unwrap_or_default()
}

async fn process_codex_auto_resume_ids(
    state: Arc<ServeState>,
    targets: Vec<CodexAutoResumeTarget>,
) {
    let mut tasks = JoinSet::new();
    for target in targets {
        let state = state.clone();
        tasks.spawn(async move { process_codex_auto_resume_id(state, target).await });
        if tasks.len() >= MAX_CONCURRENT_AUTO_RESUME_TICKS {
            report_auto_resume_join(tasks.join_next().await);
        }
    }
    while !tasks.is_empty() {
        report_auto_resume_join(tasks.join_next().await);
    }
}

async fn process_codex_capacity_auto_resume_ids(
    state: Arc<ServeState>,
    targets: Vec<CodexAutoResumeTarget>,
) {
    let mut tasks = JoinSet::new();
    for target in targets {
        let state = state.clone();
        tasks.spawn(async move { process_codex_capacity_auto_resume_id(state, target).await });
        if tasks.len() >= MAX_CONCURRENT_AUTO_RESUME_TICKS {
            report_auto_resume_join(tasks.join_next().await);
        }
    }
    while !tasks.is_empty() {
        report_auto_resume_join(tasks.join_next().await);
    }
}

async fn process_codex_capacity_auto_resume_id(
    state: Arc<ServeState>,
    target: CodexAutoResumeTarget,
) {
    let control = state
        .codex_controls
        .lock()
        .ok()
        .and_then(|controls| controls.get(&target.id).cloned());
    let Some(control) = control else {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "control_unavailable",
        )
        .await;
        return;
    };
    if control.launch_id != target.launch_id {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "control_unavailable",
        )
        .await;
        return;
    }
    let context = state.context.clone();
    let runtime = tokio::runtime::Handle::current();
    let handle = control.handle.clone();
    let expected_launch_id = target.launch_id;
    let id = target.id;
    tokio::task::spawn_blocking(move || {
        let now_epoch = jiff::Timestamp::now().as_second();
        if let Err(err) = auto_resume::tick_for_runtime_and_binding(
            &context,
            &id,
            &expected_launch_id,
            auto_resume::RuntimeBindingTick {
                binding: &target.binding,
                failover_account: None,
            },
            now_epoch,
            &UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                runtime
                    .block_on(handle.submit(auto_resume::CAPACITY_CONTINUATION_MESSAGE))
                    .map(|_| ())
                    .map_err(|_| {
                        CliError::runtime(
                            "codex-app-server-submit-unknown",
                            "Codex capacity continuation submission outcome is unknown",
                            Some(json!({ "id": id })),
                        )
                    })
            },
        ) {
            eprintln!(
                "warning: Codex capacity auto-resume tick failed: {}",
                err.code()
            );
        }
    })
    .await
    .ok();
}

async fn process_codex_auto_resume_id(state: Arc<ServeState>, target: CodexAutoResumeTarget) {
    let control = state
        .codex_controls
        .lock()
        .ok()
        .and_then(|controls| controls.get(&target.id).cloned());
    let Some(control) = control else {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "usage_unavailable",
        )
        .await;
        return;
    };
    if control.launch_id != target.launch_id {
        record_codex_scheduler_error_for_runtime(
            state.context.clone(),
            target.id,
            target.launch_id,
            "usage_unavailable",
        )
        .await;
        return;
    }
    let usage = match control.handle.usage().await {
        Ok(usage) => usage,
        Err(_) => {
            record_codex_scheduler_error_for_runtime(
                state.context.clone(),
                target.id,
                target.launch_id,
                "usage_unavailable",
            )
            .await;
            return;
        }
    };
    let context = state.context.clone();
    let runtime = tokio::runtime::Handle::current();
    let handle = control.handle.clone();
    let expected_launch_id = target.launch_id;
    let id = target.id;
    tokio::task::spawn_blocking(move || {
        let now_epoch = jiff::Timestamp::now().as_second();
        let failover_account = auto_resume::failover_selection_request(
            &context,
            &id,
            &expected_launch_id,
            &target.binding,
            usage.has_exhausted_windows,
        )
        .ok()
        .flatten()
        .and_then(|request| {
            crate::codex_account::select_next_account(&request.after, &request.excluded)
                .ok()
                .flatten()
        })
        .map(|account| account.account);
        if let Err(err) = auto_resume::tick_for_runtime_and_binding(
            &context,
            &id,
            &expected_launch_id,
            auto_resume::RuntimeBindingTick {
                binding: &target.binding,
                failover_account: failover_account.as_deref(),
            },
            now_epoch,
            &usage,
            |_| {
                runtime
                    .block_on(handle.submit(auto_resume::CONTINUATION_MESSAGE))
                    .map(|_| ())
                    .map_err(|_| {
                        CliError::runtime(
                            "codex-app-server-submit-unknown",
                            "Codex continuation submission outcome is unknown",
                            Some(json!({ "id": id })),
                        )
                    })
            },
        ) {
            eprintln!("warning: Codex auto-resume tick failed: {}", err.code());
        }
    })
    .await
    .ok();
}

async fn record_codex_scheduler_error_for_runtime(
    context: CliContext,
    id: String,
    expected_launch_id: String,
    reason: &'static str,
) {
    tokio::task::spawn_blocking(move || {
        let now_epoch = jiff::Timestamp::now().as_second();
        if let Err(err) = auto_resume::record_scheduler_error_for_runtime(
            &context,
            &id,
            &expected_launch_id,
            now_epoch,
            reason,
        ) {
            eprintln!(
                "warning: Codex auto-resume failure checkpoint failed: {}",
                err.code()
            );
        }
    })
    .await
    .ok();
}

async fn codex_control_loop(state: Arc<ServeState>) {
    const RECONCILE_INTERVAL: Duration = Duration::from_secs(2);
    loop {
        let context = state.context.clone();
        let tmux = state.tmux_bin.clone();
        let records = tokio::task::spawn_blocking(move || discover_codex_controls(&context, &tmux))
            .await
            .unwrap_or_default();
        let live: std::collections::HashSet<(String, String)> = records
            .iter()
            .filter_map(|record| {
                record
                    .runtime
                    .as_ref()
                    .map(|runtime| (record.id.clone(), runtime.launch_id.clone()))
            })
            .collect();
        if let Ok(mut controls) = state.codex_controls.lock() {
            controls.retain(|id, entry| live.contains(&(id.clone(), entry.launch_id.clone())));
        }
        let (mut pending_account_handles, mut repairs) = (Vec::new(), Vec::new());
        for record in records {
            let Some(launch_id) = record
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.clone())
            else {
                continue;
            };
            let running_handle = state.codex_controls.lock().ok().and_then(|controls| {
                controls
                    .get(&record.id)
                    .filter(|entry| entry.launch_id == launch_id)
                    .map(|entry| entry.handle.clone())
            });
            if let Some(handle) = running_handle {
                if crate::codex_account::queued_switch_repair(&record).is_some() {
                    repairs.push((record.id.clone(), launch_id, handle));
                } else if matches!(
                    crate::codex_account::pending_next_apply(&record),
                    Ok(Some(_))
                ) {
                    pending_account_handles.push(handle);
                }
                continue;
            }
            let prepare_context = state.context.clone();
            let prepare_id = record.id.clone();
            let prepare_launch_id = launch_id.clone();
            let record = match tokio::task::spawn_blocking(move || {
                crate::codex_account::prepare_control_reconnect(
                    &prepare_context,
                    &prepare_id,
                    &prepare_launch_id,
                )
            })
            .await
            {
                Ok(Ok(record)) => record,
                Ok(Err(err)) => {
                    eprintln!(
                        "warning: Codex account reconnect fence failed: {}",
                        err.code()
                    );
                    continue;
                }
                Err(_) => continue,
            };
            let (handle, commands, ready) = codex_app_server::starting_control_channel();
            if let Ok(mut controls) = state.codex_controls.lock() {
                controls.insert(
                    record.id.clone(),
                    CodexControlEntry {
                        launch_id: launch_id.clone(),
                        handle,
                    },
                );
            }
            let registry = state.codex_controls.clone();
            let cx = state.context.clone();
            tokio::spawn(async move {
                if let Err(err) =
                    codex_app_server::run_control(cx.clone(), record.clone(), commands, ready).await
                {
                    crate::lifecycle::recovery::record_control_close(&cx, &record, &err);
                }
                if let Ok(mut controls) = registry.lock()
                    && controls
                        .get(&record.id)
                        .is_some_and(|entry| entry.launch_id == launch_id)
                {
                    controls.remove(&record.id);
                }
            });
        }
        tokio::join!(
            apply_pending_codex_accounts(pending_account_handles),
            repair_blocked_codex_accounts(&state.context, &state.account_switches, repairs),
        );
        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

fn fence_codex_controls_before_listen(context: &CliContext, tmux: &Path) -> Result<(), CliError> {
    let root = context.state_dir.join("sessions");
    let candidates: Vec<SessionRecord> = match std::fs::read_dir(root) {
        Ok(entries) => entries
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|id| load_session_record(context, &id).ok())
            .filter(|record| {
                codex_app_server::runtime_is_supported(record)
                    && crate::codex_account::binding_is_present(record)
            })
            .collect(),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            return Err(CliError::runtime(
                "codex-account-reconnect-fence-unavailable",
                "failed to inspect session records before serving",
                None,
            ));
        }
    };
    if candidates.is_empty() {
        return Ok(());
    }
    let snapshots = crate::tmux_session_snapshots(tmux).ok_or_else(|| {
        CliError::runtime(
            "codex-account-reconnect-fence-unavailable",
            "failed to inspect live sessions before fencing Codex account controls",
            None,
        )
    })?;
    for record in candidates
        .into_iter()
        .filter(|record| snapshots.contains_key(&record.tmux_session))
    {
        let Some(launch_id) = record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
        else {
            continue;
        };
        crate::codex_account::prepare_control_reconnect(context, &record.id, launch_id)?;
    }
    Ok(())
}

fn discover_codex_controls(context: &CliContext, tmux: &Path) -> Vec<SessionRecord> {
    let Some(tmux_snapshots) = crate::tmux_session_snapshots(tmux) else {
        return Vec::new();
    };
    discover_codex_controls_from_snapshots(context, &tmux_snapshots)
}

fn discover_codex_controls_from_snapshots(
    context: &CliContext,
    tmux_snapshots: &crate::TmuxSessionSnapshots,
) -> Vec<SessionRecord> {
    let root = context.state_dir.join("sessions");
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|id| load_session_record(context, &id).ok())
        .filter(|record| {
            codex_app_server::runtime_is_supported(record)
                && tmux_snapshots.contains_key(&record.tmux_session)
        })
        .collect()
}

async fn process_auto_resume_ids(state: Arc<ServeState>, ids: Vec<String>, usage: UsageSnapshot) {
    let items = ids.into_iter().map(|id| (id, usage.clone())).collect();
    process_auto_resume_ids_with_usage(state, items).await;
}

/// Tick each `(id, usage)` pair, letting every session carry its own usage
/// snapshot. Codex-account-backed `claude` sessions each get a snapshot built
/// from their own Codex account's rate limits.
async fn process_auto_resume_ids_with_usage(
    state: Arc<ServeState>,
    items: Vec<(String, UsageSnapshot)>,
) {
    let mut tasks = JoinSet::new();
    for (id, usage) in items {
        let context = state.context.clone();
        let tmux = state.tmux_bin.clone();
        tasks.spawn_blocking(move || {
            let now_epoch = jiff::Timestamp::now().as_second();
            if let Err(err) = auto_resume::tick(&context, &id, now_epoch, &usage, |record| {
                send_auto_resume_input(&context, record, auto_resume::CONTINUATION_MESSAGE, &tmux)
            }) {
                eprintln!("warning: auto-resume tick failed: {}", err.code());
                if let Err(record_err) =
                    auto_resume::record_scheduler_error(&context, &id, now_epoch, "scheduler_error")
                {
                    eprintln!(
                        "warning: auto-resume failure checkpoint failed: {}",
                        record_err.code()
                    );
                }
            }
        });
        if tasks.len() >= MAX_CONCURRENT_AUTO_RESUME_TICKS {
            report_auto_resume_join(tasks.join_next().await);
        }
    }
    while !tasks.is_empty() {
        report_auto_resume_join(tasks.join_next().await);
    }
}

/// Resolve one shared `codex-cli diag rate-limits --all` read, then hand each
/// Codex-account-backed `claude` session the snapshot filtered to its own
/// account nickname. A failed read degrades every target to non-authoritative.
async fn process_codex_account_auto_resume_ids(
    state: Arc<ServeState>,
    targets: Vec<CodexAccountAutoResumeTarget>,
) {
    let timeout = usage_timeout();
    let output = tokio::task::spawn_blocking(move || {
        run_usage_helper("codex-cli", CODEX_ACCOUNT_USAGE_ARGS, &[], timeout)
    })
    .await;
    let output = match output {
        Ok(Ok(output)) => Some(output),
        // Fail safe: never fall back to native-Claude quota for a Codex-account
        // profile. A missing/failed read degrades to non-authoritative retry.
        _ => None,
    };
    let items = targets
        .into_iter()
        .map(|target| {
            let usage = match output.as_ref() {
                Some(output) => codex_account_usage_snapshot(output, &target.account),
                None => non_authoritative_usage_snapshot(),
            };
            (target.id, usage)
        })
        .collect();
    process_auto_resume_ids_with_usage(state, items).await;
}

fn report_auto_resume_join(result: Option<Result<(), tokio::task::JoinError>>) {
    if result.is_some_and(|result| result.is_err()) {
        eprintln!("warning: auto-resume worker failed");
    }
}

async fn resume_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response = resume_handler_inner(State(state), headers, AxPath(id.clone())).await;
    crate::lifecycle::response(&context, &id, "resume", before, response).await
}

async fn resume_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let launch_profiles = state.launch_profiles.clone();
    match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("resume");
        validate_launch_profile_resume(&context, &id, &launch_profiles)?;
        resume_session_by_id(&context, &id, &tmux)
    })
    .await
    {
        Ok(Ok(session)) => envelope_ok(json!({ "machine": state.machine, "session": session })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

fn validate_launch_profile_resume(
    context: &CliContext,
    id: &str,
    launch_profiles: &AgentLaunchProfiles,
) -> Result<(), CliError> {
    let record = load_session_record(context, id)?;
    validate_launch_profile_resume_record(&record, launch_profiles)
}

fn validate_launch_profile_resume_record(
    record: &SessionRecord,
    launch_profiles: &AgentLaunchProfiles,
) -> Result<(), CliError> {
    let Some(profile) = validate_launch_profile_context_record(record, launch_profiles)? else {
        return Ok(());
    };
    if !profile.is_ready() {
        return Err(unavailable_launch_profile(&profile.id));
    }
    Ok(())
}

fn validate_launch_profile_context_record<'a>(
    record: &SessionRecord,
    launch_profiles: &'a AgentLaunchProfiles,
) -> Result<Option<&'a AgentLaunchProfile>, CliError> {
    let Some(profile_id) = crate::session_agent_profile(record) else {
        return Ok(None);
    };
    if launch_profiles.get(profile_id).is_none() {
        return Err(unavailable_launch_profile(profile_id));
    }
    let Some(durable) = crate::durable_profile_resume_context(record)? else {
        return Err(crate::profile_metadata_unavailable(profile_id));
    };
    validate_durable_launch_profile_context(&durable, launch_profiles)
}

fn validate_durable_launch_profile_context<'a>(
    durable: &crate::DurableProfileResumeContext,
    launch_profiles: &'a AgentLaunchProfiles,
) -> Result<Option<&'a AgentLaunchProfile>, CliError> {
    let Some(profile) = launch_profiles.get(&durable.profile_id) else {
        return Err(unavailable_launch_profile(&durable.profile_id));
    };
    if durable.agent != profile.agent
        || durable.agent_bin != profile.agent_bin
        || durable.provider_config_dir.as_deref() != profile.provider_config_dir.as_deref()
        || durable.auto_resume_supported != profile.auto_resume_supported
    {
        return Err(drifted_launch_profile(&durable.profile_id));
    }
    Ok(Some(profile))
}

fn unavailable_launch_profile(profile_id: &str) -> CliError {
    profile_unavailable(profile_id)
}

fn drifted_launch_profile(profile_id: &str) -> CliError {
    CliError::runtime(
        "agent-profile-context-mismatch",
        "session launch profile context no longer matches the server registry",
        Some(json!({ "agent_profile": profile_id })),
    )
}

async fn maintenance_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    query: Result<Query<MaintenanceQuery>, QueryRejection>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let (operation, contract) = match query {
        Ok(Query(query)) => (query.operation, query.schema_version),
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-maintenance-query",
                "maintenance query requires one supported operation and contract version",
                Some(json!({ "field": "operation" })),
            ));
        }
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        maintenance::preview(&context, &id, &tmux, operation, contract)
    })
    .await
    {
        Ok(Ok(maintenance)) => {
            envelope_ok(json!({ "machine": state.machine, "maintenance": maintenance }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn maintenance_action_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<MaintenanceActionRequest>, JsonRejection>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let Json(request) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-maintenance-action-request",
                "maintenance action body does not match the versioned contract",
                None,
            ));
        }
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let launch_profiles = state.launch_profiles.clone();
    let action_id = id.clone();
    match tokio::task::spawn_blocking(move || {
        maintenance::execute_with_resume_guard(&context, &action_id, &tmux, request, |record| {
            validate_launch_profile_resume_record(record, &launch_profiles)
        })
    })
    .await
    {
        Ok(Ok(result)) => {
            if let Some(fence) = result.deleted_registry_fence().cloned() {
                cleanup_deleted_session_registries(&state, &fence).await;
            }
            envelope_ok(json!({ "machine": state.machine, "maintenance_result": result }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn cleanup_deleted_session_registries(state: &ServeState, fence: &SessionRegistryFence) {
    if let Some(expected_launch_id) = fence.runtime_launch_id.as_deref()
        && let Ok(mut controls) = state.codex_controls.lock()
    {
        let matches = controls
            .get(&fence.session_id)
            .is_some_and(|entry| entry.launch_id == expected_launch_id);
        if matches {
            controls.remove(&fence.session_id);
        }
    }
    state
        .attach_brokers
        .shutdown_session_if_matches(fence)
        .await;
    state
        .provider_prompt_discovery
        .evict_session_if_matches(fence)
        .await;
}

async fn update_session_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let Some(object) = body.as_object() else {
        return envelope_err(CliError::usage(
            "invalid-session-update",
            "session update body must be a JSON object",
            None,
        ));
    };
    let title_input = match object.get("title") {
        Some(Value::String(title)) => Some(Some(title.clone())),
        Some(Value::Null) => Some(None),
        Some(_) => {
            return envelope_err(CliError::usage(
                "invalid-title",
                "session title must be a string or null",
                Some(json!({ "field": "title" })),
            ));
        }
        None => None,
    };
    let title_state_input = match object.get("title_state") {
        Some(Value::Null) => Some(None),
        Some(value) => match serde_json::from_value::<SessionTitleStateInput>(value.clone()) {
            Ok(state) => Some(Some(SessionTitleState::from(state))),
            Err(_) => {
                return envelope_err(CliError::usage(
                    "invalid-title-state",
                    "title_state must be a valid structured title object or null",
                    Some(json!({ "field": "title_state" })),
                ));
            }
        },
        None => None,
    };
    let title_supplied = title_input.is_some();
    let (title, title_state) = match title_state_input {
        Some(Some(state)) => (title_input.flatten(), Some(state)),
        Some(None) => (title_input.flatten(), None),
        None => match title_input {
            Some(title) => (title, None),
            None => {
                return envelope_err(CliError::usage(
                    "missing-title",
                    "session update requires a title or title_state field",
                    Some(json!({ "field": "title" })),
                ));
            }
        },
    };
    let expected_title_revision = match object.get("expected_title_revision") {
        Some(Value::Number(revision)) => match revision.as_u64() {
            Some(revision) => Some(revision),
            None => {
                return envelope_err(CliError::usage(
                    "invalid-title-revision",
                    "expected_title_revision must be an unsigned integer",
                    Some(json!({ "field": "expected_title_revision" })),
                ));
            }
        },
        Some(_) => {
            return envelope_err(CliError::usage(
                "invalid-title-revision",
                "expected_title_revision must be an unsigned integer",
                Some(json!({ "field": "expected_title_revision" })),
            ));
        }
        None => None,
    };
    let expected_session_created_at = match object.get("expected_session_created_at") {
        Some(Value::String(created_at))
            if !created_at.trim().is_empty() && created_at.len() <= 128 =>
        {
            Some(created_at.clone())
        }
        Some(_) => {
            return envelope_err(CliError::usage(
                "invalid-session-incarnation",
                "expected_session_created_at must be a non-empty string",
                Some(json!({ "field": "expected_session_created_at" })),
            ));
        }
        None => None,
    };
    let expected_session_incarnation = match object.get("expected_session_incarnation") {
        Some(Value::String(incarnation))
            if !incarnation.trim().is_empty() && incarnation.len() <= 128 =>
        {
            Some(incarnation.clone())
        }
        Some(_) => {
            return envelope_err(CliError::usage(
                "invalid-session-incarnation",
                "expected_session_incarnation must be a non-empty string",
                Some(json!({ "field": "expected_session_incarnation" })),
            ));
        }
        None => None,
    };
    let expected_session_title = match object.get("expected_session_title") {
        // This is observed state, not a proposed title. Older clients could persist
        // raw prompt titles beyond the current normalized-title limit, and upgraded
        // writers must still be able to replace those values safely.
        Some(Value::String(title)) => Some(Some(title.clone())),
        Some(Value::Null) => Some(None),
        Some(_) => {
            return envelope_err(CliError::usage(
                "invalid-expected-session-title",
                "expected_session_title must be a string or null",
                Some(json!({ "field": "expected_session_title" })),
            ));
        }
        None => None,
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    match tokio::task::spawn_blocking(move || {
        update_session_title_if_revision(
            &context,
            &id,
            title,
            title_supplied,
            title_state,
            crate::TitleUpdatePreconditions {
                title_revision: expected_title_revision,
                session_created_at: expected_session_created_at,
                session_incarnation: expected_session_incarnation,
                session_title: expected_session_title,
            },
            &tmux,
        )
    })
    .await
    {
        Ok(Ok(session)) => {
            invalidate_managed_history_titles(&state.managed_history_titles);
            envelope_ok(json!({ "machine": state.machine, "session": session }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn upload_attachment_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    Query(query): Query<AttachmentQuery>,
    body: Body,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    if headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|value| value > state.max_attachment_bytes)
    {
        return status_json(
            StatusCode::PAYLOAD_TOO_LARGE,
            "attachment-too-large",
            "attachment exceeds the maximum allowed size",
        );
    }
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let context = state.context.clone();
    let filename = query.filename.clone();
    let pending = match tokio::task::spawn_blocking(move || {
        prepare_session_attachment(&context, &id, filename.as_deref(), content_type)
    })
    .await
    {
        Ok(Ok(pending)) => pending,
        Ok(Err(err)) => return envelope_err(err),
        Err(_) => return join_err(),
    };
    let file = match pending.reopen() {
        Ok(file) => file,
        Err(err) => return envelope_err(err),
    };
    let mut file = tokio::fs::File::from_std(file);
    let mut stream = body.into_data_stream();
    let mut bytes = 0_u64;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => {
                drop(file);
                return status_json(
                    StatusCode::BAD_REQUEST,
                    "attachment-read-failed",
                    "failed to read the attachment request body",
                );
            }
        };
        bytes = match bytes.checked_add(chunk.len() as u64) {
            Some(total) if total <= state.max_attachment_bytes => total,
            _ => {
                drop(file);
                return status_json(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "attachment-too-large",
                    "attachment exceeds the maximum allowed size",
                );
            }
        };
        if let Err(err) = file.write_all(&chunk).await {
            drop(file);
            return envelope_err(CliError::runtime(
                "file-write-failed",
                format!("failed to write the attachment temporary file: {err}"),
                None,
            ));
        }
    }
    if let Err(err) = file.sync_all().await {
        drop(file);
        return envelope_err(CliError::runtime(
            "file-write-failed",
            format!("failed to sync the attachment temporary file: {err}"),
            None,
        ));
    }
    drop(file);
    match tokio::task::spawn_blocking(move || pending.publish(bytes)).await {
        Ok(Ok(attachment)) => {
            envelope_ok(json!({ "machine": state.machine, "attachment": attachment }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn workdirs_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    Query(query): Query<WorkdirQuery>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let q = query.q.clone();
    let limit = query.limit;
    let options = WorkdirSearchOptions {
        git_only: query.git_only,
        exclude_worktrees: query.exclude_worktrees,
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        search_workdirs(&context, q.as_deref(), limit, options)
    })
    .await
    {
        Ok(Ok(workdirs)) => envelope_ok(json!({ "machine": state.machine, "workdirs": workdirs })),
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn repo_remote_url_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    Query(query): Query<RepoRemoteUrlQuery>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let Some(cwd) = query
        .cwd
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return envelope_err(CliError::usage(
            "missing-cwd",
            "missing cwd query parameter",
            None,
        ));
    };
    let cwd = cwd.to_string();
    match tokio::task::spawn_blocking(move || repo_remote_url_from_cwd(&cwd)).await {
        Ok(url) => envelope_ok(json!({ "machine": state.machine, "url": url })),
        Err(_) => join_err(),
    }
}

async fn delete_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    query: Result<Query<DeleteQuery>, QueryRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response = delete_handler_inner(State(state), headers, AxPath(id.clone()), query).await;
    crate::lifecycle::response(&context, &id, "delete", before, response).await
}

async fn delete_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    query: Result<Query<DeleteQuery>, QueryRejection>,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let Ok(Query(query)) = query else {
        return envelope_err(CliError::usage(
            "invalid-query",
            "delete accepts only orphan_children=true|false",
            None,
        ));
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let delete_id = id.clone();
    let machine = state.machine.clone();
    match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("delete");
        crate::delete_session_guarding_children(
            &context,
            &machine,
            &delete_id,
            tmux,
            query.orphan_children,
        )
    })
    .await
    {
        Ok(Ok(result)) => {
            cleanup_deleted_session_registries(&state, &result.registry_fence).await;
            envelope_ok(json!({ "machine": state.machine, "deleted": result }))
        }
        Ok(Err(err)) => envelope_err(err),
        Err(_) => join_err(),
    }
}

async fn archive_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<ArchiveBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let context = state.context.clone();
    let before = load_session_record(&context, &id).ok();
    let response = archive_handler_inner(State(state), headers, AxPath(id.clone()), body).await;
    crate::lifecycle::response(&context, &id, "archive", before, response).await
}

async fn archive_handler_inner(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<ArchiveBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Json(body) = match body {
        Ok(body) => body,
        Err(_) => {
            return envelope_err(CliError::usage(
                "invalid-json-body",
                "archive request requires expected_session_incarnation",
                None,
            ));
        }
    };
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let machine = state.machine.clone();
    let response_machine = machine.clone();
    let starred = body.starred;
    match tokio::task::spawn_blocking(move || {
        let _journal_owner = crate::lifecycle::ServeGuard::enter("archive");
        let children =
            crate::lineage::guard_children(&context, &machine, &id, body.orphan_children)?;
        let (archive, mut deleted) = archive_session_with_expected_incarnation(
            &context,
            &id,
            tmux,
            &body.expected_session_incarnation,
        )?;
        deleted.children = Some(children);
        // The session is already archived and gone; a star that cannot be stored
        // is reported as unstarred rather than failing the archive after the fact.
        let starred_at = starred
            .then(activity_observed_at)
            .and_then(|starred_at| {
                provider_history::write_star(
                    &provider_history::star_root(&context.state_dir),
                    &archive.history_id,
                    &starred_at,
                )
                .ok()
            })
            .map(|record| record.starred_at);
        Ok((archive, starred_at, deleted))
    })
    .await
    {
        Ok(Ok((archive, starred_at, deleted))) => {
            state.history_catalog.invalidate();
            cleanup_deleted_session_registries(&state, &deleted.registry_fence).await;
            let mut archived = json!(archive);
            if let Some(starred_at) = starred_at {
                archived["starred_at"] = json!(starred_at);
            }
            envelope_ok(json!({
                "machine": response_machine,
                "archived": archived,
                "deleted": deleted,
            }))
        }
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn attach_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    ws: WebSocketUpgrade,
) -> Response {
    if let Some(resp) = deny_unauthorized(&state, &headers) {
        return resp;
    }
    let context = state.context.clone();
    let tmux = state.tmux_bin.clone();
    let lookup = tokio::task::spawn_blocking({
        let context = context.clone();
        let id = id.clone();
        let tmux = tmux.clone();
        move || {
            let record = load_session_record(&context, &id)?;
            let status = session_status(&context, &tmux, &record);
            Ok::<_, CliError>((record, status))
        }
    })
    .await;
    let (record, status) = match lookup {
        Ok(Ok(result)) => result,
        Ok(Err(err)) => return envelope_err(err),
        Err(_) => return join_err(),
    };
    if status != "running" {
        return envelope_err(attach_unavailable_error(&record, &status));
    }
    ws.on_upgrade(move |socket| attach_socket(socket, state, record))
}

fn attach_unavailable_error(record: &crate::SessionRecord, status: &str) -> CliError {
    if status == "unknown" {
        return CliError::runtime(
            "session-status-unknown",
            format!("session status could not be checked: {}", record.id),
            Some(json!({ "id": record.id.clone() })),
        );
    }
    CliError::data(
        "session-not-running",
        format!("session is not running: {}", record.id),
        Some(json!({ "id": record.id.clone(), "status": status })),
    )
}

// --- websocket attach ---------------------------------------------------------

#[derive(Clone, Debug)]
enum AttachEvent {
    Output(Bytes),
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachWriterExit {
    Drained,
    SendFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttachPumpExit {
    BrokerClosed,
    Lagged,
    WriterClosed,
}

struct AttachHandoff {
    snapshot: Option<String>,
    live: VecDeque<Bytes>,
    broker_closed: bool,
}

struct AbortOnDropTask<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> AbortOnDropTask<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }

    fn handle_mut(&mut self) -> &mut tokio::task::JoinHandle<T> {
        self.handle.as_mut().expect("task handle is present")
    }

    fn abort(&mut self) {
        if let Some(handle) = &self.handle {
            handle.abort();
        }
    }

    async fn abort_and_wait(mut self) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        handle.abort();
        let _ = handle.await;
    }
}

impl<T> Drop for AbortOnDropTask<T> {
    fn drop(&mut self) {
        self.abort();
    }
}

struct AttachSubscription {
    receiver: Option<tokio::sync::broadcast::Receiver<AttachEvent>>,
    lease: Option<AttachLease>,
    resize_lock: Arc<tokio::sync::Mutex<()>>,
}

struct AttachLease {
    slot: Arc<tokio::sync::Mutex<AttachBrokerSlot>>,
    generation: u64,
    released: bool,
}

#[derive(Default)]
struct AttachBrokerRegistry {
    entries: tokio::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<AttachBrokerSlot>>>>,
}

struct ProviderPromptDiscoveryRegistry {
    entries: tokio::sync::Mutex<
        HashMap<ProviderPromptDiscoveryKey, Arc<tokio::sync::Mutex<ProviderPromptDiscoverySlot>>>,
    >,
    resolver: Arc<ProviderPromptSourceResolver>,
    scan_permits: Arc<tokio::sync::Semaphore>,
    tracker_opener: Arc<ProviderLastPromptTrackerOpener>,
    recovery_permits: Arc<tokio::sync::Semaphore>,
}

type ProviderPromptSourceResolver =
    dyn Fn(&crate::SessionRecord) -> Option<ProviderPromptSource> + Send + Sync;
type ProviderLastPromptTrackerOpener =
    dyn Fn(ProviderPromptSource) -> Option<ProviderLastPromptTracker> + Send + Sync;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LastPromptState {
    Current,
    Pending,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LastPromptProjection {
    state: LastPromptState,
    continuity: Option<String>,
    prompt: Option<LastPrompt>,
    message: Option<crate::provider_prompt::LastMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ProviderPromptDiscoveryKey {
    session_id: String,
    provider: String,
    provider_session_id: String,
    generation: u64,
    launch_id: String,
    tmux_session: String,
}

struct ProviderPromptDiscoverySlot {
    source: Option<ProviderPromptSource>,
    last_prompt_tracker: Arc<StdMutex<Option<ProviderLastPromptTracker>>>,
    last_prompt_in_flight: bool,
    last_prompt_invalidated: bool,
    last_prompt_continuity: String,
    in_flight: bool,
    progress: tokio::sync::watch::Sender<u64>,
    next_scan_at: Option<Instant>,
    backoff: Duration,
    scan_attempts: usize,
}

struct AttachBrokerSlot {
    accepting: bool,
    next_generation: u64,
    active: Option<AttachBrokerGeneration>,
}

struct AttachBrokerGeneration {
    generation: u64,
    subscribers: usize,
    registry_fence: SessionRegistryFence,
    broker: AttachBroker,
}

struct AttachBroker {
    shell_binding: Option<(CliContext, crate::SessionRecord)>,
    target: String,
    fifo_path: PathBuf,
    tmux: PathBuf,
    sender: tokio::sync::broadcast::Sender<AttachEvent>,
    reader_task: tokio::task::JoinHandle<()>,
    closed: Arc<AtomicBool>,
    resize_lock: Arc<tokio::sync::Mutex<()>>,
}

struct AttachFifoCleanup {
    path: PathBuf,
    armed: bool,
}

impl AttachFifoCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AttachFifoCleanup {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl Default for AttachBrokerSlot {
    fn default() -> Self {
        Self {
            accepting: true,
            next_generation: 1,
            active: None,
        }
    }
}

impl Default for ProviderPromptDiscoverySlot {
    fn default() -> Self {
        let (progress, _) = tokio::sync::watch::channel(0);
        Self {
            source: None,
            last_prompt_tracker: Arc::new(StdMutex::new(None)),
            last_prompt_in_flight: false,
            last_prompt_invalidated: false,
            last_prompt_continuity: uuid::Uuid::new_v4().simple().to_string(),
            in_flight: false,
            progress,
            next_scan_at: None,
            backoff: PROVIDER_PROMPT_PENDING_POLL_INTERVAL,
            scan_attempts: 0,
        }
    }
}

impl Default for ProviderPromptDiscoveryRegistry {
    fn default() -> Self {
        Self {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            resolver: Arc::new(ProviderPromptTail::resolve_source),
            scan_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS,
            )),
            tracker_opener: Arc::new(ProviderLastPromptTracker::open_source),
            recovery_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS,
            )),
        }
    }
}

impl ProviderPromptDiscoveryKey {
    fn from_record(record: &crate::SessionRecord) -> Option<Self> {
        let runtime = record.runtime.as_ref()?;
        let resume = record.provider_resume.as_ref()?;
        Some(Self {
            session_id: record.id.clone(),
            provider: resume.provider.clone(),
            provider_session_id: resume.session_id.clone(),
            generation: runtime.generation,
            launch_id: runtime.launch_id.clone(),
            tmux_session: runtime.tmux_session.clone(),
        })
    }
}

impl ProviderPromptDiscoveryRegistry {
    #[cfg(test)]
    fn with_resolver<F>(resolver: F) -> Self
    where
        F: Fn(&crate::SessionRecord) -> Option<ProviderPromptSource> + Send + Sync + 'static,
    {
        Self {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            resolver: Arc::new(resolver),
            scan_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS,
            )),
            tracker_opener: Arc::new(ProviderLastPromptTracker::open_source),
            recovery_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS,
            )),
        }
    }

    #[cfg(test)]
    fn with_resolver_and_tracker_opener<F, G>(resolver: F, tracker_opener: G) -> Self
    where
        F: Fn(&crate::SessionRecord) -> Option<ProviderPromptSource> + Send + Sync + 'static,
        G: Fn(ProviderPromptSource) -> Option<ProviderLastPromptTracker> + Send + Sync + 'static,
    {
        Self {
            entries: tokio::sync::Mutex::new(HashMap::new()),
            resolver: Arc::new(resolver),
            scan_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_DISCOVERY_MAX_CONCURRENT_SCANS,
            )),
            tracker_opener: Arc::new(tracker_opener),
            recovery_permits: Arc::new(tokio::sync::Semaphore::new(
                PROVIDER_PROMPT_RECOVERY_MAX_CONCURRENT_SCANS,
            )),
        }
    }

    async fn invalidate_source(&self, record: &crate::SessionRecord) {
        let Some(key) = ProviderPromptDiscoveryKey::from_record(record) else {
            return;
        };
        let slot = self.entries.lock().await.get(&key).cloned();
        if let Some(slot) = slot {
            let mut state = slot.lock().await;
            state.source = None;
            state.last_prompt_tracker = Arc::new(StdMutex::new(None));
            state.last_prompt_in_flight = false;
            state.last_prompt_invalidated = true;
            state.last_prompt_continuity = uuid::Uuid::new_v4().simple().to_string();
            state.next_scan_at = None;
            state.backoff = PROVIDER_PROMPT_PENDING_POLL_INTERVAL;
        }
    }

    async fn continuity_for_key(&self, key: &ProviderPromptDiscoveryKey) -> Option<String> {
        let slot = self.entries.lock().await.get(key).cloned()?;
        Some(slot.lock().await.last_prompt_continuity.clone())
    }

    async fn evict_session_if_matches(&self, fence: &SessionRegistryFence) {
        let (Some(expected_launch_id), Some(expected_generation)) =
            (fence.runtime_launch_id.as_deref(), fence.runtime_generation)
        else {
            return;
        };
        self.entries.lock().await.retain(|key, _| {
            key.session_id != fence.session_id
                || key.tmux_session != fence.tmux_session
                || key.launch_id != expected_launch_id
                || key.generation != expected_generation
        });
    }

    #[cfg(test)]
    async fn scan_attempts(&self, record: &crate::SessionRecord) -> usize {
        let Some(key) = ProviderPromptDiscoveryKey::from_record(record) else {
            return 0;
        };
        let slot = self.entries.lock().await.get(&key).cloned();
        let Some(slot) = slot else { return 0 };
        slot.lock().await.scan_attempts
    }

    #[cfg(test)]
    async fn entry_count(&self) -> usize {
        self.entries.lock().await.len()
    }

    #[cfg(test)]
    async fn contains_record(&self, record: &crate::SessionRecord) -> bool {
        let Some(key) = ProviderPromptDiscoveryKey::from_record(record) else {
            return false;
        };
        self.entries.lock().await.contains_key(&key)
    }

    #[cfg(test)]
    async fn in_flight_count(&self) -> usize {
        let slots: Vec<_> = self.entries.lock().await.values().cloned().collect();
        let mut count = 0;
        for slot in slots {
            if slot.lock().await.in_flight {
                count += 1;
            }
        }
        count
    }
}

impl AttachSubscription {
    fn receiver_mut(&mut self) -> &mut tokio::sync::broadcast::Receiver<AttachEvent> {
        self.receiver.as_mut().expect("attach receiver is present")
    }

    fn take_receiver(&mut self) -> tokio::sync::broadcast::Receiver<AttachEvent> {
        self.receiver.take().expect("attach receiver is present")
    }

    async fn release(mut self) {
        if let Some(lease) = self.lease.take() {
            lease.release().await;
        }
    }
}

impl Drop for AttachSubscription {
    fn drop(&mut self) {
        let Some(lease) = self.lease.take() else {
            return;
        };
        drop(lease);
    }
}

impl AttachLease {
    async fn release(mut self) {
        let cleanup = self.schedule_release();
        self.released = true;
        if let Some(cleanup) = cleanup {
            let _ = cleanup.await;
        }
    }

    fn schedule_release(&self) -> Option<tokio::task::JoinHandle<()>> {
        let handle = tokio::runtime::Handle::try_current().ok()?;
        let slot = self.slot.clone();
        let generation = self.generation;
        Some(handle.spawn(async move {
            let mut slot = slot.lock().await;
            let Some(active) = slot.active.as_mut() else {
                return;
            };
            if active.generation != generation {
                return;
            }
            if active.subscribers > 1 {
                active.subscribers -= 1;
                return;
            }
            if let Some(active) = slot.active.take() {
                // This supervisor task owns teardown independently of the
                // caller, so cancellation cannot strand the pipe or FIFO.
                // Keep the per-session lock so an old generation cannot close
                // a replacement pane pipe for the same session.
                active.broker.stop().await;
            }
        }))
    }
}

impl Drop for AttachLease {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        let _ = self.schedule_release();
    }
}

impl AttachBrokerRegistry {
    async fn subscribe(
        &self,
        context: &CliContext,
        tmux: &Path,
        record: &crate::SessionRecord,
    ) -> io::Result<AttachSubscription> {
        let slot = {
            let mut entries = self.entries.lock().await;
            entries
                .entry(if record.agent == "shell" {
                    format!(
                        "shell:{}",
                        record
                            .extra
                            .get("emergency_shell")
                            .and_then(|v| v.get("owner"))
                            .and_then(Value::as_str)
                            .ok_or_else(|| io::Error::other("shell owner is missing"))?
                    )
                } else {
                    record.id.clone()
                })
                .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(AttachBrokerSlot::default())))
                .clone()
        };
        let mut slot_state = slot.lock().await;
        if !slot_state.accepting {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "attach broker is shutting down",
            ));
        }

        let registry_fence = SessionRegistryFence::from_record(record);
        let target = attach_pane_target(record);
        if let Some(active) = slot_state.active.as_mut()
            && active.broker.target == target
            && active.registry_fence == registry_fence
        {
            // Subscribe before checking `closed`. If the reader exits between
            // these operations, this receiver observes its terminal event.
            let receiver = active.broker.sender.subscribe();
            if !active.broker.closed.load(Ordering::Acquire) {
                active.subscribers += 1;
                return Ok(AttachSubscription {
                    receiver: Some(receiver),
                    lease: Some(AttachLease {
                        slot: slot.clone(),
                        generation: active.generation,
                        released: false,
                    }),
                    resize_lock: active.broker.resize_lock.clone(),
                });
            }
        }

        if let Some(active) = slot_state.active.take() {
            active.broker.stop().await;
        }
        let generation = slot_state.next_generation;
        slot_state.next_generation = slot_state.next_generation.wrapping_add(1).max(1);
        let (broker, receiver) = AttachBroker::start(context, tmux, record).await?;
        let resize_lock = broker.resize_lock.clone();
        slot_state.active = Some(AttachBrokerGeneration {
            generation,
            subscribers: 1,
            registry_fence,
            broker,
        });
        Ok(AttachSubscription {
            receiver: Some(receiver),
            lease: Some(AttachLease {
                slot: slot.clone(),
                generation,
                released: false,
            }),
            resize_lock,
        })
    }

    async fn shutdown_all(&self) {
        let mut entries = self.entries.lock().await;
        let slots: Vec<_> = entries.drain().map(|(_, slot)| slot).collect();
        drop(entries);
        for slot in slots {
            let mut slot = slot.lock().await;
            slot.accepting = false;
            if let Some(active) = slot.active.take() {
                active.broker.stop().await;
            }
        }
    }

    async fn shutdown_session_if_matches(&self, fence: &SessionRegistryFence) {
        let slot = self.entries.lock().await.get(&fence.session_id).cloned();
        if let Some(slot) = slot {
            let mut slot = slot.lock().await;
            let matches = slot
                .active
                .as_ref()
                .is_some_and(|active| active.registry_fence == *fence);
            if matches && let Some(active) = slot.active.take() {
                active.broker.stop().await;
            }
        }
    }

    #[cfg(test)]
    async fn subscriber_count(&self, session_id: &str) -> usize {
        let slot = self.entries.lock().await.get(session_id).cloned();
        let Some(slot) = slot else { return 0 };
        let slot = slot.lock().await;
        slot.active.as_ref().map_or(0, |active| active.subscribers)
    }
}

impl AttachBroker {
    async fn start(
        context: &CliContext,
        tmux: &Path,
        record: &crate::SessionRecord,
    ) -> io::Result<(Self, tokio::sync::broadcast::Receiver<AttachEvent>)> {
        Self::start_with_fifo_opener(context, tmux, record, |fifo_path| {
            let fifo = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(fifo_path)?;
            unsafe { tokio::io::unix::AsyncFd::register(fifo) }.map_err(Into::into) // SAFETY: `fifo` stays valid and owned by `AsyncFd`.
        })
        .await
    }

    async fn start_with_fifo_opener<F>(
        context: &CliContext,
        tmux: &Path,
        record: &crate::SessionRecord,
        open_fifo: F,
    ) -> io::Result<(Self, tokio::sync::broadcast::Receiver<AttachEvent>)>
    where
        F: FnOnce(&Path) -> io::Result<tokio::io::unix::AsyncFd<std::fs::File>>,
    {
        let target = attach_pane_target(record);
        let fifo_path = session_dir(context, &record.id).join(ATTACH_LIVE_FIFO_NAME);
        create_private_fifo(&fifo_path)?;
        let mut fifo_cleanup = AttachFifoCleanup::new(fifo_path.clone());
        let fifo = open_fifo(&fifo_path)?;

        let (sender, receiver) = tokio::sync::broadcast::channel(ATTACH_BROADCAST_CAPACITY);
        if record.agent == "shell" {
            let context = context.clone();
            let tmux = tmux.to_path_buf();
            let record = record.clone();
            let target = target.clone();
            let fifo = fifo_path.clone();
            tokio::task::spawn_blocking(move || {
                shell::terminal_command(
                    &context,
                    &tmux,
                    &record,
                    &[
                        "pipe-pane",
                        "-t",
                        &target,
                        &format!("cat > {}", shell_words::quote(&fifo.to_string_lossy())),
                    ],
                )
                .map(|_| ())
                .map_err(|e| io::Error::other(e.message().to_string()))
            })
            .await
            .map_err(io::Error::other)??;
        } else {
            enable_tmux_pipe(tmux, &target, &fifo_path).await?;
        }

        let reader_sender = sender.clone();
        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = closed.clone();
        let reader_task = tokio::spawn(async move {
            read_attach_fifo(fifo, reader_sender, reader_closed).await;
        });
        let resize_lock = Arc::new(tokio::sync::Mutex::new(()));
        fifo_cleanup.disarm();
        Ok((
            Self {
                shell_binding: (record.agent == "shell").then(|| (context.clone(), record.clone())),
                target,
                fifo_path,
                tmux: tmux.to_path_buf(),
                sender,
                reader_task,
                closed,
                resize_lock,
            },
            receiver,
        ))
    }

    async fn stop(self) {
        if let Some((context, record)) = self.shell_binding {
            let tmux = self.tmux.clone();
            let target = self.target.clone();
            let _ = tokio::task::spawn_blocking(move || {
                shell::terminal_command(&context, &tmux, &record, &["pipe-pane", "-t", &target])
            })
            .await;
        } else {
            let _ = close_tmux_pipe(&self.tmux, &self.target).await;
        }
        let mut reader_task = self.reader_task;
        if tokio::time::timeout(ATTACH_READER_STOP_TIMEOUT, &mut reader_task)
            .await
            .is_err()
        {
            reader_task.abort();
            let _ = reader_task.await;
        }
        let _ = std::fs::remove_file(&self.fifo_path);
    }
}

fn create_private_fifo(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => std::fs::remove_file(path)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "attach FIFO path contains a NUL byte",
        )
    })?;
    // SAFETY: `path` is a valid, NUL-terminated C string and mode 0600 keeps
    // terminal bytes private to the daemon user.
    if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

async fn enable_tmux_pipe(tmux: &Path, target: &str, fifo_path: &Path) -> io::Result<()> {
    let tmux = tmux.to_path_buf();
    let target = target.to_string();
    let fifo_path = fifo_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut command = ProcessCommand::new(&tmux);
        command.arg("pipe-pane").arg("-t").arg(&target).arg(format!(
            "cat > {}",
            shell_words::quote(&fifo_path.to_string_lossy())
        ));
        let status = run_process_with_timeout(&mut command, ATTACH_TMUX_COMMAND_TIMEOUT)?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "tmux pipe-pane failed with status {status}"
            )))
        }
    })
    .await
    .map_err(|err| io::Error::other(format!("tmux pipe task failed: {err}")))?
}

async fn close_tmux_pipe(tmux: &Path, target: &str) -> io::Result<()> {
    let tmux = tmux.to_path_buf();
    let target = target.to_string();
    tokio::task::spawn_blocking(move || {
        let mut command = ProcessCommand::new(&tmux);
        command.arg("pipe-pane").arg("-t").arg(&target);
        run_process_with_timeout(&mut command, ATTACH_TMUX_COMMAND_TIMEOUT).map(|_| ())
    })
    .await
    .map_err(|err| io::Error::other(format!("tmux close task failed: {err}")))?
}

fn run_process_with_timeout(
    command: &mut ProcessCommand,
    timeout: Duration,
) -> io::Result<std::process::ExitStatus> {
    let mut child = command.spawn()?;
    wait_process_with_timeout(&mut child, timeout)
}

fn run_process_output_with_timeout(
    command: &mut ProcessCommand,
    timeout: Duration,
) -> io::Result<std::process::Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("child stdout pipe is unavailable"))?;
    let stdout_reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let status = wait_process_with_timeout(&mut child, timeout);
    let stdout = stdout_reader
        .join()
        .map_err(|_| io::Error::other("child stdout reader panicked"))??;
    Ok(std::process::Output {
        status: status?,
        stdout,
        stderr: Vec::new(),
    })
}

fn wait_process_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> io::Result<std::process::ExitStatus> {
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("process exceeded {} ms", timeout.as_millis()),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Cursor home plus erase-display, so a replayed screen lands at a known origin
/// instead of continuing from wherever the live cursor happened to be.
const ATTACH_REPLAY_ORIGIN: &str = "\x1b[H\x1b[2J";
/// Close every kind of run `-e` can leave open, so the replay neither inherits
/// state from whatever was on the wire before it nor leaks state into the
/// provider's own repaint underneath. `-J` closes NONE of them at the end of a
/// row or region — verified against tmux 3.4 — so all three matter:
///
/// - OSC 8: a capture ending inside a hyperlink would make every byte the
///   provider paints next part of that link, and a click anywhere would open
///   its target.
/// - SGR: the last captured attribute would colour the repaint.
/// - SI (shift-in): tmux encodes ACS line drawing as SO/SI, and the `-J` form
///   emits the SO without its SI. A later `enacs` from any ncurses repaint
///   would then translate ordinary text into box-drawing glyphs. SGR 0 does not
///   reset the charset glevel, so this needs its own byte.
const ATTACH_REPLAY_RESET: &str = "\x1b]8;;\x1b\\\x1b[0m\x0f";

/// Turn a `capture-pane -p -e -J` screen into bytes that are safe to inject
/// into the raw PTY stream an attaching client reads. With `-J` each row is a
/// joined logical line: it can exceed the pane width (the client re-wraps it)
/// and keeps the trailing spaces at former wrap points. `-e` adds escape
/// sequences, and the alphabet it can emit is SGR, OSC 8, and SO/SI — never
/// cursor motion, which is what keeps the staircase normalization below valid.
///
/// `capture-pane -p` joins pane rows with a bare `\n`, with or without `-e`. In a PTY stream a lone LF
/// only moves the cursor down one row and keeps its column, so replaying the
/// capture verbatim staircases every row further right until it wraps at the
/// edge — the client then repaints the same content correctly underneath, which
/// is what surfaced as a duplicated, mangled attach screen.
///
/// This is provider-agnostic on purpose: Codex, Claude Code, and DSH all
/// stream through this one capture path, and each of their TUIs repaints over
/// this baseline on its next render or on the resize the client sends right
/// after attach.
fn pty_ready_attach_replay(capture: &str) -> Option<Vec<u8>> {
    let mut rows: Vec<&str> = capture
        .split('\n')
        .map(|row| row.strip_suffix('\r').unwrap_or(row))
        .collect();
    // tmux pads the capture to the pane height; trailing blank rows would only
    // push the provider's own repaint down the screen. A padding row can carry
    // attribute or hyperlink resets, which are not whitespace, so emptiness is
    // judged on the visible text.
    while rows
        .last()
        .is_some_and(|row| attach_replay_row_is_blank(row))
    {
        rows.pop();
    }
    if rows.is_empty() {
        return None;
    }
    let mut replay = String::with_capacity(
        ATTACH_REPLAY_ORIGIN.len() + 2 * ATTACH_REPLAY_RESET.len() + capture.len() + rows.len(),
    );
    // Reset BEFORE the erase: erase-display paints with the current background
    // on a back-colour-erase terminal, and the client reuses one Terminal
    // across reconnects, so a stream that ended mid-SGR would otherwise repaint
    // the whole screen in that stale colour until the provider redraws.
    replay.push_str(ATTACH_REPLAY_RESET);
    replay.push_str(ATTACH_REPLAY_ORIGIN);
    for row in rows {
        replay.push_str(row);
        replay.push_str("\r\n");
    }
    replay.push_str(ATTACH_REPLAY_RESET);
    Some(replay.into_bytes())
}

/// Whether a captured row carries no visible text, ignoring escape sequences.
///
/// Only used to decide which trailing rows are padding. `nils_common::strip_ansi`
/// handles CSI but not OSC, and `-e` output carries OSC 8 hyperlinks, so this
/// walks both forms.
fn attach_replay_row_is_blank(row: &str) -> bool {
    let bytes = row.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != 0x1b {
            if !bytes[i].is_ascii_whitespace() {
                return false;
            }
            i += 1;
            continue;
        }
        match bytes.get(i + 1) {
            // CSI: parameters and intermediates, then a final byte.
            Some(b'[') => {
                i += 2;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1;
            }
            // OSC: terminated by BEL or ST.
            Some(b']') => {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\') {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
            }
            // Any other two-byte escape.
            Some(_) => i += 2,
            None => i += 1,
        }
    }
    true
}

async fn capture_attach_snapshot(
    tmux: &Path,
    record: &crate::SessionRecord,
) -> io::Result<Option<String>> {
    capture_attach_snapshot_with_timeout(tmux, record, ATTACH_SNAPSHOT_TIMEOUT).await
}

async fn capture_attach_snapshot_with_timeout(
    tmux: &Path,
    record: &crate::SessionRecord,
    timeout: Duration,
) -> io::Result<Option<String>> {
    let tmux = tmux.to_path_buf();
    let tmux_session = record.tmux_session.clone();
    let shell_record = (record.agent == "shell").then(|| record.clone());
    tokio::task::spawn_blocking(move || {
        if let Some(record) = shell_record {
            let context = shell::terminal_context(&record)
                .map_err(|e| io::Error::other(e.message().to_string()))?;
            return shell::terminal_command(
                &context,
                &tmux,
                &record,
                &[
                    "capture-pane",
                    "-p",
                    "-e",
                    "-J",
                    "-t",
                    &tmux_session,
                    "-S",
                    "-200",
                ],
            )
            .map(|out| Some(String::from_utf8_lossy(&out.stdout).to_string()))
            .map_err(|e| io::Error::other(e.message().to_string()));
        }
        let mut command = ProcessCommand::new(&tmux);
        // -J joins tmux-wrapped rows back into logical lines (preserving the
        // trailing spaces at each join), so the attaching client's terminal
        // re-wraps them at its own width and marks continuation rows as
        // wrapped. Replayed wrapped URLs and commands then stay one logical
        // line for client-side selection and link detection, and scrollback
        // captured at an earlier pane width re-wraps at the current width.
        // -e keeps the escape sequences tmux stored for the pane. `-p` alone
        // prints plain text, so every re-attach replayed the scrollback with no
        // attributes and no OSC 8 hyperlinks: a link whose visible label is not
        // a URL stopped existing until the provider repainted that region.
        command
            .arg("capture-pane")
            .arg("-p")
            .arg("-e")
            .arg("-J")
            .arg("-t")
            .arg(&tmux_session)
            .arg("-S")
            .arg("-200");
        let output = run_process_output_with_timeout(&mut command, timeout)?;
        if output.status.success() {
            Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|err| io::Error::other(format!("tmux capture task failed: {err}")))?
}

async fn capture_attach_handoff(
    tmux: &Path,
    record: &crate::SessionRecord,
    receiver: &mut tokio::sync::broadcast::Receiver<AttachEvent>,
) -> io::Result<AttachHandoff> {
    for recaptures in 0..=ATTACH_HANDOFF_MAX_RECAPTURES {
        let capture = capture_attach_snapshot(tmux, record);
        tokio::pin!(capture);
        let mut live = VecDeque::with_capacity(ATTACH_HANDOFF_BUFFER_CAPACITY);
        let mut overflowed = false;
        let mut broker_closed = false;

        let snapshot = loop {
            tokio::select! {
                snapshot = &mut capture => break snapshot?,
                event = receiver.recv(), if !broker_closed => {
                    match event {
                        Ok(AttachEvent::Output(bytes)) => {
                            if !overflowed && live.len() < ATTACH_HANDOFF_BUFFER_CAPACITY {
                                live.push_back(bytes);
                            } else {
                                // Continue draining so output generated by the
                                // daemon's own snapshot handoff cannot make this
                                // client appear slow. A fresh snapshot below
                                // becomes the new terminal-state baseline.
                                overflowed = true;
                                live.clear();
                            }
                        }
                        Ok(AttachEvent::Closed)
                        | Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            broker_closed = true;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            overflowed = true;
                            live.clear();
                        }
                    }
                }
            }
        };

        if !overflowed {
            return Ok(AttachHandoff {
                snapshot,
                live,
                broker_closed,
            });
        }
        if recaptures == ATTACH_HANDOFF_MAX_RECAPTURES {
            return Err(io::Error::other(
                "attach snapshot handoff overflowed after bounded recapture",
            ));
        }
    }
    unreachable!("bounded snapshot handoff loop returns")
}

async fn pump_attach_events(
    mut receiver: tokio::sync::broadcast::Receiver<AttachEvent>,
    terminal_tx: mpsc::Sender<Message>,
) -> AttachPumpExit {
    loop {
        match receiver.recv().await {
            Ok(AttachEvent::Output(bytes)) => {
                if terminal_tx.send(Message::Binary(bytes)).await.is_err() {
                    return AttachPumpExit::WriterClosed;
                }
            }
            Ok(AttachEvent::Closed) | Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return AttachPumpExit::BrokerClosed;
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                return AttachPumpExit::Lagged;
            }
        }
    }
}

async fn read_attach_fifo(
    fifo: tokio::io::unix::AsyncFd<std::fs::File>,
    sender: tokio::sync::broadcast::Sender<AttachEvent>,
    closed: Arc<AtomicBool>,
) {
    let mut buf = vec![0u8; 8192];
    let started_at = Instant::now();
    let mut writer_seen = false;
    loop {
        let read = match tokio::time::timeout(ATTACH_FIFO_POLL_INTERVAL, fifo.readable()).await {
            Ok(Ok(mut ready)) => match ready.try_io(|inner| {
                let mut file = inner.get_ref();
                file.read(&mut buf)
            }) {
                Ok(read) => read,
                Err(_would_block) => continue,
            },
            Ok(Err(err)) => Err(err),
            Err(_) => {
                // Some kqueue implementations do not emit another readable
                // notification when a FIFO's final writer disappears. The
                // descriptor is nonblocking, so a low-frequency direct read
                // distinguishes an idle writer (WouldBlock) from EOF without
                // tying up the async runtime.
                let mut file = fifo.get_ref();
                file.read(&mut buf)
            }
        };
        match read {
            Ok(0) if !writer_seen && started_at.elapsed() < ATTACH_PIPE_STARTUP_GRACE => {
                // `pipe-pane` starts its shell asynchronously. A nonblocking
                // FIFO reader briefly sees EOF until that writer opens.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            Ok(0) => break,
            Ok(n) => {
                writer_seen = true;
                let _ = sender.send(AttachEvent::Output(Bytes::copy_from_slice(&buf[..n])));
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => break,
        }
    }
    closed.store(true, Ordering::Release);
    let _ = sender.send(AttachEvent::Closed);
}

#[cfg(test)]
fn attach_event_bytes(
    event: Result<AttachEvent, tokio::sync::broadcast::error::RecvError>,
) -> Option<Bytes> {
    match event {
        Ok(AttachEvent::Output(bytes)) => Some(bytes),
        Ok(AttachEvent::Closed)
        | Err(tokio::sync::broadcast::error::RecvError::Closed)
        | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => None,
    }
}

async fn send_attach_message<S>(sender: &mut S, message: Message, timeout: Duration) -> bool
where
    S: futures_util::Sink<Message> + Unpin,
{
    matches!(
        tokio::time::timeout(timeout, sender.send(message)).await,
        Ok(Ok(()))
    )
}

fn attach_pane_target(record: &crate::SessionRecord) -> String {
    if record.agent == "shell" {
        record.tmux_session.clone()
    } else {
        format!("{}:0.0", record.tmux_session)
    }
}

async fn attach_socket(socket: WebSocket, state: Arc<ServeState>, record: crate::SessionRecord) {
    attach_socket_inner(socket, state, record, None).await;
}

async fn attach_socket_inner(
    socket: WebSocket,
    state: Arc<ServeState>,
    mut record: crate::SessionRecord,
    shell_record: Option<shell::ShellRecord>,
) {
    let context = if shell_record.is_some() {
        shell::context(&state)
    } else {
        state.context.clone()
    };
    let target = attach_pane_target(&record);
    let (sender, mut receiver) = socket.split();
    let (terminal_tx, terminal_rx) = mpsc::channel(ATTACH_TERMINAL_QUEUE_CAPACITY);
    let (control_tx, control_rx) = mpsc::channel(ATTACH_CONTROL_QUEUE_CAPACITY);
    let mut writer_task = Some(AbortOnDropTask::new(tokio::spawn(outbound_writer(
        sender,
        terminal_rx,
        control_rx,
        ATTACH_WEBSOCKET_SEND_TIMEOUT,
    ))));

    // Subscribe before capturing the initial screen so pane output produced
    // during capture is buffered instead of silently lost. Snapshot and live
    // bytes may overlap; preserving every byte is preferable to a gap.
    let mut subscription = match state
        .attach_brokers
        .subscribe(&context, &state.tmux_bin, &record)
        .await
    {
        Ok(subscription) => subscription,
        Err(err) => {
            eprintln!(
                "warning: failed to start live attach broker for {}: {err}",
                record.id
            );
            return;
        }
    };
    let handoff =
        match capture_attach_handoff(&state.tmux_bin, &record, subscription.receiver_mut()).await {
            Ok(handoff) => handoff,
            Err(err) => {
                eprintln!(
                    "warning: failed to establish attach snapshot handoff for {}: {err}",
                    record.id
                );
                subscription.release().await;
                if let Some(task) = writer_task.take() {
                    task.abort_and_wait().await;
                }
                return;
            }
        };
    if let Some(replay) = handoff
        .snapshot
        .as_deref()
        .and_then(pty_ready_attach_replay)
        && terminal_tx
            .send(Message::Binary(replay.into()))
            .await
            .is_err()
    {
        subscription.release().await;
        if let Some(task) = writer_task.take() {
            task.abort_and_wait().await;
        }
        return;
    }
    for bytes in handoff.live {
        if terminal_tx.send(Message::Binary(bytes)).await.is_err() {
            subscription.release().await;
            if let Some(task) = writer_task.take() {
                task.abort_and_wait().await;
            }
            return;
        }
    }

    let mut pump_task = (!handoff.broker_closed).then(|| {
        AbortOnDropTask::new(tokio::spawn(pump_attach_events(
            subscription.take_receiver(),
            terminal_tx.clone(),
        )))
    });

    // Client -> pane: JSON control frames { text?, key?, keys?, resize{cols,rows} }.
    // The first resize after attach forces a full-frame repaint (see resize_pane).
    let mut initial_repaint_pending = true;
    let resize_lock = subscription.resize_lock.clone();
    let mut provider_prompt_task: Option<AbortOnDropTask<()>> = None;
    let mut provider_open_task: Option<
        AbortOnDropTask<(crate::SessionRecord, Option<ProviderPromptTail>)>,
    > = None;
    let mut provider_refresh_pending = false;
    let mut provider_pending_deadline: Option<Instant> = None;
    let mut drain_writer = handoff.broker_closed;
    while pump_task.is_some() {
        tokio::select! {
            writer_result = async {
                writer_task
                    .as_mut()
                    .expect("writer task select guard")
                    .handle_mut()
                    .await
            }, if writer_task.is_some() => {
                writer_task.take();
                if matches!(writer_result, Ok(AttachWriterExit::SendFailed) | Err(_)) {
                    eprintln!(
                        "warning: outbound attach writer stopped for {}",
                        record.id
                    );
                }
                break;
            }
            pump_result = async {
                pump_task
                    .as_mut()
                    .expect("attach pump select guard")
                    .handle_mut()
                    .await
            }, if pump_task.is_some() => {
                pump_task.take();
                drain_writer = matches!(pump_result, Ok(AttachPumpExit::BrokerClosed));
                break;
            }
            opened = async {
                provider_open_task
                    .as_mut()
                    .expect("provider open task select guard")
                    .handle_mut()
                    .await
            }, if provider_open_task.is_some() => {
                provider_open_task.take();
                let prompt_tail = match opened {
                    Ok((updated_record, tail)) => {
                        record = updated_record;
                        tail
                    }
                    Err(_) => None,
                };
                let capability = ProviderPromptCapabilityState {
                    supported: prompt_tail.is_some(),
                    provider: prompt_tail.as_ref().map(ProviderPromptTail::provider),
                };
                if control_tx
                    .send(Message::Text(
                        provider_prompt_capability_frame(capability).into(),
                    ))
                    .await
                    .is_err()
                {
                    break;
                }

                if provider_refresh_pending && prompt_tail.is_none() {
                    provider_refresh_pending = false;
                    provider_open_task = Some(open_provider_prompt_task(
                        state.context.clone(),
                        state.provider_prompt_discovery.clone(),
                        record.clone(),
                        *provider_pending_deadline
                            .get_or_insert_with(|| Instant::now() + PROVIDER_PROMPT_PENDING_TIMEOUT),
                    ));
                } else if let Some(tail) = prompt_tail {
                    provider_refresh_pending = false;
                    provider_prompt_task = Some(AbortOnDropTask::new(tokio::spawn(
                        provider_prompt_loop(tail, control_tx.clone()),
                    )));
                }
            }
            message = receiver.next() => {
                let Some(Ok(message)) = message else { break; };
                match message {
                    Message::Text(text) => {
                        if provider_prompt_subscription_requested(text.as_str()) {
                            if shell_record.is_some() {
                                let capability = ProviderPromptCapabilityState { supported:false, provider:None };
                                if control_tx.send(Message::Text(provider_prompt_capability_frame(capability).into())).await.is_err() { break; }
                                continue;
                            }
                            if let Some(mut task) = provider_prompt_task.take() {
                                task.abort();
                            }
                            if provider_open_task.is_some() {
                                // Preserve the old serial refresh behavior without
                                // pausing broker consumption or launching overlapping
                                // transcript scans for repeated subscriptions.
                                provider_refresh_pending = true;
                            } else {
                                provider_open_task = Some(open_provider_prompt_task(
                                    state.context.clone(),
                                    state.provider_prompt_discovery.clone(),
                                    record.clone(),
                                    *provider_pending_deadline.get_or_insert_with(|| {
                                        Instant::now() + PROVIDER_PROMPT_PENDING_TIMEOUT
                                    }),
                                ));
                            }
                            continue;
                        }
                        if let Some(shell) = &shell_record {
                            if shell::input(state.clone(), shell.clone(), text.to_string()).await.is_err() { break; }
                            continue;
                        }
                        if let Err(err) = handle_input(
                            &state.context,
                            &state.tmux_bin,
                            &record,
                            &target,
                            text.as_str(),
                            &mut initial_repaint_pending,
                            &resize_lock,
                        )
                        .await
                        {
                            eprintln!(
                                "warning: terminal input failed for {}: {}",
                                record.id,
                                err.code()
                            );
                            if !terminal_input_error_is_recoverable(&err) {
                                break;
                            }
                            if control_tx
                                .send(Message::Text(terminal_input_rejected_frame(&err).into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    }

    if let Some(task) = pump_task {
        task.abort_and_wait().await;
    }
    if let Some(task) = provider_prompt_task {
        task.abort_and_wait().await;
    }
    if let Some(task) = provider_open_task {
        task.abort_and_wait().await;
    }
    subscription.release().await;
    drop(terminal_tx);
    drop(control_tx);
    if let Some(task) = writer_task {
        finish_outbound_writer(task, drain_writer).await;
    }
}

fn open_provider_prompt_task(
    context: CliContext,
    discovery: Arc<ProviderPromptDiscoveryRegistry>,
    record: crate::SessionRecord,
    pending_deadline: Instant,
) -> AbortOnDropTask<(crate::SessionRecord, Option<ProviderPromptTail>)> {
    AbortOnDropTask::new(tokio::spawn(resolve_provider_prompt_tail(
        context,
        discovery,
        record,
        pending_deadline,
    )))
}

async fn resolve_provider_prompt_tail(
    context: CliContext,
    discovery: Arc<ProviderPromptDiscoveryRegistry>,
    record: crate::SessionRecord,
    pending_deadline: Instant,
) -> (crate::SessionRecord, Option<ProviderPromptTail>) {
    let pending_fresh_runtime = provider_prompt_pending_fresh_runtime(&record);
    let initial_source = discovery.resolve_source(&record).await;
    let had_initial_source = initial_source.is_some();
    let mut opened = if let Some(source) = initial_source {
        tokio::task::spawn_blocking(move || ProviderPromptTail::open_source_at_eof(source))
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    if had_initial_source && opened.is_none() {
        discovery.invalidate_source(&record).await;
        if let Some(source) = discovery.resolve_source(&record).await {
            opened =
                tokio::task::spawn_blocking(move || ProviderPromptTail::open_source_at_eof(source))
                    .await
                    .ok()
                    .flatten();
        }
    }
    if opened.is_some() || !pending_fresh_runtime {
        return (record, opened);
    }

    let mut current = record;
    while Instant::now() < pending_deadline {
        tokio::time::sleep(PROVIDER_PROMPT_PENDING_POLL_INTERVAL).await;
        let load_context = context.clone();
        let id = current.id.clone();
        let loaded =
            tokio::task::spawn_blocking(move || load_session_record(&load_context, &id)).await;
        let Ok(Ok(updated)) = loaded else {
            continue;
        };
        if !same_provider_prompt_runtime(&current, &updated) {
            return (updated, None);
        }
        current = updated;
        if !provider_prompt_new_runtime_source_authorized(&current) {
            continue;
        }
        let source = discovery.resolve_source(&current).await;
        let had_source = source.is_some();
        let tail = if let Some(source) = source {
            tokio::task::spawn_blocking(move || ProviderPromptTail::open_new_runtime_source(source))
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        if tail.is_some() {
            return (current, tail);
        }
        if had_source {
            discovery.invalidate_source(&current).await;
        }
    }
    (current, None)
}

fn provider_prompt_new_runtime_source_authorized(record: &crate::SessionRecord) -> bool {
    let Some(resume) = record.provider_resume.as_ref() else {
        return false;
    };
    match AgentKind::from_name(&record.agent) {
        Some(AgentKind::Codex) => {
            resume.provider == "codex"
                && matches!(
                    resume.capture_method.as_str(),
                    "codex-user-prompt-submit-hook"
                        | crate::codex_app_server::PROVIDER_RESUME_CAPTURE_METHOD
                )
        }
        Some(AgentKind::Claude) => {
            resume.provider == "claude" && resume.capture_method == "claude-explicit-session-id"
        }
        Some(AgentKind::Dsh) | None => false,
    }
}

fn provider_prompt_pending_fresh_runtime(record: &crate::SessionRecord) -> bool {
    if record
        .runtime
        .as_ref()
        .is_none_or(|runtime| runtime.generation != 1)
    {
        return false;
    }
    match AgentKind::from_name(&record.agent) {
        Some(AgentKind::Codex) => record.provider_resume.is_none(),
        Some(AgentKind::Claude) => record.provider_resume.as_ref().is_some_and(|resume| {
            resume.provider == "claude"
                && resume.capture_method == "claude-explicit-session-id"
                && !resume.session_id.trim().is_empty()
        }),
        Some(AgentKind::Dsh) | None => false,
    }
}

fn same_provider_prompt_runtime(
    expected: &crate::SessionRecord,
    current: &crate::SessionRecord,
) -> bool {
    match (&expected.runtime, &current.runtime) {
        (Some(expected), Some(current)) => {
            expected.generation == current.generation
                && expected.launch_id == current.launch_id
                && expected.tmux_session == current.tmux_session
        }
        _ => false,
    }
}

async fn finish_outbound_writer(mut writer_task: AbortOnDropTask<AttachWriterExit>, drain: bool) {
    if drain
        && tokio::time::timeout(ATTACH_WEBSOCKET_SEND_TIMEOUT, writer_task.handle_mut())
            .await
            .is_ok()
    {
        return;
    }
    writer_task.abort_and_wait().await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderPromptCapabilityState {
    supported: bool,
    provider: Option<ProviderKind>,
}

fn provider_prompt_subscription_requested(frame: &str) -> bool {
    serde_json::from_str::<Value>(frame)
        .ok()
        .and_then(|value| value.get("subscribe").and_then(Value::as_array).cloned())
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|capability| capability.as_str() == Some(PROVIDER_PROMPT_CAPABILITY))
        })
}

fn provider_prompt_capability_frame(capability: ProviderPromptCapabilityState) -> String {
    json!({
        "schema_version": ATTACH_SCHEMA_VERSION,
        "type": "capability",
        "capability": PROVIDER_PROMPT_CAPABILITY,
        "supported": capability.supported,
        "provider": capability.provider.map(ProviderKind::as_str),
        "prompt_max_bytes": MAX_PROVIDER_PROMPT_BYTES,
    })
    .to_string()
}

fn terminal_input_rejected_frame(error: &CliError) -> String {
    json!({
        "schema_version": ATTACH_EVENT_SCHEMA_VERSION,
        "type": "input_rejected",
        "code": error.code(),
        "retryable": true,
    })
    .to_string()
}

fn terminal_input_error_is_recoverable(error: &CliError) -> bool {
    matches!(
        error.code(),
        "codex-account-next-pending"
            | "codex-account-not-bound"
            | "codex-input-section-unavailable"
            | "codex-input-section-lease-failed"
            | "session-record-lock-busy"
            | "session-record-lock-timeout"
    )
}

fn provider_prompt_event_frame(provider: ProviderKind, event: ProviderPromptEvent) -> String {
    json!({
        "schema_version": ATTACH_EVENT_SCHEMA_VERSION,
        "type": "prompt_submitted",
        "event_id": event.id,
        "provider": provider.as_str(),
        "submitted_at": event.submitted_at,
        "text": event.prompt,
        "truncated": event.truncated,
    })
    .to_string()
}

async fn provider_prompt_loop(mut tail: ProviderPromptTail, sender: mpsc::Sender<Message>) {
    let provider = tail.provider();
    let mut interval = tokio::time::interval(PROVIDER_PROMPT_POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = sender.closed() => break,
            _ = interval.tick() => {}
        }
        let polled = tokio::task::spawn_blocking(move || {
            let result = tail.poll();
            (tail, result)
        })
        .await;
        let Ok((returned_tail, result)) = polled else {
            break;
        };
        tail = returned_tail;
        let Ok(events) = result else {
            break;
        };
        for event in events {
            let frame = Message::Text(provider_prompt_event_frame(provider, event).into());
            match sender.try_send(frame) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => {
                    // Prompt events are advisory. Queue saturation must never
                    // apply backpressure to terminal output.
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
}

async fn outbound_writer<S>(
    mut sender: S,
    mut terminal_rx: mpsc::Receiver<Message>,
    mut control_rx: mpsc::Receiver<Message>,
    send_timeout: Duration,
) -> AttachWriterExit
where
    S: futures_util::Sink<Message> + Unpin,
{
    let mut terminal_burst = 0;
    while let Some(message) =
        next_outbound_message(&mut terminal_rx, &mut control_rx, &mut terminal_burst).await
    {
        if !send_attach_message(&mut sender, message, send_timeout).await {
            return AttachWriterExit::SendFailed;
        }
    }
    AttachWriterExit::Drained
}

async fn next_outbound_message(
    terminal_rx: &mut mpsc::Receiver<Message>,
    control_rx: &mut mpsc::Receiver<Message>,
    terminal_burst: &mut usize,
) -> Option<Message> {
    if *terminal_burst >= ATTACH_TERMINAL_BURST
        && let Ok(message) = control_rx.try_recv()
    {
        *terminal_burst = 0;
        return Some(message);
    }
    if let Ok(message) = terminal_rx.try_recv() {
        *terminal_burst += 1;
        return Some(message);
    }
    if let Ok(message) = control_rx.try_recv() {
        *terminal_burst = 0;
        return Some(message);
    }
    tokio::select! {
        biased;
        message = terminal_rx.recv() => {
            match message {
                Some(message) => {
                    *terminal_burst += 1;
                    Some(message)
                }
                None => {
                    let message = control_rx.recv().await;
                    if message.is_some() {
                        *terminal_burst = 0;
                    }
                    message
                },
            }
        }
        message = control_rx.recv() => {
            match message {
                Some(message) => {
                    *terminal_burst = 0;
                    Some(message)
                }
                None => {
                    let message = terminal_rx.recv().await;
                    if message.is_some() {
                        *terminal_burst += 1;
                    }
                    message
                },
            }
        }
    }
}

/// After a fresh (re)attach the client rebuilds its terminal emulator from
/// scratch and sends its real size as the first resize frame. A `resize-window`
/// to dimensions the tmux pane already has is a no-op — no SIGWINCH — so a
/// full-screen agent (codex/claude/dsh) never repaints, and the client is
/// left rendering the stale pre-attach snapshot mis-wrapped against its new
/// grid. Force exactly one guaranteed size change on that first resize so the
/// agent redraws its whole frame into the fresh grid. The short pause lets the
/// agent observe the intermediate size, so the two changes are not coalesced
/// into "no net change" (which would skip the redraw).
const INITIAL_REPAINT_NUDGE_DELAY: Duration = Duration::from_millis(150);

/// Apply a client resize to the pane. On the first resize after attach
/// (`force_repaint`), nudge to an off-by-one height first so the final resize to
/// `rows` is always a real change that repaints the agent's frame; afterwards a
/// single `resize-window` is issued so ordinary mid-session resizes don't flicker.
async fn resize_pane(tmux: &Path, target: &str, cols: u64, rows: u64, force_repaint: bool) {
    resize_pane_with_timeout(
        tmux,
        target,
        cols,
        rows,
        force_repaint,
        ATTACH_RESIZE_TIMEOUT,
    )
    .await;
}

async fn resize_pane_with_timeout(
    tmux: &Path,
    target: &str,
    cols: u64,
    rows: u64,
    force_repaint: bool,
    timeout: Duration,
) {
    if force_repaint {
        let nudge_rows = if rows > 1 { rows - 1 } else { rows + 1 };
        let _ = run_resize_window(tmux, target, cols, nudge_rows, timeout).await;
        tokio::time::sleep(INITIAL_REPAINT_NUDGE_DELAY).await;
    }
    let _ = run_resize_window(tmux, target, cols, rows, timeout).await;
}

async fn run_resize_window(
    tmux: &Path,
    target: &str,
    cols: u64,
    rows: u64,
    timeout: Duration,
) -> io::Result<()> {
    let tmux = tmux.to_path_buf();
    let target = target.to_string();
    tokio::task::spawn_blocking(move || {
        let mut command = ProcessCommand::new(&tmux);
        command
            .arg("resize-window")
            .arg("-t")
            .arg(&target)
            .arg("-x")
            .arg(cols.to_string())
            .arg("-y")
            .arg(rows.to_string());
        run_process_with_timeout(&mut command, timeout).map(|_| ())
    })
    .await
    .map_err(|err| io::Error::other(format!("tmux resize task failed: {err}")))?
}

async fn handle_input(
    context: &CliContext,
    tmux: &Path,
    record: &crate::SessionRecord,
    target: &str,
    frame: &str,
    initial_repaint_pending: &mut bool,
    resize_lock: &tokio::sync::Mutex<()>,
) -> Result<(), CliError> {
    let Ok(value) = serde_json::from_str::<Value>(frame) else {
        return Ok(());
    };

    if let Some(resize) = value.get("resize") {
        let cols = resize.get("cols").and_then(Value::as_u64);
        let rows = resize.get("rows").and_then(Value::as_u64);
        if let (Some(cols), Some(rows)) = (cols, rows) {
            let force_repaint = std::mem::take(initial_repaint_pending);
            // Concurrent attach clients share one tmux pane. Keep each resize
            // sequence atomic; the last completed client resize wins.
            let _guard = resize_lock.lock().await;
            resize_pane(tmux, target, cols, rows, force_repaint).await;
        }
        return Ok(());
    }

    let mut text = value
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut keys: Vec<SpecialKey> = value
        .get("keys")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(SpecialKey::from_name)
                .collect()
        })
        .unwrap_or_default();
    if let Some(key) = value
        .get("key")
        .and_then(Value::as_str)
        .and_then(SpecialKey::from_name)
    {
        keys.push(key);
    }
    // Older clients classify modified arrows as text. An exact key frame must
    // pass through tmux's key encoder, which honors the pane's keyboard mode;
    // pasting its escape bytes prints them literally in enhanced TUIs.
    // Embedded and bracketed-paste sequences remain literal text.
    if text.as_deref() == Some("\u{1b}[1;2D") {
        text = None;
        keys.insert(0, SpecialKey::ShiftLeft);
    }
    if text.is_none() && keys.is_empty() {
        return Ok(());
    }

    let context = context.clone();
    let tmux = tmux.to_path_buf();
    let record = record.clone();
    let id = record.id.clone();
    tokio::task::spawn_blocking(move || {
        send_input_serialized(&context, &record, text.as_deref(), &keys, &tmux)
    })
    .await
    .map_err(|_| {
        CliError::runtime(
            "attach-input-worker-failed",
            "the terminal input worker failed",
            Some(json!({ "id": id })),
        )
    })?
}

fn remote_session_token(headers: &HeaderMap) -> Result<String, CliError> {
    headers
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .ok_or_else(crate::coordination::unauthorized)
}
async fn remote_submit_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<crate::coordination::remote::Submit>, JsonRejection>,
) -> Response {
    let body = match coordination_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let Some(config) = state.federation.clone() else {
        return envelope_err(CliError::runtime(
            "remote-messaging-unavailable",
            "federation is disabled",
            None,
        ));
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::remote::submit(&context, &config, &id, &token, body)
    })
    .await
    {
        Ok(Ok(value)) => {
            state.remote_outbox_wake.notify_one();
            Json(value).into_response()
        }
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}
async fn mail_audit_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    query: Result<
        Query<crate::coordination::audit::Query>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let query = match query {
        Ok(Query(query)) => query,
        Err(_) => {
            return envelope_err(CliError::usage(
                "mail-audit-query-invalid",
                "mail audit query is invalid",
                None,
            ));
        }
    };
    let context = state.context.clone();
    let machine = state.machine.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::audit::snapshot(&context, &machine, query)
    })
    .await
    {
        Ok(Ok(value)) => envelope_ok(value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn remote_peers_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
) -> Response {
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let Some(config) = state.federation.clone() else {
        return envelope_err(CliError::runtime(
            "remote-messaging-unavailable",
            "federation is disabled",
            None,
        ));
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::remote::peers(&context, &config, &id, &token)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}
async fn remote_delivery_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, message)): AxPath<(String, String)>,
) -> Response {
    let token = match remote_session_token(&headers) {
        Ok(v) => v,
        Err(e) => return envelope_err(e),
    };
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::remote::delivery(&context, &id, &token, &message)
    })
    .await
    {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}
async fn remote_receive_handler(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    body: Result<
        Json<crate::coordination::remote::Envelope<crate::coordination::remote::Origin>>,
        JsonRejection,
    >,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Some(config) = state.federation.as_ref() else {
        return envelope_err(CliError::runtime(
            "remote-messaging-unavailable",
            "federation is disabled",
            None,
        ));
    };
    if !headers
        .get("X-Agent-Session-Relay-Token")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| constant_time_eq(v, &config.ingress_token))
    {
        return envelope_err(crate::coordination::unauthorized());
    }
    let body = match coordination_json(body) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let context = state.context.clone();
    let machine = state.machine.clone();
    match tokio::task::spawn_blocking(move || {
        crate::coordination::remote::receive(&context, &machine, body)
    })
    .await
    {
        Ok(Ok(value)) => {
            state.coordination_notification_wake.notify_one();
            Json(value).into_response()
        }
        Ok(Err(e)) => envelope_err(e),
        Err(_) => join_err(),
    }
}
#[cfg(test)]
mod tests;

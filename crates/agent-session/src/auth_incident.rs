//! Runtime-fenced provider authentication evidence and owner notification outbox.
//! Only fixed metadata enters this store; provider messages and terminal bytes
//! are inspected in memory and never persisted or sent to the owner.

use std::fs;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::activity::{Confidence, SourceKind, TurnEvent, TurnEventKind};
use crate::{CliContext, CliError, SessionRecord, session_dir};

const FILE: &str = "auth-incidents.json";
const SCHEMA: &str = "agent-session.auth-incidents.v1";
const MAX_INCIDENTS: usize = 16;
const MAX_STORE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AuthSource {
    ClaudeStopFailure,
    CodexAppServer,
    CodexExternalRefresh,
    TerminalPattern,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Notification {
    #[serde(default)]
    submission_started_at: Option<String>,
    delivered_at: Option<String>,
    message_id: Option<String>,
    degraded_reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthIncident {
    pub incident_id: String,
    pub session_id: String,
    pub runtime_incarnation: String,
    pub runtime_generation: u64,
    pub provider: String,
    pub account_nickname: Option<String>,
    pub observed_at: String,
    pub source: AuthSource,
    pub confidence: Confidence,
    pub status: String,
    pub recovery_result: Option<String>,
    pub recovery_observed_at: Option<String>,
    notification: Notification,
    recovery_notification: Notification,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema_version: String,
    incidents: Vec<AuthIncident>,
    /// Digest of the latest accepted failure. No raw event identity or bytes.
    last_failure_key: Option<String>,
    detection_health: Option<String>,
    #[serde(default)]
    terminal_auth_runtime: Option<String>,
}

fn error() -> CliError {
    CliError::runtime(
        "auth-incident-store-unavailable",
        "provider authentication evidence is unavailable",
        None,
    )
}

fn read(context: &CliContext, record: &SessionRecord) -> Result<Store, CliError> {
    let path = session_dir(context, &record.id).join(FILE);
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Store {
                schema_version: SCHEMA.into(),
                ..Store::default()
            });
        }
        Err(_) => return Err(error()),
    };
    if bytes.len() > MAX_STORE_BYTES {
        return Err(error());
    }
    let store: Store = serde_json::from_slice(&bytes).map_err(|_| error())?;
    if store.schema_version != SCHEMA || store.incidents.len() > MAX_INCIDENTS {
        return Err(error());
    }
    Ok(store)
}

fn save(context: &CliContext, record: &SessionRecord, store: &Store) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(store).map_err(|_| error())?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(error());
    }
    write_atomic(
        &session_dir(context, &record.id).join(FILE),
        &bytes,
        SECRET_FILE_MODE,
    )
    .map_err(|_| error())
}

fn key(runtime: &str, event: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(runtime.as_bytes());
    hash.update([0]);
    hash.update(event.as_bytes());
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn same_runtime(incident: &AuthIncident, record: &SessionRecord) -> bool {
    record.runtime.as_ref().is_some_and(|runtime| {
        incident.runtime_incarnation == runtime.launch_id
            && incident.runtime_generation == runtime.generation
    })
}

fn runtime_matches(left: &SessionRecord, right: &SessionRecord) -> bool {
    match (left.runtime.as_ref(), right.runtime.as_ref()) {
        (Some(left), Some(right)) => {
            left.launch_id == right.launch_id && left.generation == right.generation
        }
        _ => false,
    }
}

pub(crate) fn projection(
    context: &CliContext,
    record: &SessionRecord,
) -> (Option<AuthIncident>, Option<String>) {
    match read(context, record) {
        Ok(store) => (
            store
                .incidents
                .into_iter()
                .rev()
                .find(|incident| same_runtime(incident, record)),
            store.detection_health,
        ),
        Err(_) => (None, Some("degraded_store_unavailable".into())),
    }
}
#[cfg(test)]
pub(crate) fn view(context: &CliContext, record: &SessionRecord) -> Option<AuthIncident> {
    projection(context, record).0
}
#[cfg(test)]
pub(crate) fn detection_health(context: &CliContext, record: &SessionRecord) -> Option<String> {
    projection(context, record).1
}

fn observe_locked(
    store: &mut Store,
    record: &SessionRecord,
    event: &str,
    source: AuthSource,
    confidence: Confidence,
    observed_at: &str,
) {
    let Some(runtime) = record.runtime.as_ref() else {
        return;
    };
    let event_key = key(&runtime.launch_id, event);
    if store.last_failure_key.as_ref() == Some(&event_key) {
        return;
    }
    if store
        .incidents
        .last()
        .is_some_and(|incident| same_runtime(incident, record) && incident.status == "auth_failed")
    {
        store.last_failure_key = Some(event_key);
        return;
    }
    store.detection_health = None;
    if store.incidents.len() == MAX_INCIDENTS {
        let settled = store.incidents.iter().position(|incident| {
            incident.status != "auth_failed"
                && incident.notification.delivered_at.is_some()
                && incident.recovery_result.is_some()
                && incident.recovery_notification.delivered_at.is_some()
        });
        let Some(index) = settled else {
            store.detection_health = Some("degraded_queue_capacity".into());
            return;
        };
        store.incidents.remove(index);
    }
    store.last_failure_key = Some(event_key.clone());
    let account_nickname = if record.agent == "codex" {
        crate::codex_account::selected_account(record)
    } else {
        crate::claude_account::view_for_record(record).and_then(|view| view.selected_account)
    };
    store.incidents.push(AuthIncident {
        incident_id: event_key,
        session_id: record.id.clone(),
        runtime_incarnation: runtime.launch_id.clone(),
        runtime_generation: runtime.generation,
        provider: record.agent.clone(),
        account_nickname,
        observed_at: observed_at.into(),
        source,
        confidence,
        status: "auth_failed".into(),
        recovery_result: None,
        recovery_observed_at: None,
        notification: Notification::default(),
        recovery_notification: Notification::default(),
    });
}

pub(crate) fn observe(
    context: &CliContext,
    record: &SessionRecord,
    event: &str,
    source: AuthSource,
    confidence: Confidence,
    observed_at: &str,
) -> Result<(), CliError> {
    let _lock =
        crate::acquire_session_record_lock_timed(context, &record.id, Duration::from_millis(250))?;
    let current = crate::load_session_record(context, &record.id)?;
    crate::ensure_same_session_identity(record, &current)?;
    if !runtime_matches(&current, record) {
        return Err(CliError::data(
            "runtime-id-mismatch",
            "authentication observation belongs to a replaced runtime",
            None,
        ));
    }
    let mut store = read(context, &current)?;
    let before = serde_json::to_vec(&store).map_err(|_| error())?;
    observe_locked(&mut store, &current, event, source, confidence, observed_at);
    if store.detection_health.as_deref() != Some("degraded_queue_capacity") {
        store.detection_health = Some(
            if source == AuthSource::TerminalPattern {
                "degraded_terminal_fallback"
            } else {
                "structured"
            }
            .into(),
        );
    }
    if serde_json::to_vec(&store).map_err(|_| error())? != before {
        save(context, &current, &store)?;
    }
    Ok(())
}

pub(crate) fn observe_activity(
    context: &CliContext,
    record: &SessionRecord,
    event: &TurnEvent,
    state: &crate::activity::TurnState,
) -> Result<(), CliError> {
    let Some(last) = state.last_turn.as_ref() else {
        return Ok(());
    };
    if event.provider_turn_id.is_some() && event.provider_turn_id != last.provider_turn_id {
        return Ok(());
    }
    if event.kind == TurnEventKind::TurnFailed
        && event.failure_reason.as_deref() == Some("authentication")
        && last.provider_failure_kind() == Some("authentication")
        && last.outcome == "failed"
    {
        let source = match event.provider.as_str() {
            "claude" => AuthSource::ClaudeStopFailure,
            "codex" => AuthSource::CodexAppServer,
            _ => return Ok(()),
        };
        observe(
            context,
            record,
            &format!("activity-auth:{}", last.completed_at),
            source,
            event.confidence.clone(),
            &last.completed_at,
        )
    } else if event.kind == TurnEventKind::TurnCompleted
        && event.source_kind == SourceKind::ProviderHook
        && last.outcome == "completed"
    {
        recover(context, record, "healthy", &last.completed_at)
    } else {
        Ok(())
    }
}

pub(crate) fn recover(
    context: &CliContext,
    record: &SessionRecord,
    result: &str,
    at: &str,
) -> Result<(), CliError> {
    let _lock =
        crate::acquire_session_record_lock_timed(context, &record.id, Duration::from_millis(250))?;
    let current = crate::load_session_record(context, &record.id)?;
    crate::ensure_same_session_identity(record, &current)?;
    if !runtime_matches(&current, record) {
        return Ok(());
    }
    let mut store = read(context, &current)?;
    if let Some(incident) = store.incidents.last_mut().filter(|incident| {
        same_runtime(incident, &current)
            && incident.status == "auth_failed"
            && at
                .parse::<jiff::Timestamp>()
                .ok()
                .zip(incident.observed_at.parse::<jiff::Timestamp>().ok())
                .is_some_and(|(at, observed)| at >= observed)
    }) {
        incident.status = if matches!(result, "healthy" | "credentials_refreshed") {
            "recovered"
        } else {
            "recovery_failed"
        }
        .into();
        incident.recovery_result = Some(result.into());
        incident.recovery_observed_at = Some(at.into());
        save(context, &current, &store)?;
    }
    Ok(())
}

/// Inspect only the bottom status line (or the line immediately above an empty
/// provider prompt). A quoted error in normal output or scrollback is not a hit.
fn terminal_auth_pattern(provider: &str, text: &str) -> bool {
    let mut lines = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let Some(mut line) = lines.next() else {
        return false;
    };
    if matches!(line, "❯" | "›" | ">") {
        line = lines.next().unwrap_or_default();
    }
    match provider {
        "claude" => matches!(
            line.strip_prefix("⎿  ")
                .or_else(|| line.strip_prefix("API Error: "))
                .unwrap_or_default(),
            "OAuth token revoked · Please run /login"
                | "Invalid authentication credentials · Please run /login"
        ),
        "codex" => {
            let error = line.strip_prefix("■ ").unwrap_or_default();
            error == "unexpected status 401 Unauthorized"
                || error.starts_with("unexpected status 401 Unauthorized:")
        }
        _ => false,
    }
}

fn sample(context: &CliContext, record: &SessionRecord, tmux: &Path) -> Result<(), CliError> {
    let state = crate::activity::state_for_view(context, record);
    // Reconcile the durable activity transaction after a crash between its
    // commit and incident persistence. It uses the same key as event ingress.
    if let Some(last) = state.as_ref().and_then(|state| state.last_turn.as_ref())
        && last.provider_failure_kind() == Some("authentication")
        && last.outcome == "failed"
    {
        let source = if record.agent == "claude" {
            AuthSource::ClaudeStopFailure
        } else {
            AuthSource::CodexAppServer
        };
        observe(
            context,
            record,
            &format!("activity-auth:{}", last.completed_at),
            source,
            Confidence::Authoritative,
            &last.completed_at,
        )?;
        return Ok(());
    }
    if let Some(state) = state.as_ref()
        && state.source.kind == SourceKind::ProviderHook
        && let Some(last) = state.last_turn.as_ref()
        && last.outcome == "completed"
        && read(context, record)?
            .incidents
            .last()
            .is_some_and(|incident| {
                same_runtime(incident, record) && incident.status == "auth_failed"
            })
    {
        recover(context, record, "healthy", &last.completed_at)?;
    }
    if state.as_ref().is_some_and(|state| {
        state.source.kind == SourceKind::ProviderHook
            && matches!(
                state.phase,
                crate::activity::TurnPhase::Waiting | crate::activity::TurnPhase::NeedsInput
            )
    }) {
        return Ok(());
    }
    if state.as_ref().is_some_and(|state| {
        state.source.kind == SourceKind::ProviderHook
            && state
                .semantic_event
                .as_ref()
                .and_then(|event| event.observed_at.parse::<jiff::Timestamp>().ok())
                .is_some_and(|observed| {
                    jiff::Timestamp::now()
                        .as_second()
                        .saturating_sub(observed.as_second())
                        < 30
                })
    }) {
        return Ok(());
    }
    let mut command = Command::new(tmux);
    command.args([
        "capture-pane",
        "-p",
        "-t",
        &crate::managed_tmux_pane_target(&record.tmux_session),
        "-S",
        "-8",
    ]);
    let output =
        crate::run_output_with_timeout_and_strict_cap(command, Duration::from_millis(250), 4096);
    let _lock =
        crate::acquire_session_record_lock_timed(context, &record.id, Duration::from_millis(250))?;
    let current = crate::load_session_record(context, &record.id)?;
    if !runtime_matches(&current, record) {
        return Ok(());
    }
    let mut store = read(context, &current)?;
    let before = serde_json::to_vec(&store).map_err(|_| error())?;
    match output {
        Ok(output) if output.status.success() => {
            let text = String::from_utf8_lossy(&output.stdout);
            let runtime = current.runtime.as_ref().expect("runtime fence");
            if terminal_auth_pattern(&current.agent, &text) {
                if store.terminal_auth_runtime.as_deref() != Some(runtime.launch_id.as_str()) {
                    let event = format!("terminal-auth-status:{}", jiff::Timestamp::now());
                    observe_locked(
                        &mut store,
                        &current,
                        &event,
                        AuthSource::TerminalPattern,
                        Confidence::Inferred,
                        &jiff::Timestamp::now().to_string(),
                    );
                    store.terminal_auth_runtime = Some(runtime.launch_id.clone());
                }
            } else {
                // Re-arm fallback only after the old status line disappears.
                store.terminal_auth_runtime = None;
            }
            if store.detection_health.as_deref() != Some("degraded_queue_capacity") {
                store.detection_health = Some("degraded_terminal_fallback".into());
            }
        }
        _ => store.detection_health = Some("degraded_source_unavailable".into()),
    }
    if serde_json::to_vec(&store).map_err(|_| error())? != before {
        save(context, &current, &store)?;
    }
    Ok(())
}

/// Send a stable body/key through the existing authenticated mailbox. An
/// unknown response keeps the exact same request for the next watchdog tick.
fn send(
    context: &CliContext,
    record: &SessionRecord,
    incident: &AuthIncident,
    recovery: bool,
    notification: &Notification,
    local_machine: &str,
) -> Result<Value, CliError> {
    let owner = crate::lineage::effective_parent(record).ok_or_else(|| {
        CliError::runtime(
            "auth-owner-unavailable",
            "provider authentication incident has no owner route",
            None,
        )
    })?;
    let idempotency_key = format!(
        "auth:{}:{}",
        incident.incident_id,
        if recovery { "recovery" } else { "observed" }
    );
    let result = if recovery {
        incident.recovery_result.as_deref().unwrap_or("unknown")
    } else {
        "authentication"
    };
    let body = serde_json::json!({"kind":if recovery {"provider_auth_recovery_result"} else {"provider_auth_incident"}, "incident_id":incident.incident_id, "session_id":incident.session_id, "runtime_incarnation":incident.runtime_incarnation, "provider":incident.provider, "account_nickname":incident.account_nickname, "observed_at":if recovery {incident.recovery_observed_at.as_ref()} else {Some(&incident.observed_at)}, "source":incident.source, "confidence":incident.confidence, "result":result}).to_string();
    let local_digest = crate::coordination::request_digest(
        "message-send",
        &serde_json::json!({
            "sender":record.id, "recipient":owner.session_id,
            "body_digest":crate::coordination::digest_bytes(body.as_bytes()),
            "reply_to":null, "expiry_secs":crate::coordination::mailbox::parse_expiry(None)?,
            "if_revision":null, "category":crate::cli::MessageCategory::Report,
        }),
    );
    let remote_request = crate::coordination::remote::Submit {
        to_machine: owner.machine.clone(),
        to_session: owner.session_id.clone(),
        body: body.clone(),
        idempotency_key: idempotency_key.clone(),
        reply_to: None,
        expires_in: None,
        reply_revision: None,
        expected_recipient_incarnation: None,
        category: Some(crate::cli::MessageCategory::Report),
        forward: None,
    };
    let remote_digest = crate::coordination::digest_bytes(
        &serde_json::to_vec(&remote_request).map_err(|_| error())?,
    );
    // New submissions still require the current runtime's normal capability.
    // Historical receipts must match the exact owner, body and category.
    if let Some(receipt) = crate::coordination::auth_incident_receipt(
        context,
        &record.id,
        &idempotency_key,
        &local_digest,
    )? {
        return Ok(receipt);
    }
    if let Some(receipt) = crate::coordination::remote::auth_incident_receipt(
        context,
        &record.id,
        &idempotency_key,
        &remote_digest,
    )? {
        return Ok(receipt);
    }
    // A crash after commit can outlive mailbox journal retention. Fail closed
    // instead of duplicating an owner notification after its receipt expires.
    if !retry_window_open(notification, jiff::Timestamp::now()) {
        return Err(CliError::runtime(
            "auth-delivery-unknown",
            "authentication notification delivery cannot be safely retried",
            None,
        ));
    }
    let runtime = record.runtime.as_ref().ok_or_else(error)?;
    let capability = crate::coordination::capability_path(context, &record.id, &runtime.launch_id);
    if let Some(message_id) = notification.message_id.as_ref() {
        return crate::coordination::remote::cli_delivery(
            context,
            crate::cli::MessageDeliveryArgs {
                session: record.id.clone(),
                message: message_id.clone(),
                capability_file: Some(capability),
                format: nils_common::cli_contract::OutputFormat::Json,
            },
        );
    }
    let body_file = session_dir(context, &record.id).join(if recovery {
        "auth-recovery-message.json"
    } else {
        "auth-incident-message.json"
    });
    write_atomic(&body_file, body.as_bytes(), SECRET_FILE_MODE).map_err(|_| error())?;
    crate::coordination::mailbox::send(
        context,
        crate::cli::MessageSendArgs {
            from_session: record.id.clone(),
            to_session: owner.session_id.clone(),
            to_machine: (owner.machine != local_machine).then(|| owner.machine.clone()),
            body_file,
            capability_file: Some(capability),
            idempotency_key,
            reply_to: None,
            expires_in: None,
            format: nils_common::cli_contract::OutputFormat::Json,
            category: Some(crate::cli::MessageCategory::Report),
        },
    )
}

fn retry_window_open(notification: &Notification, now: jiff::Timestamp) -> bool {
    notification
        .submission_started_at
        .as_deref()
        .is_none_or(|started| {
            started.parse::<jiff::Timestamp>().is_ok_and(|started| {
                now.as_second().saturating_sub(started.as_second()) < 23 * 60 * 60
            })
        })
}

fn retire_replaced(store: &mut Store, record: &SessionRecord) {
    for incident in &mut store.incidents {
        if !same_runtime(incident, record) && incident.status == "auth_failed" {
            incident.status = "runtime_replaced".into();
            incident.recovery_result = Some("runtime_replaced".into());
            incident.recovery_observed_at = Some(jiff::Timestamp::now().to_string());
        }
    }
}

fn deliver_with<F>(store: &mut Store, mut submit: F)
where
    F: FnMut(&AuthIncident, bool, &Notification) -> Result<Value, CliError>,
{
    for incident in &mut store.incidents {
        for recovery in [false, true] {
            if recovery && incident.recovery_result.is_none() {
                continue;
            }
            let notification = if recovery {
                &incident.recovery_notification
            } else {
                &incident.notification
            };
            if notification.delivered_at.is_some() {
                continue;
            }
            let response = submit(incident, recovery, notification);
            let notification = if recovery {
                &mut incident.recovery_notification
            } else {
                &mut incident.notification
            };
            match response {
                Ok(value) => {
                    notification.message_id = value
                        .get("message_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if matches!(
                        value.get("state").and_then(Value::as_str),
                        Some("delivered" | "unread" | "read" | "acknowledged")
                    ) {
                        notification.delivered_at = Some(jiff::Timestamp::now().to_string());
                        notification.degraded_reason = None;
                    } else {
                        notification.degraded_reason = Some("relay_pending".into());
                    }
                }
                Err(err) => {
                    // Missing owner routing proves no submission was attempted.
                    // Identity conflicts instead prove an earlier committed key.
                    if err.code() == "auth-owner-unavailable" {
                        notification.submission_started_at = None;
                    }
                    notification.degraded_reason = Some(
                        if err.code() == "auth-owner-unavailable" {
                            "owner_unavailable"
                        } else if err.code() == "auth-delivery-unknown" {
                            "delivery_unknown"
                        } else if err.code() == "idempotency-key-conflict" {
                            "identity_conflict"
                        } else {
                            "relay_unavailable"
                        }
                        .into(),
                    )
                }
            }
        }
    }
}

pub(crate) fn tick(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    local_machine: &str,
    terminal_available: Option<bool>,
) -> Result<(), CliError> {
    if terminal_available == Some(true) {
        sample(context, record, tmux)?;
    } else if terminal_available.is_none() {
        let _lock = crate::acquire_session_record_lock_timed(
            context,
            &record.id,
            Duration::from_millis(250),
        )?;
        let current = crate::load_session_record(context, &record.id)?;
        if runtime_matches(&current, record) {
            let mut store = read(context, &current)?;
            if store.detection_health.as_deref() != Some("degraded_source_unavailable") {
                store.detection_health = Some("degraded_source_unavailable".into());
                save(context, &current, &store)?;
            }
        }
    }
    let discovered = read(context, record)?;
    if !discovered.incidents.iter().any(|incident| {
        (!same_runtime(incident, record) && incident.status == "auth_failed")
            || incident.notification.delivered_at.is_none()
            || (incident.recovery_result.is_some()
                && incident.recovery_notification.delivered_at.is_none())
    }) {
        return Ok(());
    }
    let mut pending = {
        let _lock = crate::acquire_session_record_lock_timed(
            context,
            &record.id,
            Duration::from_millis(250),
        )?;
        let current = crate::load_session_record(context, &record.id)?;
        if !runtime_matches(&current, record) {
            return Ok(());
        }
        let mut store = read(context, &current)?;
        let before = serde_json::to_vec(&store).map_err(|_| error())?;
        retire_replaced(&mut store, &current);
        for incident in &mut store.incidents {
            for recovery in [false, true] {
                if recovery && incident.recovery_result.is_none() {
                    continue;
                }
                let notification = if recovery {
                    &mut incident.recovery_notification
                } else {
                    &mut incident.notification
                };
                if notification.delivered_at.is_none()
                    && notification.submission_started_at.is_none()
                {
                    notification.submission_started_at = Some(jiff::Timestamp::now().to_string());
                }
            }
        }
        if serde_json::to_vec(&store).map_err(|_| error())? != before {
            save(context, &current, &store)?;
        }
        Store {
            schema_version: SCHEMA.into(),
            incidents: store.incidents,
            ..Store::default()
        }
    };
    let pending_before = serde_json::to_vec(&pending).map_err(|_| error())?;
    // The mailbox owns lifecycle locks and runtime revalidation. Never hold
    // the record lock over a mailbox call or a relay request.
    deliver_with(&mut pending, |incident, recovery, notification| {
        send(
            context,
            record,
            incident,
            recovery,
            notification,
            local_machine,
        )
    });
    if serde_json::to_vec(&pending).map_err(|_| error())? == pending_before {
        return Ok(());
    }
    let _lock =
        crate::acquire_session_record_lock_timed(context, &record.id, Duration::from_millis(250))?;
    let current = crate::load_session_record(context, &record.id)?;
    if !runtime_matches(&current, record) {
        return Ok(());
    }
    let mut store = read(context, &current)?;
    let before = serde_json::to_vec(&store).map_err(|_| error())?;
    for updated in pending.incidents {
        if let Some(original) = store
            .incidents
            .iter_mut()
            .find(|incident| incident.incident_id == updated.incident_id)
        {
            original.notification = updated.notification;
            original.recovery_notification = updated.recovery_notification;
        }
    }
    if serde_json::to_vec(&store).map_err(|_| error())? != before {
        save(context, &current, &store)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt;

    fn record() -> SessionRecord {
        serde_json::from_value(json!({
            "schema_version":"agent-session.session.v1", "id":"auth-fixture", "agent":"claude", "mode":"interactive", "title":null,
            "cwd":"/fixture/repository", "tmux_session":"auth-fixture", "prompt_file":null, "log_file":null,
            "created_at":"2030-01-01T00:00:00Z", "updated_at":"2030-01-01T00:00:00Z",
            "runtime":{"kind":"tmux", "tmux_session":"auth-fixture", "generation":1, "started_at":"2030-01-01T00:00:00Z", "launch_id":"runtime-a"}
        })).unwrap()
    }

    #[test]
    fn auth_loss_runtime_replacement_does_not_strand_owner_outbox() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record();
        fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        observe(
            &context,
            &record,
            "event",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        let mut replacement = record.clone();
        replacement.runtime.as_mut().unwrap().launch_id = "runtime-b".into();
        replacement.runtime.as_mut().unwrap().generation = 2;
        crate::write_session_record(&context, &replacement).unwrap();
        tick(
            &context,
            &replacement,
            Path::new("unused"),
            "fixture",
            Some(false),
        )
        .unwrap();
        let store = read(&context, &replacement).unwrap();
        assert_eq!(
            store.incidents[0].notification.degraded_reason.as_deref(),
            Some("owner_unavailable")
        );
        assert!(view(&context, &replacement).is_none());
    }

    #[test]
    fn auth_loss_retention_preserves_all_pending_owner_obligations() {
        let record = record();
        let mut store = Store {
            schema_version: SCHEMA.into(),
            ..Store::default()
        };
        for index in 0..MAX_INCIDENTS {
            observe_locked(
                &mut store,
                &record,
                &format!("event-{index}"),
                AuthSource::ClaudeStopFailure,
                Confidence::Authoritative,
                "2030-01-01T00:00:00Z",
            );
            let incident = store.incidents.last_mut().unwrap();
            incident.status = "recovered".into();
            incident.recovery_result = Some("healthy".into());
        }
        let accepted: Vec<_> = store
            .incidents
            .iter()
            .map(|i| i.incident_id.clone())
            .collect();
        observe_locked(
            &mut store,
            &record,
            "overflow",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:01Z",
        );
        assert_eq!(
            store
                .incidents
                .iter()
                .map(|i| i.incident_id.clone())
                .collect::<Vec<_>>(),
            accepted
        );
        assert_eq!(
            store.detection_health.as_deref(),
            Some("degraded_queue_capacity")
        );
        let mut delivered = Vec::new();
        deliver_with(&mut store, |incident, recovery, _| {
            delivered.push((incident.incident_id.clone(), recovery));
            Ok(json!({"message_id":"fixture", "state":"delivered"}))
        });
        assert_eq!(delivered.len(), MAX_INCIDENTS * 2);
        deliver_with(&mut store, |_, _, _| {
            panic!("settled obligations must not repeat")
        });
        observe_locked(
            &mut store,
            &record,
            "overflow",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:01Z",
        );
        assert_eq!(store.incidents.len(), MAX_INCIDENTS);
        assert_eq!(store.incidents.last().unwrap().status, "auth_failed");
    }

    #[test]
    fn auth_loss_lost_send_response_reconciles_receipt_across_runtime_replacement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record();
        let receipt = json!({"message_id":"original", "state":"unread"});
        {
            let mut locked = crate::coordination::lock_registry(&context).unwrap();
            crate::coordination::store_receipt(
                &mut locked.registry,
                "auth:fixture:observed".into(),
                record.id.clone(),
                "runtime-a".into(),
                "message-send".into(),
                "digest".into(),
                receipt.clone(),
                crate::coordination::now_epoch(),
            )
            .unwrap();
            locked.save().unwrap();
        }
        assert_eq!(
            crate::coordination::auth_incident_receipt(
                &context,
                &record.id,
                "auth:fixture:observed",
                "digest"
            )
            .unwrap(),
            Some(receipt)
        );
        assert_eq!(
            crate::coordination::auth_incident_receipt(
                &context,
                &record.id,
                "auth:fixture:observed",
                "different-body-or-owner"
            )
            .unwrap_err()
            .code(),
            "idempotency-key-conflict"
        );
        assert_eq!(
            crate::coordination::auth_incident_receipt(
                &context,
                "different-session",
                "auth:fixture:observed",
                "digest"
            )
            .unwrap(),
            None
        );
        assert_eq!(
            crate::coordination::auth_incident_receipt(
                &context,
                &record.id,
                "auth:fixture:recovery",
                "digest"
            )
            .unwrap(),
            None
        );
    }

    #[test]
    fn auth_loss_old_runtime_history_settles_without_pinning_capacity() {
        let mut record = record();
        let mut store = Store {
            schema_version: SCHEMA.into(),
            ..Store::default()
        };
        for generation in 1..=MAX_INCIDENTS as u64 + 1 {
            let runtime = record.runtime.as_mut().unwrap();
            runtime.generation = generation;
            runtime.launch_id = format!("runtime-{generation}");
            retire_replaced(&mut store, &record);
            deliver_with(&mut store, |_, _, _| {
                Ok(json!({"message_id":"fixture", "state":"delivered"}))
            });
            observe_locked(
                &mut store,
                &record,
                "failure",
                AuthSource::ClaudeStopFailure,
                Confidence::Authoritative,
                "2030-01-01T00:00:00Z",
            );
            assert_eq!(
                store.incidents.last().unwrap().runtime_generation,
                generation
            );
            assert!(store.incidents.len() <= MAX_INCIDENTS);
        }
    }

    #[test]
    fn auth_loss_known_preflight_failure_does_not_expire_first_owner_delivery() {
        let record = record();
        let mut store = Store {
            schema_version: SCHEMA.into(),
            ..Store::default()
        };
        observe_locked(
            &mut store,
            &record,
            "failure",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:00Z",
        );
        store.incidents[0].notification.submission_started_at = Some("2030-01-01T00:00:00Z".into());
        deliver_with(&mut store, |_, _, _| {
            Err(CliError::runtime(
                "auth-owner-unavailable",
                "no owner route",
                None,
            ))
        });
        assert!(retry_window_open(
            &store.incidents[0].notification,
            "2030-01-03T00:00:00Z".parse().unwrap()
        ));
        let mut sends = 0;
        deliver_with(&mut store, |_, _, _| {
            sends += 1;
            Ok(json!({"message_id":"owner", "state":"delivered"}))
        });
        deliver_with(&mut store, |_, _, _| {
            panic!("owner notice must stay deduped")
        });
        assert_eq!(sends, 1);
    }

    #[test]
    fn auth_loss_identity_conflict_preserves_uncertain_commit_fence() {
        let record = record();
        let mut store = Store {
            schema_version: SCHEMA.into(),
            ..Store::default()
        };
        observe_locked(
            &mut store,
            &record,
            "failure",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:00Z",
        );
        store.incidents[0].notification.submission_started_at = Some("2030-01-01T00:00:00Z".into());
        deliver_with(&mut store, |_, _, _| {
            Err(CliError::data(
                "idempotency-key-conflict",
                "historical owner differs",
                None,
            ))
        });
        assert_eq!(
            store.incidents[0].notification.degraded_reason.as_deref(),
            Some("identity_conflict")
        );
        assert!(!retry_window_open(
            &store.incidents[0].notification,
            "2030-01-03T00:00:00Z".parse().unwrap()
        ));
    }

    #[test]
    fn auth_loss_unknown_delivery_fails_closed_after_journal_retention() {
        let notification = Notification {
            submission_started_at: Some("2030-01-01T00:00:00Z".into()),
            ..Notification::default()
        };
        assert!(retry_window_open(
            &notification,
            "2030-01-01T22:59:59Z".parse().unwrap()
        ));
        assert!(!retry_window_open(
            &notification,
            "2030-01-03T00:00:00Z".parse().unwrap()
        ));
    }

    #[test]
    fn auth_loss_duplicate_observation_does_not_replace_store() {
        use std::os::unix::fs::MetadataExt;
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record();
        fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        observe(
            &context,
            &record,
            "event",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        let path = session_dir(&context, &record.id).join(FILE);
        let inode = fs::metadata(&path).unwrap().ino();
        observe(
            &context,
            &record,
            "event",
            AuthSource::ClaudeStopFailure,
            Confidence::Authoritative,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    }

    #[test]
    fn auth_loss_retry_dedupes_across_restart_and_emits_one_final_result() {
        let record = record();
        let mut store = Store {
            schema_version: SCHEMA.into(),
            ..Store::default()
        };
        for event in ["event-a", "event-a", "event-b"] {
            observe_locked(
                &mut store,
                &record,
                event,
                AuthSource::ClaudeStopFailure,
                Confidence::Authoritative,
                "2030-01-01T00:00:00Z",
            );
        }
        assert_eq!(store.incidents.len(), 1);
        let mut attempts = 0;
        deliver_with(&mut store, |_, _, _| {
            attempts += 1;
            Err(error())
        });
        assert_eq!(
            store.incidents[0].notification.degraded_reason.as_deref(),
            Some("relay_unavailable")
        );
        let durable = serde_json::to_vec(&store).unwrap();
        let mut store: Store = serde_json::from_slice(&durable).unwrap();
        let incident_id = store.incidents[0].incident_id.clone();
        deliver_with(&mut store, |incident, recovery, _| {
            attempts += 1;
            assert_eq!(incident.incident_id, incident_id);
            assert!(!recovery);
            Ok(json!({"message_id":"message-a", "state":"queued"}))
        });
        assert_eq!(
            store.incidents[0].notification.degraded_reason.as_deref(),
            Some("relay_pending")
        );
        deliver_with(&mut store, |_, _, notification| {
            attempts += 1;
            assert_eq!(notification.message_id.as_deref(), Some("message-a"));
            Ok(json!({"message_id":"message-a", "state":"delivered"}))
        });
        store.incidents[0].status = "recovered".into();
        store.incidents[0].recovery_result = Some("healthy".into());
        deliver_with(&mut store, |_, recovery, _| {
            attempts += 1;
            assert!(recovery);
            Ok(json!({"message_id":"message-b", "state":"delivered"}))
        });
        deliver_with(&mut store, |_, _, _| {
            panic!("delivered incidents must not send again")
        });
        assert_eq!(attempts, 4);
        assert!(
            store.incidents[0]
                .recovery_notification
                .delivered_at
                .is_some()
        );
    }

    #[test]
    fn auth_loss_terminal_patterns_are_bounded_and_exclude_mentions_and_mcp() {
        for (provider, text, expected) in [
            (
                "claude",
                "⎿  OAuth token revoked · Please run /login\n❯",
                true,
            ),
            (
                "claude",
                "OAuth token revoked · Please run /login\n❯",
                false,
            ),
            ("codex", "unexpected status 401 Unauthorized\n›", false),
            (
                "claude",
                "The report says OAuth token revoked · Please run /login",
                false,
            ),
            (
                "claude",
                "MCP login failed: OAuth token revoked · Please run /login",
                false,
            ),
            (
                "claude",
                "OAuth token revoked · Please run /login\nnormal output\n❯",
                false,
            ),
            ("codex", "■ unexpected status 401 Unauthorized\n›", true),
            ("codex", "unexpected status 403 Forbidden\n›", false),
            (
                "codex",
                "A document mentions unexpected status 401 Unauthorized",
                false,
            ),
            (
                "codex",
                "MCP server: unexpected status 401 Unauthorized",
                false,
            ),
        ] {
            assert_eq!(terminal_auth_pattern(provider, text), expected, "{text}");
        }
    }

    #[test]
    fn auth_loss_missing_hook_fallback_persists_no_terminal_text_and_fences_runtime() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let record = record();
        fs::create_dir_all(session_dir(&context, &record.id)).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        let tmux = tmp.path().join("fake-tmux");
        fs::write(&tmux, "#!/bin/sh\nprintf 'private-terminal-canary\\n⎿  OAuth token revoked · Please run /login\\n❯\\n'\n").unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        for _ in 0..3 {
            sample(&context, &record, &tmux).unwrap();
        }
        let incident = view(&context, &record).unwrap();
        assert_eq!(incident.confidence, Confidence::Inferred);
        assert_eq!(incident.source, AuthSource::TerminalPattern);
        assert_eq!(read(&context, &record).unwrap().incidents.len(), 1);
        let text = fs::read_to_string(session_dir(&context, &record.id).join(FILE)).unwrap();
        assert!(!text.contains("private-terminal-canary"));
        assert!(!text.contains("Please run /login"));
        recover(
            &context,
            &record,
            "healthy",
            &jiff::Timestamp::now().to_string(),
        )
        .unwrap();
        sample(&context, &record, &tmux).unwrap();
        assert_eq!(
            read(&context, &record).unwrap().incidents.len(),
            1,
            "persistent screen must stay deduped after recovery"
        );
        fs::write(&tmux, "#!/bin/sh\nprintf '❯\\n'\n").unwrap();
        sample(&context, &record, &tmux).unwrap();
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf '⎿  OAuth token revoked · Please run /login\\n❯\\n'\n",
        )
        .unwrap();
        sample(&context, &record, &tmux).unwrap();
        assert_eq!(
            read(&context, &record).unwrap().incidents.len(),
            2,
            "new fallback episode after clear must be visible"
        );
        let mut replacement = record.clone();
        replacement.runtime.as_mut().unwrap().launch_id = "runtime-b".into();
        replacement.runtime.as_mut().unwrap().generation = 2;
        crate::write_session_record(&context, &replacement).unwrap();
        assert!(view(&context, &replacement).is_none());
        assert_eq!(
            observe(
                &context,
                &record,
                "stale",
                AuthSource::ClaudeStopFailure,
                Confidence::Authoritative,
                "2030-01-01T00:00:01Z"
            )
            .unwrap_err()
            .code(),
            "session-runtime-changed"
        );
        assert!(recover(&context, &record, "healthy", "2030-01-01T00:00:02Z").is_err());
        assert_eq!(
            read(&context, &replacement).unwrap().incidents[1].status,
            "auth_failed"
        );
    }
}

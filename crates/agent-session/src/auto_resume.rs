use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use jiff::Timestamp;
use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    CliContext, CliError, SessionRecord, acquire_session_record_lock,
    acquire_session_record_lock_timed, load_session_record, session_dir,
    try_acquire_session_record_lock,
};

const AUTO_RESUME_SCHEMA_VERSION: &str = "agent-session.auto-resume.v1";
const AUTO_RESUME_FILE: &str = "auto-resume.json";
const MAX_TRANSIENT_ATTEMPTS: u32 = 5;
const RETRY_DELAYS_SECONDS: [i64; 5] = [30, 60, 120, 300, 600];
// Claude can report an authoritative session rate limit while its usage helper
// exposes percentage windows without reset timestamps. Back off from five
// minutes to at most one probe per hour until the provider accepts a
// continuation or the user/session state cancels the claim.
const UNKNOWN_RESET_PROBE_DELAYS_SECONDS: [i64; 4] = [300, 900, 1_800, 3_600];
const PROTOCOL_STATE_LOCK_TIMEOUT: Duration = Duration::from_secs(1);
pub(crate) const WAIT_FOR_RESET_POLICY: &str = "wait_for_reset";
pub(crate) const NEXT_ACCOUNT_THEN_RESUME_POLICY: &str = "next_account_then_resume";

const USAGE_EXHAUSTION_CAUSE: &str = "usage_exhaustion";
const PROVIDER_CAPACITY_CAUSE: &str = "provider_capacity";

pub(crate) const CONTINUATION_MESSAGE: &str = "Continue the interrupted task from where you stopped. First inspect the current session and repository state, then continue toward the existing objective. Do not repeat completed work.";
pub(crate) const CAPACITY_CONTINUATION_MESSAGE: &str = "The selected model was at capacity, interrupting the previous turn. Please continue from where you stopped.";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct DurableAutoResume {
    schema_version: String,
    enabled: bool,
    #[serde(default = "default_recovery_policy")]
    recovery_policy: String,
    state: String,
    updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scheduled_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    next_check_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery_cause: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_account_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_binding_state: Option<String>,
    #[serde(default)]
    attempt: u32,
    #[serde(default)]
    ever_scheduled: bool,
    // Number of consecutive fallback probes scheduled while the session was
    // armed by an authoritative provider rate-limit but no usage window
    // reported `used_percent >= 100`. Preserved across re-arms so an unknown
    // reset does not probe more often than the bounded cadence above.
    #[serde(default)]
    fallback_schedules: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attempted_accounts: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct AutoResumeView {
    pub(crate) schema_version: &'static str,
    pub(crate) supported: bool,
    pub(crate) enabled: bool,
    pub(crate) recovery_policy: String,
    pub(crate) state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) scheduled_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) failure_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UsageSnapshot {
    pub(crate) authoritative: bool,
    pub(crate) has_exhausted_windows: bool,
    pub(crate) exhausted_reset_epochs: Vec<i64>,
    // Soonest reset epoch across ALL present usage windows (exhausted or not).
    // Used only to schedule a single fallback wake when a session was armed by
    // an authoritative provider rate-limit signal but no usage window reports
    // `used_percent >= 100` — the Claude "session limit" case, where the
    // provider throttles a session while its percentage windows stay below
    // 100% and no structured reset time is exposed by the stop hook. `None`
    // disables the fallback and keeps the historical fail-closed behavior.
    pub(crate) soonest_reset_epoch: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PendingSessions {
    pub(crate) recovery_ids: Vec<String>,
    pub(crate) capacity_ids: Vec<String>,
    pub(crate) usage_ids: Vec<String>,
    pub(crate) error_codes: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TickOutcome {
    Unchanged,
    Scheduled,
    Resumed,
    Retrying,
    TerminalFailure,
    AccountSwitchQueued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FailoverSelectionRequest {
    pub(crate) after: String,
    pub(crate) excluded: Vec<String>,
}

pub(crate) struct RuntimeBindingTick<'a> {
    pub(crate) binding: &'a crate::codex_account::BindingSnapshot,
    pub(crate) failover_account: Option<&'a str>,
}

struct TickFences<'a> {
    launch_id: Option<&'a str>,
    binding: Option<&'a crate::codex_account::BindingSnapshot>,
    failover_account: Option<&'a str>,
}

fn default_recovery_policy() -> String {
    WAIT_FOR_RESET_POLICY.to_string()
}

fn supported(record: &SessionRecord) -> bool {
    // Claude Code StopFailure exposes an official structured `error=rate_limit`
    // signal. Codex is supported only when this exact runtime was launched
    // through the app-server v2 protocol; the standalone TUI notification
    // surface remains fail-closed because it has no structured failure reason.
    crate::session_profile_auto_resume_supported(record)
        && (record.agent == "claude" || crate::codex_app_server::runtime_is_supported(record))
}

fn default_state(now: &str) -> DurableAutoResume {
    DurableAutoResume {
        schema_version: AUTO_RESUME_SCHEMA_VERSION.to_string(),
        enabled: false,
        recovery_policy: default_recovery_policy(),
        state: "disabled".to_string(),
        updated_at: now.to_string(),
        scheduled_at: None,
        next_check_at: None,
        failure_reason: None,
        recovery_cause: None,
        blocked_turn_id: None,
        blocked_revision: None,
        blocked_account: None,
        blocked_account_revision: None,
        blocked_binding_state: None,
        attempt: 0,
        ever_scheduled: false,
        fallback_schedules: 0,
        attempted_accounts: Vec::new(),
    }
}

fn reset_enabled_recovery(state: &mut DurableAutoResume, now: &str) {
    state.enabled = true;
    state.state = "enabled".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = None;
    state.recovery_cause = None;
    state.blocked_turn_id = None;
    state.blocked_revision = None;
    state.blocked_account = None;
    state.blocked_account_revision = None;
    state.blocked_binding_state = None;
    state.attempt = 0;
    state.ever_scheduled = false;
    state.fallback_schedules = 0;
    state.attempted_accounts.clear();
}

fn path(context: &CliContext, id: &str) -> PathBuf {
    session_dir(context, id).join(AUTO_RESUME_FILE)
}

fn read_state(context: &CliContext, id: &str, now: &str) -> Result<DurableAutoResume, CliError> {
    let path = path(context, id);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(default_state(now)),
        Err(err) => {
            return Err(CliError::runtime(
                "auto-resume-read-failed",
                format!("failed to read durable auto-resume state: {err}"),
                Some(json!({ "id": id })),
            ));
        }
    };
    let state: DurableAutoResume = serde_json::from_slice(&bytes).map_err(|_| {
        CliError::data(
            "auto-resume-state-invalid",
            "durable auto-resume state is malformed",
            Some(json!({ "id": id })),
        )
    })?;
    if state.schema_version != AUTO_RESUME_SCHEMA_VERSION {
        return Err(CliError::data(
            "auto-resume-version-unsupported",
            "durable auto-resume state has an unsupported schema version",
            Some(json!({ "id": id })),
        ));
    }
    if !matches!(
        state.recovery_policy.as_str(),
        WAIT_FOR_RESET_POLICY | NEXT_ACCOUNT_THEN_RESUME_POLICY
    ) {
        return Err(CliError::data(
            "auto-resume-state-invalid",
            "durable auto-resume state has an invalid recovery policy",
            Some(json!({ "id": id })),
        ));
    }
    if state
        .recovery_cause
        .as_deref()
        .is_some_and(|cause| !matches!(cause, USAGE_EXHAUSTION_CAUSE | PROVIDER_CAPACITY_CAUSE))
    {
        return Err(CliError::data(
            "auto-resume-state-invalid",
            "durable auto-resume state has an invalid recovery cause",
            Some(json!({ "id": id })),
        ));
    }
    Ok(state)
}

fn write_state(context: &CliContext, id: &str, state: &DurableAutoResume) -> Result<(), CliError> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|err| {
        CliError::runtime(
            "auto-resume-render-failed",
            format!("failed to render durable auto-resume state: {err}"),
            Some(json!({ "id": id })),
        )
    })?;
    write_atomic(&path(context, id), &bytes, SECRET_FILE_MODE).map_err(|err| {
        CliError::runtime(
            "auto-resume-write-failed",
            format!("failed to write durable auto-resume state: {err}"),
            Some(json!({ "id": id })),
        )
    })
}

fn view(record: &SessionRecord, state: DurableAutoResume) -> AutoResumeView {
    AutoResumeView {
        schema_version: AUTO_RESUME_SCHEMA_VERSION,
        supported: supported(record),
        enabled: state.enabled,
        recovery_policy: state.recovery_policy,
        state: state.state,
        scheduled_at: state.scheduled_at,
        failure_reason: state.failure_reason,
    }
}

fn projection_unavailable(state: &DurableAutoResume) -> bool {
    state.state == "terminal_failure"
        && state.failure_reason.as_deref() == Some("state_unavailable")
}

pub(crate) fn view_for_record(context: &CliContext, record: &SessionRecord) -> AutoResumeView {
    if crate::activity::runtime_is_unhealthy(context, record) {
        return AutoResumeView {
            schema_version: AUTO_RESUME_SCHEMA_VERSION,
            supported: supported(record),
            enabled: false,
            recovery_policy: default_recovery_policy(),
            state: "terminal_failure".to_string(),
            scheduled_at: None,
            failure_reason: Some("state_unavailable".to_string()),
        };
    }
    let now = Timestamp::now().to_string();
    match read_state(context, &record.id, &now) {
        Ok(state) => view(record, state),
        Err(_) => AutoResumeView {
            schema_version: AUTO_RESUME_SCHEMA_VERSION,
            supported: supported(record),
            enabled: false,
            recovery_policy: default_recovery_policy(),
            state: "terminal_failure".to_string(),
            scheduled_at: None,
            failure_reason: Some("state_unavailable".to_string()),
        },
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn set_enabled(
    context: &CliContext,
    id: &str,
    enabled: bool,
    now: &str,
) -> Result<AutoResumeView, CliError> {
    set_enabled_with_policy(context, id, enabled, WAIT_FOR_RESET_POLICY, now)
}

pub(crate) fn set_enabled_with_policy(
    context: &CliContext,
    id: &str,
    enabled: bool,
    recovery_policy: &str,
    now: &str,
) -> Result<AutoResumeView, CliError> {
    if !matches!(
        recovery_policy,
        WAIT_FOR_RESET_POLICY | NEXT_ACCOUNT_THEN_RESUME_POLICY
    ) {
        return Err(CliError::usage(
            "auto-resume-recovery-policy-invalid",
            "auto-resume recovery policy is invalid",
            None,
        ));
    }
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock = acquire_session_record_lock(context, &canonical_id)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if enabled && crate::activity::runtime_is_unhealthy(context, &record) {
        return Err(CliError::data(
            "auto-resume-state-unavailable",
            "auto-resume projection is unavailable until this session runtime is restarted",
            Some(json!({ "id": record.id })),
        ));
    }
    if enabled && !supported(&record) {
        return Err(CliError::data(
            "auto-resume-unsupported",
            "this provider does not expose an authoritative structured auto-resume signal",
            Some(json!({ "id": record.id, "provider": record.agent })),
        ));
    }
    if enabled
        && recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
        && (record.agent != "codex"
            || !crate::codex_account::broker_is_configured()
            || crate::codex_account::selected_account(&record).is_none())
    {
        return Err(CliError::data(
            "auto-resume-account-failover-unsupported",
            "account failover requires a bound Codex account",
            Some(json!({ "id": record.id })),
        ));
    }
    let mut state = read_state(context, &record.id, now)?;
    if projection_unavailable(&state) {
        if enabled {
            return Err(CliError::data(
                "auto-resume-state-unavailable",
                "auto-resume projection is unavailable until this session runtime is restarted",
                Some(json!({ "id": record.id })),
            ));
        }
        return Ok(view(&record, state));
    }
    state.enabled = enabled;
    state.recovery_policy = recovery_policy.to_string();
    state.state = if enabled { "enabled" } else { "disabled" }.to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = None;
    state.recovery_cause = None;
    state.blocked_turn_id = None;
    state.blocked_revision = None;
    state.blocked_account = None;
    state.blocked_account_revision = None;
    state.blocked_binding_state = None;
    state.attempt = 0;
    state.ever_scheduled = false;
    state.fallback_schedules = 0;
    state.attempted_accounts.clear();
    write_state(context, &record.id, &state)?;
    Ok(view(&record, state))
}

/// Atomically re-enable and arm usage-exhaustion recovery for one exact
/// runtime incarnation. The session-record lock fences both identity
/// validation and the auto-resume write, so a replacement reusing the public
/// session id cannot inherit an older handoff's mutation.
pub(crate) fn rearm_usage_exhaustion_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    blocked_turn_id: String,
    blocked_revision: u64,
    now: &str,
) -> Result<(), CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, Some(expected_launch_id)) {
        return Err(CliError::data(
            "auto-resume-runtime-changed",
            "auto-resume mutation refused a replacement runtime incarnation",
            Some(json!({ "id": record.id })),
        ));
    }
    if crate::activity::runtime_is_unhealthy(context, &record) {
        return Err(CliError::data(
            "auto-resume-state-unavailable",
            "auto-resume projection is unavailable until this session runtime is restarted",
            Some(json!({ "id": record.id })),
        ));
    }
    if !supported(&record) {
        return Err(CliError::data(
            "auto-resume-unsupported",
            "this provider does not expose an authoritative structured usage-exhaustion signal",
            Some(json!({ "id": record.id, "provider": record.agent })),
        ));
    }
    let mut state = read_state(context, &record.id, now)?;
    if projection_unavailable(&state) {
        return Err(CliError::data(
            "auto-resume-state-unavailable",
            "auto-resume projection is unavailable until this session runtime is restarted",
            Some(json!({ "id": record.id })),
        ));
    }
    state.enabled = true;
    state.state = "armed".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = None;
    state.recovery_cause = Some(USAGE_EXHAUSTION_CAUSE.to_string());
    state.blocked_turn_id = Some(blocked_turn_id);
    state.blocked_revision = Some(blocked_revision);
    let blocked_account = crate::codex_account::input_binding_identity(&record)?;
    state.blocked_account = blocked_account
        .as_ref()
        .map(|(account, _)| account.to_string());
    state.blocked_account_revision = blocked_account.map(|(_, revision)| revision);
    state.blocked_binding_state = None;
    state.attempt = 0;
    state.ever_scheduled = false;
    state.fallback_schedules = 0;
    state.attempted_accounts.clear();
    write_state(context, &record.id, &state)?;
    Ok(())
}

pub(crate) fn cancel(
    context: &CliContext,
    id: &str,
    now: &str,
) -> Result<AutoResumeView, CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock = acquire_session_record_lock(context, &canonical_id)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    let mut state = read_state(context, &record.id, now)?;
    if projection_unavailable(&state) {
        return Ok(view(&record, state));
    }
    if state.state == "resumed" {
        return Err(CliError::data(
            "auto-resume-already-submitted",
            "the continuation was already submitted",
            Some(json!({ "id": record.id })),
        ));
    }
    state.enabled = false;
    state.state = "cancelled".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = None;
    write_state(context, &record.id, &state)?;
    Ok(view(&record, state))
}

/// Cancel an armed continuation before a human or another control-plane caller
/// writes to the pane. The caller must already hold the session record lock so
/// cancellation and input remain one serialized operation.
pub(crate) fn cancel_for_manual_input_locked(
    context: &CliContext,
    id: &str,
    now: &str,
) -> Result<(), CliError> {
    let mut state = read_state(context, id, now)?;
    if state.enabled && state.state == "resumed" {
        // The prior automatic continuation has already been submitted. A new
        // manual input begins a distinct recovery chain, so it must not inherit
        // the accounts or retry budget consumed by the earlier failure.
        reset_enabled_recovery(&mut state, now);
        return write_state(context, id, &state);
    }
    if state.enabled && state.state == "enabled" && current_capacity_failure(context, id) {
        return cancel_unarmed_capacity(context, id, &mut state, now, "manual_input");
    }
    cancel_active_locked(context, id, now, "manual_input")
}

pub(crate) fn cancel_for_account_switch_locked(
    context: &CliContext,
    id: &str,
    now: &str,
) -> Result<(), CliError> {
    let mut state = read_state(context, id, now)?;
    if state.enabled
        && state.state == "resumed"
        && state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE)
    {
        reset_enabled_recovery(&mut state, now);
        return write_state(context, id, &state);
    }
    if state.enabled && state.state == "enabled" && current_capacity_failure(context, id) {
        return cancel_unarmed_capacity(context, id, &mut state, now, "account_switch");
    }
    cancel_active_locked(context, id, now, "account_switch")
}

fn current_capacity_failure(context: &CliContext, id: &str) -> bool {
    let Ok(record) = load_session_record(context, id) else {
        return false;
    };
    crate::activity::state_for_view(context, &record).is_some_and(|activity| {
        activity.current_turn.is_none()
            && activity.last_turn.as_ref().is_some_and(|turn| {
                turn.outcome == "failed"
                    && turn.provider_failure_kind() == Some(PROVIDER_CAPACITY_CAUSE)
            })
    })
}

fn cancel_unarmed_capacity(
    context: &CliContext,
    id: &str,
    state: &mut DurableAutoResume,
    now: &str,
    reason: &str,
) -> Result<(), CliError> {
    state.enabled = false;
    state.state = "cancelled".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = Some(reason.to_string());
    write_state(context, id, state)
}

fn cancel_active_locked(
    context: &CliContext,
    id: &str,
    now: &str,
    reason: &str,
) -> Result<(), CliError> {
    let mut state = read_state(context, id, now)?;
    if !state.enabled
        || !matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "checking" | "transient_failure" | "switching_account"
        )
    {
        return Ok(());
    }
    state.enabled = false;
    state.state = "cancelled".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = Some(reason.to_string());
    write_state(context, id, &state)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManualInputCancelOutcome {
    Ready,
    Busy,
    RuntimeChanged,
}

pub(crate) fn try_cancel_for_manual_input_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now: &str,
    record_turn_fence: bool,
) -> Result<ManualInputCancelOutcome, CliError> {
    cancel_for_manual_input_for_runtime(
        context,
        id,
        expected_launch_id,
        now,
        record_turn_fence,
        None,
    )
}

pub(crate) fn cancel_for_manual_input_for_runtime_with_timeout(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now: &str,
    record_turn_fence: bool,
) -> Result<ManualInputCancelOutcome, CliError> {
    cancel_for_manual_input_for_runtime(
        context,
        id,
        expected_launch_id,
        now,
        record_turn_fence,
        Some(PROTOCOL_STATE_LOCK_TIMEOUT),
    )
}

fn cancel_for_manual_input_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now: &str,
    record_turn_fence: bool,
    timeout: Option<Duration>,
) -> Result<ManualInputCancelOutcome, CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let lock = match timeout {
        Some(timeout) => match acquire_session_record_lock_timed(context, &canonical_id, timeout) {
            Ok(lock) => Some(lock),
            Err(error) if error.code() == "session-record-lock-timeout" => None,
            Err(error) => return Err(error),
        },
        None => try_acquire_session_record_lock(context, &canonical_id)?,
    };
    let Some(_lock) = lock else {
        return Ok(ManualInputCancelOutcome::Busy);
    };
    let mut record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, Some(expected_launch_id)) {
        return Ok(ManualInputCancelOutcome::RuntimeChanged);
    }
    crate::codex_account::authorize_proxy_input_locked(context, &mut record, record_turn_fence)?;
    cancel_for_manual_input_locked(context, &record.id, now)?;
    Ok(ManualInputCancelOutcome::Ready)
}

pub(crate) fn fail_closed_projection_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now: &str,
) -> Result<(), CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, Some(expected_launch_id)) {
        return Ok(());
    }
    let mut state = read_state(context, &record.id, now)?;
    if !state.enabled
        || !matches!(
            state.state.as_str(),
            "enabled"
                | "armed"
                | "scheduled"
                | "checking"
                | "transient_failure"
                | "switching_account"
        )
    {
        return Ok(());
    }
    state.enabled = false;
    state.state = "terminal_failure".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = Some("state_unavailable".to_string());
    write_state(context, &record.id, &state)
}

pub(crate) fn arm_usage_exhaustion(
    context: &CliContext,
    id: &str,
    blocked_turn_id: String,
    blocked_revision: u64,
    now: &str,
) -> Result<bool, CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !supported(&record) {
        return Ok(false);
    }
    let mut state = read_state(context, &record.id, now)?;
    if !state.enabled {
        return Ok(false);
    }
    if state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE) && state.state == "checking"
    {
        // A durable pre-submit claim is already in flight. Even a newer
        // capacity event cannot prove whether that submission was accepted,
        // so it must never create a replayable schedule.
        return Ok(false);
    }
    if state.blocked_turn_id.as_deref() == Some(blocked_turn_id.as_str())
        && matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "checking" | "resumed" | "switching_account"
        )
    {
        return Ok(false);
    }
    let preserve_recovery_chain = state.recovery_cause.as_deref() != Some(PROVIDER_CAPACITY_CAUSE)
        && state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
        && record.agent == "codex"
        && state.state == "resumed";
    state.state = "armed".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = None;
    state.next_check_at = None;
    state.failure_reason = None;
    state.recovery_cause = Some(USAGE_EXHAUSTION_CAUSE.to_string());
    state.blocked_turn_id = Some(blocked_turn_id);
    state.blocked_revision = Some(blocked_revision);
    let blocked_account = crate::codex_account::input_binding_identity(&record)?;
    state.blocked_account = blocked_account
        .as_ref()
        .map(|(account, _)| account.to_string());
    state.blocked_account_revision = blocked_account.map(|(_, revision)| revision);
    state.blocked_binding_state = None;
    if !preserve_recovery_chain {
        state.attempt = 0;
        state.ever_scheduled = false;
        state.attempted_accounts.clear();
    }
    write_state(context, &record.id, &state)?;
    Ok(true)
}

pub(crate) fn arm_provider_capacity(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    expected_binding: &crate::codex_account::BindingSnapshot,
    blocked_turn_id: String,
    blocked_revision: u64,
    now: &str,
) -> Result<bool, CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if record.agent != "codex"
        || !crate::codex_app_server::runtime_is_supported(&record)
        || !runtime_matches(&record, Some(expected_launch_id))
        || crate::codex_account::binding_snapshot(&record) != *expected_binding
    {
        return Ok(false);
    }
    let mut state = read_state(context, &record.id, now)?;
    if !state.enabled {
        return Ok(false);
    }
    if state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE) && state.state == "checking"
    {
        // A durable pre-submit claim is already in flight. Even a newer
        // capacity event cannot prove whether that submission was accepted,
        // so it must never create a replayable schedule.
        return Ok(false);
    }
    if !crate::activity::state_for_view(context, &record).is_some_and(|activity| {
        activity.phase == crate::activity::TurnPhase::Waiting
            && activity.revision == blocked_revision
            && activity
                .current_turn
                .as_ref()
                .and_then(|turn| turn.attention.as_ref())
                .is_none()
    }) {
        return Ok(false);
    }
    if state.blocked_turn_id.as_deref() == Some(blocked_turn_id.as_str())
        && state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE)
        && matches!(state.state.as_str(), "scheduled" | "checking" | "resumed")
    {
        return Ok(false);
    }
    let preserve_recovery_chain = state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE)
        && state.state == "resumed";
    if preserve_recovery_chain && state.attempt >= MAX_TRANSIENT_ATTEMPTS {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now.to_string();
        state.scheduled_at = None;
        state.next_check_at = None;
        state.failure_reason = Some("capacity_retry_exhausted".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(true);
    }
    if !preserve_recovery_chain {
        state.attempt = 0;
        state.fallback_schedules = 0;
    }
    let now_epoch = epoch_from_string(now).ok_or_else(|| {
        CliError::data(
            "auto-resume-time-invalid",
            "auto-resume timestamp is outside the supported range",
            None,
        )
    })?;
    let delay = RETRY_DELAYS_SECONDS[state.attempt as usize];
    state.state = "scheduled".to_string();
    state.updated_at = now.to_string();
    state.scheduled_at = Some(epoch_string(now_epoch.saturating_add(delay))?);
    state.next_check_at = None;
    state.failure_reason = None;
    state.recovery_cause = Some(PROVIDER_CAPACITY_CAUSE.to_string());
    state.blocked_turn_id = Some(blocked_turn_id);
    state.blocked_revision = Some(blocked_revision);
    record_capacity_binding(&record, &mut state);
    state.ever_scheduled = true;
    state.attempted_accounts.clear();
    write_state(context, &record.id, &state)?;
    Ok(true)
}

pub(crate) fn complete_provider_capacity_recovery(
    context: &CliContext,
    id: &str,
    completed_revision: u64,
    now: &str,
) -> Result<(), CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    let mut state = read_state(context, &record.id, now)?;
    if state.enabled
        && state.state == "resumed"
        && state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE)
        && completed_revision > state.blocked_revision.unwrap_or_default()
    {
        reset_enabled_recovery(&mut state, now);
        write_state(context, &record.id, &state)?;
    }
    Ok(())
}

pub(crate) fn pending_sessions(
    context: &CliContext,
    now_epoch: i64,
) -> Result<PendingSessions, CliError> {
    let root = context.state_dir.join("sessions");
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(PendingSessions::default());
        }
        Err(err) => {
            return Err(CliError::runtime(
                "auto-resume-discovery-failed",
                format!("failed to inspect session state: {err}"),
                None,
            ));
        }
    };
    let mut pending = PendingSessions::default();
    for entry in entries {
        let entry = entry.map_err(|err| {
            CliError::runtime(
                "auto-resume-discovery-failed",
                format!("failed to inspect session state: {err}"),
                None,
            )
        })?;
        let Ok(id) = entry.file_name().into_string() else {
            continue;
        };
        let now = Timestamp::now().to_string();
        let state = match read_state(context, &id, &now) {
            Ok(state) => state,
            Err(err) => {
                pending.error_codes.push(err.code().to_string());
                continue;
            }
        };
        if legacy_usage_window_terminal(&state) {
            match load_session_record(context, &id) {
                Ok(record) if record.agent == "claude" => pending.usage_ids.push(id),
                Ok(_) => {}
                Err(err) => pending.error_codes.push(err.code().to_string()),
            }
            continue;
        }
        if !state.enabled {
            continue;
        }
        if state.state == "checking" {
            pending.recovery_ids.push(id);
            continue;
        }
        let pending_ids = if state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE) {
            &mut pending.capacity_ids
        } else {
            &mut pending.usage_ids
        };
        match state.state.as_str() {
            "switching_account" => pending_ids.push(id),
            "armed" => pending_ids.push(id),
            "scheduled"
                if state
                    .scheduled_at
                    .as_deref()
                    .and_then(epoch_from_string)
                    .is_none_or(|due| due <= now_epoch) =>
            {
                pending_ids.push(id);
            }
            "transient_failure"
                if state
                    .next_check_at
                    .as_deref()
                    .and_then(epoch_from_string)
                    .is_none_or(|due| due <= now_epoch) =>
            {
                pending_ids.push(id);
            }
            _ => {}
        }
    }
    pending.recovery_ids.sort();
    pending.capacity_ids.sort();
    pending.usage_ids.sort();
    pending.error_codes.sort();
    Ok(pending)
}

pub(crate) fn record_scheduler_error(
    context: &CliContext,
    id: &str,
    now_epoch: i64,
    reason: &str,
) -> Result<TickOutcome, CliError> {
    record_scheduler_error_inner(context, id, None, now_epoch, reason)
}

pub(crate) fn record_scheduler_error_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now_epoch: i64,
    reason: &str,
) -> Result<TickOutcome, CliError> {
    record_scheduler_error_inner(context, id, Some(expected_launch_id), now_epoch, reason)
}

fn record_scheduler_error_inner(
    context: &CliContext,
    id: &str,
    expected_launch_id: Option<&str>,
    now_epoch: i64,
    reason: &str,
) -> Result<TickOutcome, CliError> {
    let now = epoch_string(now_epoch)?;
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock =
        acquire_session_record_lock_timed(context, &canonical_id, PROTOCOL_STATE_LOCK_TIMEOUT)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, expected_launch_id) {
        return Ok(TickOutcome::Unchanged);
    }
    if crate::activity::runtime_is_unhealthy(context, &record) {
        let mut state = read_state(context, &record.id, &now)?;
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now;
        state.failure_reason = Some("state_unavailable".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    let state = read_state(context, &record.id, &now)?;
    if !state.enabled
        || !matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "transient_failure" | "switching_account"
        )
    {
        return Ok(TickOutcome::Unchanged);
    }
    if state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE) {
        return record_capacity_scheduler_retry(context, &record, state, now_epoch, reason);
    }
    record_retry(context, &record, state, now_epoch, reason)
}

/// Advance an already-scheduled claim when the bound provider reports that
/// usage is available before the previously advertised reset epoch. This is
/// intentionally narrower than arming: reconnects and rate-limit updates may
/// wake an existing claim, but can never create one or revive a cancellation.
#[cfg(test)]
pub(crate) fn wake_scheduled_if_usage_open(
    context: &CliContext,
    id: &str,
    now_epoch: i64,
) -> Result<bool, CliError> {
    wake_scheduled_if_usage_open_inner(context, id, None, now_epoch)
}

pub(crate) fn wake_scheduled_if_usage_open_for_runtime(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now_epoch: i64,
) -> Result<bool, CliError> {
    wake_scheduled_if_usage_open_inner(context, id, Some(expected_launch_id), now_epoch)
}

fn wake_scheduled_if_usage_open_inner(
    context: &CliContext,
    id: &str,
    expected_launch_id: Option<&str>,
    now_epoch: i64,
) -> Result<bool, CliError> {
    let now = epoch_string(now_epoch)?;
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let Some(_lock) = try_acquire_session_record_lock(context, &canonical_id)? else {
        return Ok(false);
    };
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, expected_launch_id) {
        return Ok(false);
    }
    let mut state = read_state(context, &record.id, &now)?;
    if !state.enabled || state.state != "scheduled" || !state.ever_scheduled {
        return Ok(false);
    }
    let due = state
        .scheduled_at
        .as_deref()
        .and_then(epoch_from_string)
        .is_none_or(|due| due <= now_epoch);
    if due {
        return Ok(false);
    }
    state.updated_at = now.clone();
    state.scheduled_at = Some(now);
    write_state(context, &record.id, &state)?;
    Ok(true)
}

pub(crate) fn failover_selection_request(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    expected_binding: &crate::codex_account::BindingSnapshot,
    current_usage_exhausted: bool,
) -> Result<Option<FailoverSelectionRequest>, CliError> {
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock = acquire_session_record_lock(context, &canonical_id)?;
    let record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, Some(expected_launch_id))
        || crate::codex_account::binding_snapshot(&record) != *expected_binding
    {
        return Ok(None);
    }
    let state = read_state(context, &record.id, &Timestamp::now().to_string())?;
    if !state.enabled
        || state.recovery_policy != NEXT_ACCOUNT_THEN_RESUME_POLICY
        || !matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "transient_failure" | "switching_account"
        )
    {
        return Ok(None);
    }
    if crate::codex_account::next_transition_state(&record)
        != crate::codex_account::NextTransitionState::Absent
    {
        return Ok(None);
    }
    if !blocked_account_identity_available(&state) {
        return Ok(None);
    }
    if !current_usage_exhausted && current_account_changed(&record, &state) {
        return Ok(None);
    }
    let Some(after) = crate::codex_account::selected_account(&record) else {
        return Ok(None);
    };
    let mut excluded = state.attempted_accounts;
    if !excluded.contains(&after) {
        excluded.push(after.clone());
    }
    Ok(Some(FailoverSelectionRequest { after, excluded }))
}

fn blocked_account_identity_available(state: &DurableAutoResume) -> bool {
    matches!(
        (state.blocked_account.as_deref(), state.blocked_account_revision),
        (Some(account), Some(revision)) if !account.is_empty() && revision > 0
    )
}

fn record_capacity_binding(record: &SessionRecord, state: &mut DurableAutoResume) {
    match crate::codex_account::binding_snapshot(record) {
        crate::codex_account::BindingSnapshot::Unbound => {
            state.blocked_binding_state = Some("unbound".to_string());
            state.blocked_account = None;
            state.blocked_account_revision = None;
        }
        crate::codex_account::BindingSnapshot::Bound { account, revision } => {
            state.blocked_binding_state = Some("bound".to_string());
            state.blocked_account = Some(account);
            state.blocked_account_revision = Some(revision);
        }
        crate::codex_account::BindingSnapshot::Blocked => {
            state.blocked_binding_state = Some("blocked".to_string());
            state.blocked_account = None;
            state.blocked_account_revision = None;
        }
    }
}

fn capacity_binding_matches(record: &SessionRecord, state: &DurableAutoResume) -> bool {
    match (
        state.blocked_binding_state.as_deref(),
        state.blocked_account.as_deref(),
        state.blocked_account_revision,
        crate::codex_account::binding_snapshot(record),
    ) {
        (Some("unbound"), None, None, crate::codex_account::BindingSnapshot::Unbound) => true,
        (
            Some("bound"),
            Some(expected_account),
            Some(expected_revision),
            crate::codex_account::BindingSnapshot::Bound { account, revision },
        ) => account == expected_account && revision == expected_revision,
        (Some("blocked"), None, None, crate::codex_account::BindingSnapshot::Blocked) => true,
        _ => false,
    }
}

fn current_account_changed(record: &SessionRecord, state: &DurableAutoResume) -> bool {
    match (
        state.blocked_account.as_deref(),
        state.blocked_account_revision,
        crate::codex_account::binding_snapshot(record),
    ) {
        (
            Some(blocked_account),
            Some(blocked_revision),
            crate::codex_account::BindingSnapshot::Bound { account, revision },
        ) => account != blocked_account && revision != blocked_revision,
        _ => false,
    }
}

pub(crate) fn tick<F>(
    context: &CliContext,
    id: &str,
    now_epoch: i64,
    usage: &UsageSnapshot,
    submit: F,
) -> Result<TickOutcome, CliError>
where
    F: FnMut(&SessionRecord) -> Result<(), CliError>,
{
    tick_inner(
        context,
        id,
        TickFences {
            launch_id: None,
            binding: None,
            failover_account: None,
        },
        now_epoch,
        usage,
        submit,
    )
}

#[cfg(test)]
pub(crate) fn tick_for_runtime<F>(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    now_epoch: i64,
    usage: &UsageSnapshot,
    submit: F,
) -> Result<TickOutcome, CliError>
where
    F: FnMut(&SessionRecord) -> Result<(), CliError>,
{
    tick_inner(
        context,
        id,
        TickFences {
            launch_id: Some(expected_launch_id),
            binding: None,
            failover_account: None,
        },
        now_epoch,
        usage,
        submit,
    )
}

pub(crate) fn tick_for_runtime_and_binding<F>(
    context: &CliContext,
    id: &str,
    expected_launch_id: &str,
    runtime_binding: RuntimeBindingTick<'_>,
    now_epoch: i64,
    usage: &UsageSnapshot,
    submit: F,
) -> Result<TickOutcome, CliError>
where
    F: FnMut(&SessionRecord) -> Result<(), CliError>,
{
    tick_inner(
        context,
        id,
        TickFences {
            launch_id: Some(expected_launch_id),
            binding: Some(runtime_binding.binding),
            failover_account: runtime_binding.failover_account,
        },
        now_epoch,
        usage,
        submit,
    )
}

fn tick_inner<F>(
    context: &CliContext,
    id: &str,
    fences: TickFences<'_>,
    now_epoch: i64,
    usage: &UsageSnapshot,
    mut submit: F,
) -> Result<TickOutcome, CliError>
where
    F: FnMut(&SessionRecord) -> Result<(), CliError>,
{
    let now = epoch_string(now_epoch)?;
    let observed = load_session_record(context, id)?;
    let canonical_id = observed.id.clone();
    let _lock = acquire_session_record_lock(context, &canonical_id)?;
    let mut record = load_session_record(context, &canonical_id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    if !runtime_matches(&record, fences.launch_id) {
        return Ok(TickOutcome::Unchanged);
    }
    if fences
        .binding
        .is_some_and(|expected| crate::codex_account::binding_snapshot(&record) != *expected)
    {
        return Ok(TickOutcome::Unchanged);
    }
    let mut state = read_state(context, &record.id, &now)?;
    if !supported(&record) {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now;
        state.failure_reason = Some("provider_unsupported".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    if state.state == "checking" {
        let activity = crate::activity::state_for_view(context, &record);
        if activity
            .as_ref()
            .is_some_and(|activity| activity.revision > state.blocked_revision.unwrap_or_default())
        {
            state.state = "resumed".to_string();
            state.updated_at = now;
            state.failure_reason = None;
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::Resumed);
        }
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now;
        state.failure_reason = Some("submission_outcome_unknown".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    if record.agent == "claude" && legacy_usage_window_terminal(&state) {
        if !blocked_claim_is_eligible(context, &record, &state) {
            state.failure_reason = Some("session_state_changed".to_string());
            state.updated_at = now;
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::TerminalFailure);
        }
        state.enabled = true;
        state.state = "armed".to_string();
        state.updated_at = now.clone();
        state.failure_reason = None;
        state.attempt = 0;
        state.ever_scheduled = false;
    }
    if !state.enabled
        || !matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "transient_failure" | "switching_account"
        )
    {
        return Ok(TickOutcome::Unchanged);
    }
    if let Some(next) = state.next_check_at.as_deref().and_then(epoch_from_string)
        && next > now_epoch
    {
        return Ok(TickOutcome::Unchanged);
    }
    if let Some(scheduled) = state.scheduled_at.as_deref().and_then(epoch_from_string)
        && scheduled > now_epoch
    {
        return Ok(TickOutcome::Unchanged);
    }

    let capacity_recovery = state.recovery_cause.as_deref() == Some(PROVIDER_CAPACITY_CAUSE);
    if capacity_recovery {
        if state.attempt >= MAX_TRANSIENT_ATTEMPTS {
            state.enabled = false;
            state.state = "terminal_failure".to_string();
            state.updated_at = now;
            state.failure_reason = Some("capacity_retry_exhausted".to_string());
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::TerminalFailure);
        }
        if !capacity_binding_matches(&record, &state) {
            state.enabled = false;
            state.state = "terminal_failure".to_string();
            state.updated_at = now;
            state.failure_reason = Some("account_changed".to_string());
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::TerminalFailure);
        }
    }

    if !capacity_recovery {
        if !usage.authoritative {
            return record_retry(context, &record, state, now_epoch, "usage_unavailable");
        }

        if state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
            && record.agent == "codex"
            && !blocked_account_identity_available(&state)
        {
            return record_retry(context, &record, state, now_epoch, "state_unavailable");
        }

        let current_account_changed = current_account_changed(&record, &state);

        if state.state == "switching_account" {
            match crate::codex_account::next_transition_state(&record) {
                crate::codex_account::NextTransitionState::Pending => {
                    return Ok(TickOutcome::Unchanged);
                }
                crate::codex_account::NextTransitionState::Failed
                | crate::codex_account::NextTransitionState::Invalid => {
                    state.enabled = false;
                    state.state = "terminal_failure".to_string();
                    state.updated_at = now;
                    state.failure_reason = Some("account_switch_failed".to_string());
                    write_state(context, &record.id, &state)?;
                    return Ok(TickOutcome::TerminalFailure);
                }
                crate::codex_account::NextTransitionState::Absent => {}
            }
        }

        // The structured provider rejection is authoritative evidence that the
        // bound account cannot accept this turn. Its percentage windows may still
        // be open when a distinct workspace-credit pool is exhausted, so a fresh
        // broker-confirmed alternative is sufficient to authorize failover.
        if state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
            && record.agent == "codex"
            && (!current_account_changed || usage.has_exhausted_windows)
            && let Some(account) = fences.failover_account
        {
            let current = crate::codex_account::selected_account(&record);
            if current.as_deref() != Some(account)
                && !state.attempted_accounts.iter().any(|item| item == account)
            {
                if let Some(current) = current
                    && !state.attempted_accounts.contains(&current)
                {
                    state.attempted_accounts.push(current);
                }
                state.attempted_accounts.push(account.to_string());
                state.state = "switching_account".to_string();
                state.updated_at = now.clone();
                state.scheduled_at = None;
                state.next_check_at = None;
                state.failure_reason = None;
                state.ever_scheduled = true;
                write_state(context, &record.id, &state)?;
                crate::codex_account::queue_auto_failover_locked(context, &mut record, account)?;
                return Ok(TickOutcome::AccountSwitchQueued);
            }
        }

        if usage.has_exhausted_windows {
            let Some(latest_reset) = usage.exhausted_reset_epochs.iter().copied().max() else {
                return record_retry(
                    context,
                    &record,
                    state,
                    now_epoch,
                    "exhausted_reset_unavailable",
                );
            };
            let wake_epoch = latest_reset.max(now_epoch.saturating_add(1))
                + bounded_jitter_seconds(
                    &record.id,
                    state.blocked_turn_id.as_deref().unwrap_or(""),
                );
            state.state = "scheduled".to_string();
            state.updated_at = now;
            state.scheduled_at = Some(epoch_string(wake_epoch)?);
            state.next_check_at = None;
            state.failure_reason = (state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY)
                .then(|| "no_account_available".to_string());
            state.ever_scheduled = true;
            // A real percentage-window exhaustion drove this schedule, so the
            // authoritative-arming fallback below is no longer in play; clear its
            // budget.
            state.fallback_schedules = 0;
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::Scheduled);
        }

        if state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
            && record.agent == "codex"
            && !current_account_changed
        {
            return record_retry(context, &record, state, now_epoch, "no_account_available");
        }

        if !state.ever_scheduled && !current_account_changed {
            // The session was armed by an authoritative provider rate-limit signal
            // (see `arm_usage_exhaustion`), yet no usage window reports
            // `used_percent >= 100`. Claude can expose this as a session limit, so
            // it schedules from the nearest window or uses a bounded unknown-reset
            // probe.
            let fallback_reset = match usage.soonest_reset_epoch.filter(|reset| *reset > now_epoch)
            {
                Some(reset) => reset,
                None if record.agent == "claude" => now_epoch
                    .saturating_add(unknown_reset_probe_delay_seconds(state.fallback_schedules)),
                None => {
                    return record_retry(
                        context,
                        &record,
                        state,
                        now_epoch,
                        "usage_window_not_exhausted",
                    );
                }
            };
            let wake_epoch = fallback_reset
                + bounded_jitter_seconds(
                    &record.id,
                    state.blocked_turn_id.as_deref().unwrap_or(""),
                );
            state.state = "scheduled".to_string();
            state.updated_at = now;
            state.scheduled_at = Some(epoch_string(wake_epoch)?);
            state.next_check_at = None;
            state.failure_reason = None;
            state.attempt = 0;
            state.ever_scheduled = true;
            state.fallback_schedules = state.fallback_schedules.saturating_add(1);
            write_state(context, &record.id, &state)?;
            return Ok(TickOutcome::Scheduled);
        }
    }

    if !blocked_claim_is_eligible(context, &record, &state) {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now;
        state.failure_reason = Some("session_state_changed".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }

    // Claim before submitting. A crash after this durable write is never
    // retried automatically, which preserves the no-duplicate guarantee.
    let health_fence = crate::activity::acquire_runtime_health_fence(context, &record)?;
    if crate::activity::runtime_is_unhealthy(context, &record) {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.updated_at = now;
        state.failure_reason = Some("state_unavailable".to_string());
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    if record.agent == "codex" {
        crate::codex_account::authorize_input_locked(context, &mut record)?;
    }
    if capacity_recovery {
        state.attempt = state.attempt.saturating_add(1);
    }
    state.state = "checking".to_string();
    state.updated_at = now.clone();
    state.scheduled_at = None;
    state.next_check_at = None;
    write_state(context, &record.id, &state)?;
    drop(health_fence);
    match submit(&record) {
        Ok(()) => {
            state.state = "resumed".to_string();
            state.updated_at = now;
            state.failure_reason = None;
            if capacity_recovery {
                state.fallback_schedules = 0;
            }
            write_state(context, &record.id, &state)?;
            Ok(TickOutcome::Resumed)
        }
        Err(_) => {
            state.enabled = false;
            state.state = "terminal_failure".to_string();
            state.updated_at = now;
            state.failure_reason = Some("submission_outcome_unknown".to_string());
            write_state(context, &record.id, &state)?;
            Ok(TickOutcome::TerminalFailure)
        }
    }
}

fn legacy_usage_window_terminal(state: &DurableAutoResume) -> bool {
    !state.enabled
        && state.state == "terminal_failure"
        && state.failure_reason.as_deref() == Some("usage_window_not_exhausted")
        && state.blocked_turn_id.is_some()
        && state.blocked_revision.is_some()
}

fn blocked_claim_is_eligible(
    context: &CliContext,
    record: &SessionRecord,
    state: &DurableAutoResume,
) -> bool {
    crate::activity::state_for_view(context, record)
        .as_ref()
        .is_some_and(|activity| {
            activity.phase == crate::activity::TurnPhase::Waiting
                && activity.revision == state.blocked_revision.unwrap_or_default()
                && activity
                    .current_turn
                    .as_ref()
                    .and_then(|turn| turn.attention.as_ref())
                    .is_none()
        })
}

pub(crate) fn has_authoritative_usage_exhaustion_idle(
    context: &CliContext,
    record: &SessionRecord,
) -> bool {
    let Ok(state) = read_state(context, &record.id, &Timestamp::now().to_string()) else {
        return false;
    };
    let eligible = blocked_claim_is_eligible(context, record, &state);
    state.enabled
        && state.recovery_policy == NEXT_ACCOUNT_THEN_RESUME_POLICY
        && blocked_account_identity_available(&state)
        && matches!(
            state.state.as_str(),
            "armed" | "scheduled" | "transient_failure" | "switching_account"
        )
        && eligible
}

fn runtime_matches(record: &SessionRecord, expected_launch_id: Option<&str>) -> bool {
    expected_launch_id.is_none_or(|expected| {
        record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.launch_id == expected)
    })
}

fn record_retry(
    context: &CliContext,
    record: &SessionRecord,
    mut state: DurableAutoResume,
    now_epoch: i64,
    reason: &str,
) -> Result<TickOutcome, CliError> {
    state.attempt = state.attempt.saturating_add(1);
    state.updated_at = epoch_string(now_epoch)?;
    state.failure_reason = Some(reason.to_string());
    state.scheduled_at = None;
    if state.attempt >= MAX_TRANSIENT_ATTEMPTS {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.next_check_at = None;
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    let delay = RETRY_DELAYS_SECONDS[state.attempt.saturating_sub(1) as usize];
    state.state = "transient_failure".to_string();
    state.next_check_at = Some(epoch_string(now_epoch.saturating_add(delay))?);
    write_state(context, &record.id, &state)?;
    Ok(TickOutcome::Retrying)
}

fn record_capacity_scheduler_retry(
    context: &CliContext,
    record: &SessionRecord,
    mut state: DurableAutoResume,
    now_epoch: i64,
    reason: &str,
) -> Result<TickOutcome, CliError> {
    state.fallback_schedules = state.fallback_schedules.saturating_add(1);
    state.updated_at = epoch_string(now_epoch)?;
    state.failure_reason = Some(reason.to_string());
    state.scheduled_at = None;
    if state.fallback_schedules >= MAX_TRANSIENT_ATTEMPTS {
        state.enabled = false;
        state.state = "terminal_failure".to_string();
        state.next_check_at = None;
        write_state(context, &record.id, &state)?;
        return Ok(TickOutcome::TerminalFailure);
    }
    let delay = RETRY_DELAYS_SECONDS[state.fallback_schedules.saturating_sub(1) as usize];
    state.state = "transient_failure".to_string();
    state.next_check_at = Some(epoch_string(now_epoch.saturating_add(delay))?);
    write_state(context, &record.id, &state)?;
    Ok(TickOutcome::Retrying)
}

fn bounded_jitter_seconds(id: &str, blocked_turn_id: &str) -> i64 {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update([0]);
    hasher.update(blocked_turn_id.as_bytes());
    i64::from(hasher.finalize()[0] % 31)
}

fn unknown_reset_probe_delay_seconds(fallback_schedules: u32) -> i64 {
    let index = usize::try_from(fallback_schedules)
        .unwrap_or(usize::MAX)
        .min(UNKNOWN_RESET_PROBE_DELAYS_SECONDS.len() - 1);
    UNKNOWN_RESET_PROBE_DELAYS_SECONDS[index]
}

fn epoch_string(epoch: i64) -> Result<String, CliError> {
    Timestamp::from_second(epoch)
        .map(|timestamp| timestamp.to_string())
        .map_err(|_| {
            CliError::data(
                "auto-resume-time-invalid",
                "auto-resume timestamp is outside the supported range",
                None,
            )
        })
}

fn epoch_from_string(value: &str) -> Option<i64> {
    value
        .parse::<Timestamp>()
        .ok()
        .map(|value| value.as_second())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeInfo, activity};
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn context(tmp: &tempfile::TempDir) -> CliContext {
        CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        }
    }

    fn seed_session(tmp: &tempfile::TempDir) -> (CliContext, SessionRecord) {
        let context = context(tmp);
        let id = "claude-reset";
        let dir = session_dir(&context, id);
        fs::create_dir_all(&dir).unwrap();
        let record = SessionRecord {
            schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
            id: id.to_string(),
            agent: "claude".to_string(),
            mode: "interactive".to_string(),
            coordination_mode: crate::cli::CoordinationMode::Advisory,
            title: None,
            title_state: None,
            title_revision: 0,
            cwd: "/repo".to_string(),
            tmux_session: "hs-claude-reset".to_string(),
            prompt_file: None,
            log_file: None,
            created_at: "2030-01-01T00:00:00Z".to_string(),
            updated_at: "2030-01-01T00:00:00Z".to_string(),
            provider_resume: None,
            runtime: Some(RuntimeInfo {
                kind: "tmux".to_string(),
                tmux_session: "hs-claude-reset".to_string(),
                generation: 1,
                started_at: "2030-01-01T00:00:00Z".to_string(),
                launch_id: "runtime-1".to_string(),
                extra: BTreeMap::new(),
            }),
            public_metadata: None,
            agent_args: Vec::new(),
            agent_bin: None,
            extra: BTreeMap::new(),
            resume_sidecar_extra: BTreeMap::new(),
        };
        crate::write_session_record(&context, &record).unwrap();
        activity::activate_runtime(&context, &record).unwrap();
        (context, record)
    }

    #[test]
    fn launch_profile_can_fail_closed_auto_resume_support() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_, mut record) = seed_session(&tmp);
        record.runtime.as_mut().unwrap().extra.insert(
            "agent_profile_auto_resume_supported".to_string(),
            json!(false),
        );

        assert!(!supported(&record));
    }

    fn seed_bound_codex_auto_resume(tmp: &tempfile::TempDir) -> (CliContext, SessionRecord, u64) {
        let (context, mut record) = seed_session(tmp);
        record.agent = "codex".to_string();
        let runtime = record.runtime.as_mut().unwrap();
        runtime.kind = crate::codex_app_server::RUNTIME_KIND.to_string();
        runtime.extra.extend(BTreeMap::from([
            (
                crate::codex_app_server::PROTOCOL_KEY.to_string(),
                json!(crate::codex_app_server::PROTOCOL_VERSION),
            ),
            (
                crate::codex_app_server::SOCKET_KEY.to_string(),
                json!("/run/codex.sock"),
            ),
            (
                crate::codex_app_server::PROXY_KEY.to_string(),
                json!("/run/codex.proxy"),
            ),
            (
                crate::codex_app_server::THREAD_HANDOFF_KEY.to_string(),
                json!("/run/codex.thread"),
            ),
            (
                crate::codex_app_server::THREAD_ATTACHED_KEY.to_string(),
                json!("/run/codex.attached"),
            ),
        ]));
        crate::codex_account::set_initial_binding_with_source(
            &mut record,
            Some("account-a"),
            Some("default_at_launch"),
        )
        .unwrap();
        crate::write_session_record(&context, &record).unwrap();
        crate::codex_account::finish_binding(
            &context,
            &record.id,
            "runtime-1",
            "account-a",
            1,
            Ok(()),
        )
        .unwrap();
        let mut record = crate::load_session_record(&context, &record.id).unwrap();
        crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
        for (event_id, kind) in [
            ("codex-start", "turn_started"),
            ("codex-done", "turn_completed"),
        ] {
            activity::ingest_event(
                &context,
                &record.id,
                serde_json::from_value(json!({
                    "schema_version": crate::activity::TURN_EVENT_VERSION,
                    "event_id": event_id,
                    "runtime_id": "runtime-1",
                    "provider": "codex",
                    "provider_turn_id": "turn-1",
                    "kind": kind,
                    "confidence": "authoritative"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let record = crate::load_session_record(&context, &record.id).unwrap();
        let revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;
        (context, record, revision)
    }

    fn ingest_provider_capacity(context: &CliContext, record: &SessionRecord, turn_id: &str) {
        activity::ingest_event(
            context,
            &record.id,
            serde_json::from_value(json!({
                "schema_version": crate::activity::TURN_EVENT_VERSION,
                "event_id": format!("{turn_id}-started"),
                "runtime_id": record.runtime.as_ref().unwrap().launch_id.as_str(),
                "provider": "codex",
                "provider_session_id": "thread-capacity",
                "provider_turn_id": turn_id,
                "kind": "turn_started",
                "confidence": "authoritative"
            }))
            .unwrap(),
        )
        .unwrap();
        activity::ingest_codex_app_server_failure_with_kind(
            context,
            &record.id,
            &record.runtime.as_ref().unwrap().launch_id,
            "thread-capacity",
            turn_id,
            crate::codex_app_server::StructuredFailureKind::ProviderCapacity,
        )
        .unwrap();
    }

    #[test]
    fn capacity_recovery_uses_bounded_delays_and_stops_after_five_submissions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let no_usage = UsageSnapshot {
            authoritative: false,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        };
        let mut submissions = 0;

        for (index, expected_delay) in RETRY_DELAYS_SECONDS.into_iter().enumerate() {
            ingest_provider_capacity(&context, &record, &format!("capacity-{index}"));
            let state = read_state(&context, &record.id, "2030-01-01T00:00:00Z").unwrap();
            assert_eq!(
                state.recovery_cause.as_deref(),
                Some(PROVIDER_CAPACITY_CAUSE)
            );
            assert_eq!(state.state, "scheduled");
            let scheduled = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
            let updated = epoch_from_string(&state.updated_at).unwrap();
            assert_eq!(scheduled - updated, expected_delay);

            let pending_before = pending_sessions(&context, scheduled - 1).unwrap();
            assert!(pending_before.capacity_ids.is_empty());
            let pending_due = pending_sessions(&context, scheduled).unwrap();
            assert_eq!(pending_due.capacity_ids, vec![record.id.clone()]);
            assert!(pending_due.usage_ids.is_empty());

            let outcome = tick_for_runtime(
                &context,
                &record.id,
                &record.runtime.as_ref().unwrap().launch_id,
                scheduled,
                &no_usage,
                |_| {
                    submissions += 1;
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(outcome, TickOutcome::Resumed);
            assert_eq!(submissions, index + 1);
        }

        ingest_provider_capacity(&context, &record, "capacity-exhausted");
        let state = read_state(&context, &record.id, "2030-01-01T00:00:00Z").unwrap();
        assert!(!state.enabled);
        assert_eq!(state.state, "terminal_failure");
        assert_eq!(
            state.failure_reason.as_deref(),
            Some("capacity_retry_exhausted")
        );
        assert_eq!(submissions, 5);
    }

    #[test]
    fn capacity_recovery_is_opt_in_deduplicated_and_runtime_fenced() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);

        ingest_provider_capacity(&context, &record, "disabled-capacity");
        assert_eq!(view_for_record(&context, &record).state, "disabled");

        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        ingest_provider_capacity(&context, &record, "scheduled-capacity");
        let before_duplicate = fs::read(path(&context, &record.id)).unwrap();
        ingest_provider_capacity(&context, &record, "scheduled-capacity");
        assert_eq!(
            fs::read(path(&context, &record.id)).unwrap(),
            before_duplicate
        );

        let state = read_state(&context, &record.id, "2030-01-01T00:00:00Z").unwrap();
        let scheduled = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        let mut submissions = 0;
        let outcome = tick_for_runtime(
            &context,
            &record.id,
            "replacement-runtime",
            scheduled,
            &UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Unchanged);
        assert_eq!(submissions, 0);

        cancel_for_manual_input_locked(&context, &record.id, "2030-01-01T00:00:01Z").unwrap();
        let view = view_for_record(&context, &record);
        assert_eq!(view.state, "cancelled");
        assert_eq!(view.failure_reason.as_deref(), Some("manual_input"));
    }

    #[test]
    fn capacity_arming_refuses_an_account_mutation_after_the_failure_event() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _) = seed_bound_codex_auto_resume(&tmp);
        let expected_binding = crate::codex_account::binding_snapshot(&record);
        ingest_provider_capacity(&context, &record, "pre-mutation-capacity");
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;

        record.extra.get_mut("codex_account_binding").unwrap()["revision"] = json!(2);
        crate::write_session_record(&context, &record).unwrap();
        assert!(
            !arm_provider_capacity(
                &context,
                &record.id,
                &record.runtime.as_ref().unwrap().launch_id,
                &expected_binding,
                "pre-mutation-capacity".to_string(),
                revision,
                "2030-01-01T00:00:01Z",
            )
            .unwrap()
        );
        assert_eq!(view_for_record(&context, &record).state, "enabled");
    }

    #[test]
    fn manual_input_wins_the_gap_between_capacity_projection_and_arming() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);
        let expected_binding = crate::codex_account::binding_snapshot(&record);
        ingest_provider_capacity(&context, &record, "capacity-before-manual");
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;
        assert!(
            current_capacity_failure(&context, &record.id),
            "state={:?}",
            activity::state_for_view(&context, &record)
        );

        cancel_for_manual_input_locked(&context, &record.id, "2030-01-01T00:00:01Z").unwrap();
        assert!(
            !arm_provider_capacity(
                &context,
                &record.id,
                &record.runtime.as_ref().unwrap().launch_id,
                &expected_binding,
                "capacity-before-manual".to_string(),
                revision,
                "2030-01-01T00:00:02Z",
            )
            .unwrap()
        );
        let view = view_for_record(&context, &record);
        assert!(!view.enabled);
        assert_eq!(view.state, "cancelled");
        assert_eq!(view.failure_reason.as_deref(), Some("manual_input"));
    }

    #[test]
    fn pending_attention_prevents_capacity_arming() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _) = seed_bound_codex_auto_resume(&tmp);
        record.runtime.as_mut().unwrap().extra.insert(
            crate::codex_app_server::ATTENTION_AUTHORITY_KEY.to_string(),
            json!("protocol"),
        );
        crate::write_session_record(&context, &record).unwrap();
        let expected_binding = crate::codex_account::binding_snapshot(&record);
        ingest_provider_capacity(&context, &record, "capacity-before-attention");
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let blocked_revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;
        activity::ingest_codex_app_server_attention(
            &context,
            &record.id,
            &record.runtime.as_ref().unwrap().launch_id,
            "thread-capacity",
            None,
            "capacity-approval",
            Some("approval"),
        )
        .unwrap();

        assert!(
            !arm_provider_capacity(
                &context,
                &record.id,
                &record.runtime.as_ref().unwrap().launch_id,
                &expected_binding,
                "capacity-before-attention".to_string(),
                blocked_revision,
                "2030-01-01T00:00:01Z",
            )
            .unwrap()
        );
        assert_eq!(view_for_record(&context, &record).state, "enabled");
    }

    #[test]
    fn capacity_submission_with_an_unknown_outcome_is_never_replayed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        ingest_provider_capacity(&context, &record, "capacity-unknown");
        let state = read_state(&context, &record.id, "2030-01-01T00:00:00Z").unwrap();
        let scheduled = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();

        assert_eq!(
            tick_for_runtime(
                &context,
                &record.id,
                &record.runtime.as_ref().unwrap().launch_id,
                scheduled,
                &UsageSnapshot {
                    authoritative: false,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                },
                |_| Err(CliError::runtime("submit-unknown", "unknown", None)),
            )
            .unwrap(),
            TickOutcome::TerminalFailure
        );
        let view = view_for_record(&context, &record);
        assert!(!view.enabled);
        assert_eq!(view.state, "terminal_failure");
        assert_eq!(
            view.failure_reason.as_deref(),
            Some("submission_outcome_unknown")
        );
        assert_eq!(
            pending_sessions(&context, scheduled + 10_000).unwrap(),
            PendingSessions::default()
        );
    }

    #[test]
    fn successful_capacity_continuation_completion_clears_the_retry_chain() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        ingest_provider_capacity(&context, &record, "capacity-completes");
        let state = read_state(&context, &record.id, "2030-01-01T00:00:00Z").unwrap();
        let scheduled = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        tick_for_runtime(
            &context,
            &record.id,
            &record.runtime.as_ref().unwrap().launch_id,
            scheduled,
            &UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| Ok(()),
        )
        .unwrap();
        for (suffix, kind) in [("started", "turn_started"), ("completed", "turn_completed")] {
            activity::ingest_event(
                &context,
                &record.id,
                serde_json::from_value(json!({
                    "schema_version": crate::activity::TURN_EVENT_VERSION,
                    "event_id": format!("capacity-success-{suffix}"),
                    "runtime_id": record.runtime.as_ref().unwrap().launch_id.as_str(),
                    "provider": "codex",
                    "provider_session_id": "thread-capacity",
                    "provider_turn_id": "capacity-success",
                    "kind": kind,
                    "confidence": "authoritative"
                }))
                .unwrap(),
            )
            .unwrap();
        }

        let cleared = read_state(&context, &record.id, "2030-01-01T00:01:00Z").unwrap();
        assert!(cleared.enabled);
        assert_eq!(cleared.state, "enabled");
        assert_eq!(cleared.recovery_cause, None);
        assert_eq!(cleared.attempt, 0);
    }

    #[test]
    fn capacity_continuation_prompt_is_fixed_and_short() {
        assert_eq!(
            CAPACITY_CONTINUATION_MESSAGE,
            "The selected model was at capacity, interrupting the previous turn. Please continue from where you stopped."
        );
    }

    #[test]
    fn account_failover_policy_is_durable_and_projected() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);

        let view = set_enabled_with_policy(
            &context,
            &record.id,
            true,
            "next_account_then_resume",
            "2030-01-01T00:00:00Z",
        )
        .unwrap();

        assert_eq!(view.recovery_policy, "next_account_then_resume");
        assert_eq!(
            read_state(&context, &record.id, "2030-01-01T00:00:01Z")
                .unwrap()
                .recovery_policy,
            "next_account_then_resume"
        );

        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let bound = crate::load_session_record(&context, &record.id).unwrap();
        let binding = crate::codex_account::binding_snapshot(&bound);
        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: Some("account-b"),
            },
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![1_893_456_600],
                soonest_reset_epoch: None,
            },
            |_| panic!("continuation must wait for the account switch"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::AccountSwitchQueued);
        assert_eq!(view_for_record(&context, &bound).state, "switching_account");
        assert_eq!(
            crate::codex_account::view_for_record(
                &crate::load_session_record(&context, &record.id).unwrap()
            )
            .next
            .and_then(|next| next.account)
            .as_deref(),
            Some("account-b")
        );
        let applying = crate::codex_account::begin_next_apply(&context, &record.id, "runtime-1")
            .unwrap()
            .expect("automatic account switch must become drainable");
        let intent_id = applying.intent_id.as_deref().expect("automatic intent id");
        crate::codex_account::finish_next_apply(
            &context,
            &record.id,
            "runtime-1",
            "account-b",
            applying.revision,
            intent_id,
            Ok(()),
        )
        .unwrap();
        let switched = crate::load_session_record(&context, &record.id).unwrap();
        let switched_view = crate::codex_account::view_for_record(&switched);
        assert_eq!(
            switched_view.effective_account.as_deref(),
            Some("account-b")
        );
        assert_eq!(
            switched_view.selection_source.as_deref(),
            Some("auto_failover")
        );
        let switched_binding = crate::codex_account::binding_snapshot(&switched);
        let healthy = UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        };
        let mut submissions = 0;
        let resumed = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &switched_binding,
                failover_account: None,
            },
            1_893_456_001,
            &healthy,
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(resumed, TickOutcome::Resumed);
        assert_eq!(submissions, 1);
        assert_eq!(
            tick_for_runtime_and_binding(
                &context,
                &record.id,
                "runtime-1",
                RuntimeBindingTick {
                    binding: &switched_binding,
                    failover_account: None,
                },
                1_893_456_002,
                &healthy,
                |_| {
                    submissions += 1;
                    Ok(())
                },
            )
            .unwrap(),
            TickOutcome::Unchanged
        );
        assert_eq!(submissions, 1);
    }

    #[test]
    fn account_failover_uses_the_structured_rejection_when_percentage_usage_is_open() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let binding = crate::codex_account::binding_snapshot(&record);

        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: Some("account-b"),
            },
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("continuation must wait for the account switch"),
        )
        .unwrap();

        assert_eq!(outcome, TickOutcome::AccountSwitchQueued);
        assert_eq!(
            view_for_record(&context, &record).state,
            "switching_account"
        );
        assert_eq!(
            crate::codex_account::view_for_record(
                &crate::load_session_record(&context, &record.id).unwrap()
            )
            .next
            .and_then(|next| next.account)
            .as_deref(),
            Some("account-b")
        );
    }

    #[test]
    fn account_switch_that_wins_the_failure_race_resumes_on_the_new_open_binding() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
        for (event_id, kind) in [
            ("codex-next-start", "turn_started"),
            ("codex-next-failed", "turn_failed"),
        ] {
            activity::ingest_event(
                &context,
                &record.id,
                serde_json::from_value(json!({
                    "schema_version": crate::activity::TURN_EVENT_VERSION,
                    "event_id": event_id,
                    "runtime_id": "runtime-1",
                    "provider": "codex",
                    "provider_turn_id": "turn-2",
                    "kind": kind,
                    "confidence": "authoritative"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let blocked_revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;
        let revision = crate::codex_account::begin_switch_binding(
            &context,
            &record.id,
            "runtime-1",
            "account-b",
        )
        .unwrap();
        crate::codex_account::finish_binding(
            &context,
            &record.id,
            "runtime-1",
            "account-b",
            revision,
            Ok(()),
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-2".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let switched = crate::load_session_record(&context, &record.id).unwrap();
        let binding = crate::codex_account::binding_snapshot(&switched);
        assert!(
            failover_selection_request(&context, &record.id, "runtime-1", &binding, false)
                .unwrap()
                .is_none(),
            "an open, newly selected binding must bypass broker discovery"
        );
        let mut submissions = 0;

        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: Some("account-c"),
            },
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(outcome, TickOutcome::Resumed);
        assert_eq!(submissions, 1);
        let account = crate::codex_account::view_for_record(
            &crate::load_session_record(&context, &record.id).unwrap(),
        );
        assert_eq!(account.effective_account.as_deref(), Some("account-b"));
        assert!(account.next.is_none());
    }

    #[test]
    fn same_account_rebind_does_not_authorize_continuation_after_rejection() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record, _) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        crate::codex_account::authorize_input_locked(&context, &mut record).unwrap();
        for (event_id, kind) in [
            ("same-account-start", "turn_started"),
            ("same-account-failed", "turn_failed"),
        ] {
            activity::ingest_event(
                &context,
                &record.id,
                serde_json::from_value(json!({
                    "schema_version": crate::activity::TURN_EVENT_VERSION,
                    "event_id": event_id,
                    "runtime_id": "runtime-1",
                    "provider": "codex",
                    "provider_turn_id": "turn-same-account",
                    "kind": kind,
                    "confidence": "authoritative"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let blocked_revision = activity::state_for_view(&context, &record)
            .unwrap()
            .revision;
        let revision = crate::codex_account::begin_switch_binding(
            &context,
            &record.id,
            "runtime-1",
            "account-a",
        )
        .unwrap();
        crate::codex_account::finish_binding(
            &context,
            &record.id,
            "runtime-1",
            "account-a",
            revision,
            Ok(()),
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-same-account".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let rebound = crate::load_session_record(&context, &record.id).unwrap();
        let binding = crate::codex_account::binding_snapshot(&rebound);
        let mut submissions = 0;

        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: None,
            },
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(outcome, TickOutcome::Retrying);
        assert_eq!(submissions, 0);
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("no_account_available")
        );
    }

    #[test]
    fn pre_upgrade_missing_binding_identity_never_switches_or_resumes() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let mut pre_upgrade = read_state(&context, &record.id, "ignored").unwrap();
        pre_upgrade.state = "switching_account".to_string();
        pre_upgrade.blocked_account = None;
        pre_upgrade.blocked_account_revision = None;
        write_state(&context, &record.id, &pre_upgrade).unwrap();
        let binding = crate::codex_account::binding_snapshot(&record);

        assert!(
            failover_selection_request(&context, &record.id, "runtime-1", &binding, false)
                .unwrap()
                .is_none(),
            "missing input-binding identity must skip broker discovery"
        );
        {
            let _guard = acquire_session_record_lock(&context, &record.id).unwrap();
            let mut current = load_session_record(&context, &record.id).unwrap();
            crate::codex_account::queue_auto_failover_locked(&context, &mut current, "account-b")
                .unwrap();
        }
        let expected = crate::codex_account::pending_auto_failover_apply(
            &load_session_record(&context, &record.id).unwrap(),
        )
        .unwrap()
        .expect("pre-upgrade automatic intent");
        assert!(
            !has_authoritative_usage_exhaustion_idle(&context, &record),
            "pre-upgrade state without blocked account identity cannot authorize an idle bypass"
        );
        assert!(
            crate::codex_account::begin_next_apply_if_unchanged(
                &context,
                &record.id,
                "runtime-1",
                &expected,
            )
            .unwrap()
            .is_none(),
            "restart recovery must not apply a pre-upgrade automatic intent without binding identity"
        );
        let mut submissions = 0;
        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: Some("account-b"),
            },
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(outcome, TickOutcome::Retrying);
        assert_eq!(submissions, 0);
        let account = crate::codex_account::view_for_record(
            &crate::load_session_record(&context, &record.id).unwrap(),
        );
        assert_eq!(account.selected_account.as_deref(), Some("account-a"));
        assert_eq!(
            account
                .next
                .as_ref()
                .and_then(|next| next.account.as_deref()),
            Some("account-b")
        );
        assert_eq!(account.next.as_ref().map(|next| next.state), Some("queued"));
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("state_unavailable")
        );
    }

    #[test]
    fn open_window_without_a_candidate_retries_before_its_future_reset() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let binding = crate::codex_account::binding_snapshot(&record);
        let base = 1_893_456_000;

        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: None,
            },
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: Some(base + 5 * 24 * 60 * 60),
            },
            |_| panic!("no candidate must not submit a continuation"),
        )
        .unwrap();

        assert_eq!(outcome, TickOutcome::Retrying);
        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(state.state, "transient_failure");
        assert_eq!(
            state.failure_reason.as_deref(),
            Some("no_account_available")
        );
        assert_eq!(
            state.next_check_at.as_deref().and_then(epoch_from_string),
            Some(base + RETRY_DELAYS_SECONDS[0])
        );
    }

    #[test]
    fn rejected_auto_continuation_preserves_the_failover_chain() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let binding = crate::codex_account::binding_snapshot(&record);
        assert_eq!(
            tick_for_runtime_and_binding(
                &context,
                &record.id,
                "runtime-1",
                RuntimeBindingTick {
                    binding: &binding,
                    failover_account: Some("account-b"),
                },
                1_893_456_000,
                &UsageSnapshot {
                    authoritative: true,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                },
                |_| panic!("continuation must wait for the account switch"),
            )
            .unwrap(),
            TickOutcome::AccountSwitchQueued
        );
        let applying = crate::codex_account::begin_next_apply(&context, &record.id, "runtime-1")
            .unwrap()
            .unwrap();
        crate::codex_account::finish_next_apply(
            &context,
            &record.id,
            "runtime-1",
            "account-b",
            applying.revision,
            applying.intent_id.as_deref().unwrap(),
            Ok(()),
        )
        .unwrap();
        let switched = crate::load_session_record(&context, &record.id).unwrap();
        let switched_binding = crate::codex_account::binding_snapshot(&switched);
        assert_eq!(
            tick_for_runtime_and_binding(
                &context,
                &record.id,
                "runtime-1",
                RuntimeBindingTick {
                    binding: &switched_binding,
                    failover_account: None,
                },
                1_893_456_001,
                &UsageSnapshot {
                    authoritative: true,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                },
                |_| Ok(()),
            )
            .unwrap(),
            TickOutcome::Resumed
        );
        for (event_id, kind) in [
            ("auto-continued-start", "turn_started"),
            ("auto-continued-failed", "turn_failed"),
        ] {
            activity::ingest_event(
                &context,
                &record.id,
                serde_json::from_value(json!({
                    "schema_version": crate::activity::TURN_EVENT_VERSION,
                    "event_id": event_id,
                    "runtime_id": "runtime-1",
                    "provider": "codex",
                    "provider_turn_id": "turn-2",
                    "kind": kind,
                    "confidence": "authoritative"
                }))
                .unwrap(),
            )
            .unwrap();
        }
        let failed_revision = activity::state_for_view(&context, &switched)
            .unwrap()
            .revision;
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-2".to_string(),
            failed_revision,
            "2030-01-01T00:00:02Z",
        )
        .unwrap();

        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(
            state.attempted_accounts,
            vec!["account-a".to_string(), "account-b".to_string()]
        );
    }

    #[test]
    fn manual_input_after_a_resume_starts_a_fresh_failover_chain() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, _) = seed_bound_codex_auto_resume(&tmp);
        let mut state = default_state("2030-01-01T00:00:00Z");
        state.enabled = true;
        state.recovery_policy = NEXT_ACCOUNT_THEN_RESUME_POLICY.to_string();
        state.state = "resumed".to_string();
        state.blocked_turn_id = Some("turn-1".to_string());
        state.blocked_revision = Some(4);
        state.blocked_account = Some("account-a".to_string());
        state.blocked_account_revision = Some(1);
        state.attempt = 2;
        state.ever_scheduled = true;
        state.attempted_accounts = vec!["account-a".to_string(), "account-b".to_string()];
        write_state(&context, &record.id, &state).unwrap();

        cancel_for_manual_input_locked(&context, &record.id, "2030-01-01T00:00:01Z").unwrap();

        let reset = read_state(&context, &record.id, "ignored").unwrap();
        assert!(reset.enabled);
        assert_eq!(reset.state, "enabled");
        assert!(reset.blocked_turn_id.is_none());
        assert!(reset.blocked_revision.is_none());
        assert!(reset.blocked_account.is_none());
        assert!(reset.blocked_account_revision.is_none());
        assert_eq!(reset.attempt, 0);
        assert!(!reset.ever_scheduled);
        assert!(reset.attempted_accounts.is_empty());
    }

    #[test]
    fn account_failover_without_a_candidate_waits_for_the_confirmed_reset() {
        let lock = GlobalStateLock::new();
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
            r#"["/configured/broker"]"#,
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record, blocked_revision) = seed_bound_codex_auto_resume(&tmp);
        set_enabled_with_policy(
            &context,
            &record.id,
            true,
            NEXT_ACCOUNT_THEN_RESUME_POLICY,
            "2030-01-01T00:00:00Z",
        )
        .unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            blocked_revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let binding = crate::codex_account::binding_snapshot(&record);
        let base = 1_893_456_000;
        let outcome = tick_for_runtime_and_binding(
            &context,
            &record.id,
            "runtime-1",
            RuntimeBindingTick {
                binding: &binding,
                failover_account: None,
            },
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 600],
                soonest_reset_epoch: None,
            },
            |_| panic!("no candidate must not submit a continuation"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Scheduled);
        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert!(state.enabled);
        assert_eq!(state.state, "scheduled");
        assert_eq!(
            state.failure_reason.as_deref(),
            Some("no_account_available")
        );
        assert!(epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap() >= base + 600);
    }

    #[test]
    fn account_handoff_rearm_refuses_replacement_incarnation_without_state_change() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record) = seed_session(&tmp);
        set_enabled(&context, &record.id, false, "2030-01-01T00:00:00Z")
            .expect("seed disabled auto-resume state");
        let auto_resume_path = path(&context, &record.id);
        let before = fs::read(&auto_resume_path).expect("seeded auto-resume bytes");

        record.runtime.as_mut().expect("runtime").launch_id = "runtime-2".to_string();
        record.runtime.as_mut().expect("runtime").generation = 2;
        record.updated_at = "2030-01-01T00:00:01Z".to_string();
        crate::write_session_record(&context, &record).expect("replacement session record");

        let error = rearm_usage_exhaustion_for_runtime(
            &context,
            &record.id,
            "runtime-1",
            "blocked-turn-old-runtime".to_string(),
            7,
            "2030-01-01T00:00:02Z",
        )
        .expect_err("replacement runtime must reject stale handoff rearm");
        assert_eq!(error.code(), "auto-resume-runtime-changed");
        assert_eq!(
            fs::read(&auto_resume_path).expect("auto-resume after rejected rearm"),
            before,
            "replacement runtime must not inherit any stale handoff auto-resume mutation"
        );
    }

    fn waiting_revision(context: &CliContext, record: &SessionRecord) -> u64 {
        let started = json!({
            "schema_version": "agent-session.turn-event.v1",
            "event_id": "start",
            "runtime_id": "runtime-1",
            "provider": "claude",
            "kind": "turn_started",
            "confidence": "observed"
        });
        activity::ingest_event(
            context,
            &record.id,
            serde_json::from_value(started).unwrap(),
        )
        .unwrap();
        let failed = json!({
            "schema_version": "agent-session.turn-event.v1",
            "event_id": "failed",
            "runtime_id": "runtime-1",
            "provider": "claude",
            "kind": "turn_failed",
            "failure_reason": "usage_exhausted",
            "confidence": "authoritative"
        });
        activity::ingest_event(context, &record.id, serde_json::from_value(failed).unwrap())
            .unwrap()
            .turn_state
            .revision
    }

    #[test]
    fn latest_exhausted_window_controls_wake_and_duplicate_ticks_submit_once() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();

        let base = 1_893_456_000;
        let first = tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 300, base + 900],
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit while blocked"),
        )
        .unwrap();
        assert_eq!(first, TickOutcome::Scheduled);

        let state = read_state(&context, &record.id, "ignored").unwrap();
        let wake = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        assert!(wake >= base + 900);

        let mut submissions = 0;
        let resumed = tick(
            &context,
            &record.id,
            wake,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(resumed, TickOutcome::Resumed);
        let duplicate = tick(
            &context,
            &record.id,
            wake + 1,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(duplicate, TickOutcome::Unchanged);
        assert_eq!(submissions, 1);
    }

    #[test]
    fn authoritative_arming_without_exhaustion_schedules_off_soonest_reset() {
        // Claude "session limit": armed by an authoritative rate-limit, but no
        // usage window reports `used_percent >= 100`. The scheduler must trust
        // the authoritative arming and wake at the soonest usage-window reset
        // instead of failing closed on `usage_window_not_exhausted`.
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();

        let base = 1_893_456_000;
        let scheduled = tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: Some(base + 300),
            },
            |_| panic!("must not submit before the fallback wake"),
        )
        .unwrap();
        assert_eq!(scheduled, TickOutcome::Scheduled);

        let state = read_state(&context, &record.id, "ignored").unwrap();
        let wake = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        assert!(wake >= base + 300);
        assert!(state.ever_scheduled);
        assert_eq!(state.fallback_schedules, 1);
        assert!(state.failure_reason.is_none());

        // At the fallback wake the session is still Waiting at the blocked
        // revision, so the continuation is submitted exactly once.
        let mut submissions = 0;
        let resumed = tick(
            &context,
            &record.id,
            wake,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: Some(base + 300),
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(resumed, TickOutcome::Resumed);
        assert_eq!(submissions, 1);
    }

    #[test]
    fn authoritative_arming_without_reset_epoch_schedules_a_probe() {
        // A structured provider rate-limit is authoritative even when the
        // usage helper cannot expose a reset timestamp. Keep the claim alive
        // and schedule a bounded probe instead of retrying into a permanent
        // terminal failure.
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();

        let base = 1_893_456_000;
        let outcome = tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit without a reset epoch"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Scheduled);
        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(state.state, "scheduled");
        let wake = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        assert!(wake >= base + UNKNOWN_RESET_PROBE_DELAYS_SECONDS[0]);
        assert!(wake <= base + UNKNOWN_RESET_PROBE_DELAYS_SECONDS[0] + 30);
        assert!(state.failure_reason.is_none());
        assert!(state.ever_scheduled);
        assert_eq!(state.fallback_schedules, 1);
    }

    #[test]
    fn codex_arming_without_reset_epoch_remains_fail_closed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record) = seed_session(&tmp);
        record.agent = "codex".to_string();
        let runtime = record.runtime.as_mut().unwrap();
        runtime.kind = "codex_app_server".to_string();
        runtime
            .extra
            .insert("codex_app_server_protocol".to_string(), json!("v2"));
        for (key, suffix) in [
            ("codex_app_server_socket", "sock"),
            ("codex_app_server_proxy", "proxy"),
            ("codex_app_server_thread_handoff", "thread"),
            ("codex_app_server_thread_attached", "attached"),
        ] {
            runtime.extra.insert(
                key.to_string(),
                json!(format!("/run/user/1000/agent-session/codex-test.{suffix}")),
            );
        }
        crate::write_session_record(&context, &record).unwrap();
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            0,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();

        let outcome = tick(
            &context,
            &record.id,
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("Codex must not probe without an authoritative open window"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Retrying);
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("usage_window_not_exhausted")
        );
    }

    #[test]
    fn fallback_scheduling_remains_live_across_rearms() {
        // A session that keeps re-arming because the provider still reports a
        // rate limit must retain its scheduled recovery instead of exhausting
        // a retry budget and becoming terminal.
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let base = 1_893_456_000;
        let snap = UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: false,
            exhausted_reset_epochs: Vec::new(),
            soonest_reset_epoch: None,
        };

        let expected_delays = [300, 900, 1_800, 3_600, 3_600, 3_600];
        for (i, expected_delay) in expected_delays.into_iter().enumerate() {
            let revision = waiting_revision(&context, &record);
            arm_usage_exhaustion(
                &context,
                &record.id,
                format!("turn-{i}"),
                revision,
                "2030-01-01T00:00:01Z",
            )
            .unwrap();
            let outcome = tick(&context, &record.id, base, &snap, |_| Ok(())).unwrap();
            assert_eq!(outcome, TickOutcome::Scheduled, "arm {i} should schedule");
            let wake = epoch_from_string(
                read_state(&context, &record.id, "ignored")
                    .unwrap()
                    .scheduled_at
                    .as_deref()
                    .unwrap(),
            )
            .unwrap();
            assert!(wake >= base + expected_delay);
            assert!(wake <= base + expected_delay + 30);
        }

        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(state.state, "scheduled");
        assert!(state.enabled);
        assert_eq!(state.fallback_schedules, 6);
        assert!(state.failure_reason.is_none());
        let wake = epoch_from_string(state.scheduled_at.as_deref().unwrap()).unwrap();
        assert!(wake >= base + 3_600);
        assert!(wake <= base + 3_630);
    }

    #[test]
    fn legacy_usage_window_terminal_is_recovered_only_while_still_eligible() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let mut pre_upgrade = read_state(&context, &record.id, "ignored").unwrap();
        pre_upgrade.enabled = false;
        pre_upgrade.state = "terminal_failure".to_string();
        pre_upgrade.failure_reason = Some("usage_window_not_exhausted".to_string());
        write_state(&context, &record.id, &pre_upgrade).unwrap();

        let base = 1_893_456_000;
        assert_eq!(
            pending_sessions(&context, base).unwrap().usage_ids,
            vec![record.id.clone()]
        );
        let outcome = tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("the recovered claim must wait before probing"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Scheduled);
        let recovered = read_state(&context, &record.id, "ignored").unwrap();
        assert!(recovered.enabled);
        assert_eq!(recovered.state, "scheduled");

        let mut stale = recovered;
        stale.enabled = false;
        stale.state = "terminal_failure".to_string();
        stale.scheduled_at = None;
        stale.failure_reason = Some("usage_window_not_exhausted".to_string());
        stale.blocked_revision = Some(revision.saturating_sub(1));
        write_state(&context, &record.id, &stale).unwrap();
        let outcome = tick(
            &context,
            &record.id,
            base + 1,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("a stale pre-upgrade claim must never submit"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::TerminalFailure);
        let stale = read_state(&context, &record.id, "ignored").unwrap();
        assert!(!stale.enabled);
        assert_eq!(
            stale.failure_reason.as_deref(),
            Some("session_state_changed")
        );
        assert!(
            pending_sessions(&context, base + 1)
                .unwrap()
                .usage_ids
                .is_empty()
        );
    }

    #[test]
    fn authoritative_open_usage_advances_only_an_existing_scheduled_claim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let base = 1_893_456_000;
        tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 900],
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit while blocked"),
        )
        .unwrap();

        assert!(wake_scheduled_if_usage_open(&context, &record.id, base + 1).unwrap());
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .scheduled_at
                .as_deref()
                .and_then(epoch_from_string),
            Some(base + 1)
        );

        cancel_for_manual_input_locked(&context, &record.id, "2030-01-01T00:00:02Z").unwrap();
        assert!(!wake_scheduled_if_usage_open(&context, &record.id, base + 2).unwrap());
        let cancelled = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(cancelled.state, "cancelled");
        assert!(!cancelled.enabled);
    }

    #[test]
    fn stale_runtime_cannot_wake_or_submit_for_a_same_id_replacement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let base = 1_893_456_000;
        assert_eq!(
            tick(
                &context,
                &record.id,
                base,
                &UsageSnapshot {
                    authoritative: true,
                    has_exhausted_windows: true,
                    exhausted_reset_epochs: vec![base + 900],
                    soonest_reset_epoch: None,
                },
                |_| panic!("must not submit while exhausted"),
            )
            .unwrap(),
            TickOutcome::Scheduled
        );
        let scheduled_before = read_state(&context, &record.id, "ignored")
            .unwrap()
            .scheduled_at;
        let mut replacement = record.clone();
        replacement.runtime.as_mut().unwrap().launch_id = "runtime-2".to_string();
        crate::write_session_record(&context, &replacement).unwrap();

        assert!(
            !wake_scheduled_if_usage_open_for_runtime(&context, &record.id, "runtime-1", base + 1,)
                .unwrap()
        );
        let mut submissions = 0;
        assert_eq!(
            tick_for_runtime(
                &context,
                &record.id,
                "runtime-1",
                base + 901,
                &UsageSnapshot {
                    authoritative: true,
                    has_exhausted_windows: false,
                    exhausted_reset_epochs: Vec::new(),
                    soonest_reset_epoch: None,
                },
                |_| {
                    submissions += 1;
                    Ok(())
                },
            )
            .unwrap(),
            TickOutcome::Unchanged
        );
        assert_eq!(submissions, 0);
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .scheduled_at,
            scheduled_before
        );
    }

    #[test]
    fn public_v1_view_serializes_the_documented_state_and_reason_allowlists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_, record) = seed_session(&tmp);
        let states = [
            "disabled",
            "enabled",
            "armed",
            "scheduled",
            "switching_account",
            "checking",
            "resumed",
            "cancelled",
            "transient_failure",
            "terminal_failure",
        ];
        let reasons = [
            "state_unavailable",
            "manual_input",
            "usage_unavailable",
            "usage_window_not_exhausted",
            "exhausted_reset_unavailable",
            "session_state_changed",
            "submission_outcome_unknown",
            "provider_unsupported",
            "account_switch",
            "no_account_available",
            "account_switch_failed",
            "scheduler_error",
            "control_unavailable",
            "account_changed",
            "capacity_retry_exhausted",
        ];
        for state in states {
            let mut durable = default_state("2030-01-01T00:00:00Z");
            durable.state = state.to_string();
            durable.enabled = state != "disabled";
            durable.scheduled_at =
                (state == "scheduled").then(|| "2030-01-01T00:05:00Z".to_string());
            let value = serde_json::to_value(view(&record, durable)).unwrap();
            assert_eq!(value["schema_version"], AUTO_RESUME_SCHEMA_VERSION);
            assert_eq!(value["state"], state);
            assert_eq!(value.get("scheduled_at").is_some(), state == "scheduled");
            assert!(value.get("recovery_cause").is_none());
        }
        for reason in reasons {
            let mut durable = default_state("2030-01-01T00:00:00Z");
            durable.state = "terminal_failure".to_string();
            durable.failure_reason = Some(reason.to_string());
            let value = serde_json::to_value(view(&record, durable)).unwrap();
            assert_eq!(value["failure_reason"], reason);
        }
    }

    #[test]
    fn cancellation_wins_before_wake_and_restart_discovers_pending_state() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            2,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        assert_eq!(
            pending_sessions(&context, 1_893_456_000).unwrap().usage_ids,
            vec![record.id.clone()]
        );
        cancel(&context, &record.id, "2030-01-01T00:00:02Z").unwrap();
        assert_eq!(
            pending_sessions(&context, 1_893_456_000).unwrap(),
            PendingSessions::default()
        );
        let outcome = tick(
            &context,
            &record.id,
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("cancelled state must not submit"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Unchanged);
    }

    #[test]
    fn malformed_state_is_isolated_from_healthy_pending_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            1,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();

        let mut corrupt = record.clone();
        corrupt.id = "claude-corrupt".to_string();
        corrupt.tmux_session = "hs-claude-corrupt".to_string();
        corrupt.created_at = "2030-01-01T00:00:02Z".to_string();
        corrupt.runtime.as_mut().unwrap().tmux_session = corrupt.tmux_session.clone();
        corrupt.runtime.as_mut().unwrap().launch_id = "runtime-corrupt".to_string();
        fs::create_dir_all(session_dir(&context, &corrupt.id)).unwrap();
        crate::write_session_record(&context, &corrupt).unwrap();
        fs::write(path(&context, &corrupt.id), b"not-json").unwrap();

        let pending = pending_sessions(&context, 1_893_456_000).unwrap();
        assert_eq!(pending.usage_ids, vec![record.id]);
        assert_eq!(pending.error_codes, vec!["auto-resume-state-invalid"]);
    }

    #[test]
    fn future_schedule_is_not_due_and_non_authoritative_usage_never_submits() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let base = 1_893_456_000;
        tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 300],
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit while blocked"),
        )
        .unwrap();
        let wake = read_state(&context, &record.id, "ignored")
            .unwrap()
            .scheduled_at
            .as_deref()
            .and_then(epoch_from_string)
            .unwrap();
        assert!(
            pending_sessions(&context, wake - 1)
                .unwrap()
                .usage_ids
                .is_empty()
        );
        assert_eq!(
            pending_sessions(&context, wake).unwrap().usage_ids,
            vec![record.id.clone()]
        );

        let outcome = tick(
            &context,
            &record.id,
            wake,
            &UsageSnapshot {
                authoritative: false,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("stale or policy-blocked usage must never submit"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Retrying);
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("usage_unavailable")
        );
    }

    #[test]
    fn exhausted_window_without_reset_never_authorizes_submission() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let base = 1_893_456_000;
        tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 60],
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit while blocked"),
        )
        .unwrap();
        let wake = read_state(&context, &record.id, "ignored")
            .unwrap()
            .scheduled_at
            .as_deref()
            .and_then(epoch_from_string)
            .unwrap();
        let outcome = tick(
            &context,
            &record.id,
            wake,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("an exhausted window without reset must never submit"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::Retrying);
        assert_eq!(
            read_state(&context, &record.id, "ignored")
                .unwrap()
                .failure_reason
                .as_deref(),
            Some("exhausted_reset_unavailable")
        );
    }

    #[test]
    fn restart_never_replays_an_unconfirmed_submission_claim() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let mut state = read_state(&context, &record.id, "ignored").unwrap();
        state.state = "checking".to_string();
        state.ever_scheduled = true;
        write_state(&context, &record.id, &state).unwrap();
        assert_eq!(
            pending_sessions(&context, 1_893_456_000)
                .unwrap()
                .recovery_ids,
            vec![record.id.clone()]
        );

        let outcome = tick(
            &context,
            &record.id,
            1_893_456_000,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| panic!("an unconfirmed durable claim must never be replayed"),
        )
        .unwrap();
        assert_eq!(outcome, TickOutcome::TerminalFailure);
        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(
            state.failure_reason.as_deref(),
            Some("submission_outcome_unknown")
        );
    }

    #[test]
    fn submission_error_after_claim_is_terminal_and_never_replayed() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, record) = seed_session(&tmp);
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();
        let revision = waiting_revision(&context, &record);
        arm_usage_exhaustion(
            &context,
            &record.id,
            "turn-1".to_string(),
            revision,
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let base = 1_893_456_000;
        tick(
            &context,
            &record.id,
            base,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: true,
                exhausted_reset_epochs: vec![base + 60],
                soonest_reset_epoch: None,
            },
            |_| panic!("must not submit while blocked"),
        )
        .unwrap();
        let wake = read_state(&context, &record.id, "ignored")
            .unwrap()
            .scheduled_at
            .as_deref()
            .and_then(epoch_from_string)
            .unwrap();

        let mut submissions = 0;
        let failed = tick(
            &context,
            &record.id,
            wake,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Err(CliError::runtime("injected-partial-send", "injected", None))
            },
        )
        .unwrap();
        assert_eq!(failed, TickOutcome::TerminalFailure);
        let replay = tick(
            &context,
            &record.id,
            wake + 600,
            &UsageSnapshot {
                authoritative: true,
                has_exhausted_windows: false,
                exhausted_reset_epochs: Vec::new(),
                soonest_reset_epoch: None,
            },
            |_| {
                submissions += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(replay, TickOutcome::Unchanged);
        assert_eq!(submissions, 1);
        let state = read_state(&context, &record.id, "ignored").unwrap();
        assert_eq!(state.state, "terminal_failure");
        assert_eq!(
            state.failure_reason.as_deref(),
            Some("submission_outcome_unknown")
        );
    }

    #[test]
    fn app_server_backed_codex_can_enable_auto_resume() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record) = seed_session(&tmp);
        record.agent = "codex".to_string();
        let runtime = record.runtime.as_mut().unwrap();
        runtime.kind = "codex_app_server".to_string();
        runtime
            .extra
            .insert("codex_app_server_protocol".to_string(), json!("v2"));
        runtime.extra.insert(
            "codex_app_server_socket".to_string(),
            json!("/run/user/1000/agent-session/codex-test.sock"),
        );
        runtime.extra.insert(
            "codex_app_server_proxy".to_string(),
            json!("/run/user/1000/agent-session/codex-test.proxy"),
        );
        runtime.extra.insert(
            "codex_app_server_thread_handoff".to_string(),
            json!("/run/user/1000/agent-session/codex-test.thread"),
        );
        runtime.extra.insert(
            "codex_app_server_thread_attached".to_string(),
            json!("/run/user/1000/agent-session/codex-test.attached"),
        );
        crate::write_session_record(&context, &record).unwrap();

        let view = set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z")
            .expect("a capability-probed app-server Codex runtime should be supported");
        assert!(view.supported);
        assert!(view.enabled);
        assert_eq!(view.state, "enabled");
    }

    #[test]
    fn profiled_app_server_codex_requires_explicit_auto_resume_support() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (_, mut record) = seed_session(&tmp);
        record.agent = "codex".to_string();
        {
            let runtime = record.runtime.as_mut().unwrap();
            runtime.kind = "codex_app_server".to_string();
            runtime
                .extra
                .insert("agent_profile".to_string(), json!("codex-custom"));
            runtime
                .extra
                .insert("codex_app_server_protocol".to_string(), json!("v2"));
            for (key, suffix) in [
                ("codex_app_server_socket", "sock"),
                ("codex_app_server_proxy", "proxy"),
                ("codex_app_server_thread_handoff", "thread"),
                ("codex_app_server_thread_attached", "attached"),
            ] {
                runtime.extra.insert(
                    key.to_string(),
                    json!(format!(
                        "/run/user/1000/agent-session/codex-profile.{suffix}"
                    )),
                );
            }
        }

        assert!(!supported(&record));
        record.runtime.as_mut().unwrap().extra.insert(
            "agent_profile_auto_resume_supported".to_string(),
            json!("invalid"),
        );
        assert!(!supported(&record));
        record.runtime.as_mut().unwrap().extra.insert(
            "agent_profile_auto_resume_supported".to_string(),
            json!(false),
        );
        assert!(!supported(&record));
        record.runtime.as_mut().unwrap().extra.insert(
            "agent_profile_auto_resume_supported".to_string(),
            json!(true),
        );
        assert!(supported(&record));
    }

    #[test]
    fn projection_loss_disables_enabled_runtime_and_blocks_reenable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let (context, mut record) = seed_session(&tmp);
        record.agent = "codex".to_string();
        let runtime = record.runtime.as_mut().unwrap();
        runtime.kind = "codex_app_server".to_string();
        runtime
            .extra
            .insert("codex_app_server_protocol".to_string(), json!("v2"));
        for (key, suffix) in [
            ("codex_app_server_socket", "sock"),
            ("codex_app_server_proxy", "proxy"),
            ("codex_app_server_thread_handoff", "thread"),
            ("codex_app_server_thread_attached", "attached"),
        ] {
            runtime.extra.insert(
                key.to_string(),
                json!(format!("/run/user/1000/agent-session/codex-test.{suffix}")),
            );
        }
        crate::write_session_record(&context, &record).unwrap();
        set_enabled(&context, &record.id, true, "2030-01-01T00:00:00Z").unwrap();

        fail_closed_projection_for_runtime(
            &context,
            &record.id,
            "runtime-1",
            "2030-01-01T00:00:01Z",
        )
        .unwrap();
        let view = view_for_record(&context, &record);
        assert!(!view.enabled);
        assert_eq!(view.state, "terminal_failure");
        assert_eq!(view.failure_reason.as_deref(), Some("state_unavailable"));
        let error = set_enabled(&context, &record.id, true, "2030-01-01T00:00:02Z").unwrap_err();
        assert_eq!(error.code(), "auto-resume-state-unavailable");
        let disabled = set_enabled(&context, &record.id, false, "2030-01-01T00:00:03Z").unwrap();
        assert!(!disabled.enabled);
        let error = set_enabled(&context, &record.id, true, "2030-01-01T00:00:04Z").unwrap_err();
        assert_eq!(error.code(), "auto-resume-state-unavailable");
        let cancelled = cancel(&context, &record.id, "2030-01-01T00:00:05Z").unwrap();
        assert!(!cancelled.enabled);
        let error = set_enabled(&context, &record.id, true, "2030-01-01T00:00:06Z").unwrap_err();
        assert_eq!(error.code(), "auto-resume-state-unavailable");
    }
}

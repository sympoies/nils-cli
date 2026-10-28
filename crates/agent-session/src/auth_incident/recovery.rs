//! Same-account Claude recovery, durably fenced by incident and runtime.
//! A committed `resuming` attempt is never blindly repeated after a crash.
use super::*;
use crate::claude_account::generation::{self, Publication};
use crate::cli::SpecialKey;
use crate::coordination::auth_effects::UnknownOperation;

const WAIT_SECONDS: i64 = 120;
const MAX_POLLS: u32 = 8;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub schema_version: String,
    pub operation_id: String,
    pub state: String,
    pub started_at_epoch: i64,
    pub deadline_epoch: i64,
    pub polls: u32,
    pub last_reason: Option<String>,
    pub provider_session_id: String,
    pub baseline: Option<Publication>,
    pub replacement: Option<Publication>,
    pub new_runtime_incarnation: Option<String>,
    pub new_runtime_generation: Option<u64>,
    pub reread: String,
    pub unknown_in_flight_effects: Vec<UnknownOperation>,
    pub unknown_operation_count: usize,
    pub untracked_effects: String,
    pub replayed: bool,
}

pub(super) fn active(receipt: &Receipt) -> bool {
    matches!(
        receipt.state.as_str(),
        "waiting_generation" | "stopping" | "stopped" | "resuming" | "awaiting_ready"
    )
}

/// A receipt that no longer fences the replacement runtime.
pub(super) fn settled(receipt: &Receipt) -> bool {
    matches!(
        receipt.state.as_str(),
        "runtime_resumed" | "credentials_refreshed"
    )
}

pub(super) fn settle_receipt(receipt: &mut Receipt) {
    if !settled(receipt) {
        receipt.state = "credentials_refreshed".into();
    }
}

/// Whether this incident's same-account recovery receipt fences the runtime.
pub(super) fn fences(incident: &AuthIncident, record: &SessionRecord) -> bool {
    incident.same_account_recovery.as_ref().is_some_and(|r| {
        !settled(r)
            && r.new_runtime_incarnation.as_deref()
                == record.runtime.as_ref().map(|r| r.launch_id.as_str())
            || active(r)
                && record.runtime.as_ref().is_some_and(|runtime| {
                    runtime.generation == incident.runtime_generation.saturating_add(1)
                })
    })
}

/// A later `credentials_refreshed` settlement of this runtime's incident
/// supersedes the failed turn it followed, never a newer failure.
fn failure_settled(store: &Store, record: &SessionRecord, completed_at: &str) -> bool {
    let Ok(failed_at) = completed_at.parse::<jiff::Timestamp>() else {
        return false;
    };
    store.incidents.iter().any(|i| {
        same_runtime(i, record)
            && i.status == "recovered"
            && i.recovery_result.as_deref() == Some("credentials_refreshed")
            && i.recovery_observed_at
                .as_deref()
                .and_then(|at| at.parse::<jiff::Timestamp>().ok())
                .is_some_and(|at| at >= failed_at)
    })
}

/// The caller holds the session lock through input. Incident observation uses
/// that same lock, so quarantine starts at the committed failure boundary.
pub(crate) fn ensure_input_allowed(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    super::failover::ensure_input_allowed(context, record)?;
    if record.agent != "claude" {
        return Ok(());
    }
    let store = read(context, record)?;
    let failed = store
        .incidents
        .iter()
        .any(|i| same_runtime(i, record) && i.status == "auth_failed");
    let pending = store.incidents.iter().any(|i| fences(i, record));
    let failed_activity = crate::activity::state_for_view(context, record).is_some_and(|s| {
        s.last_turn.is_some_and(|t| {
            t.provider_failure_kind() == Some("authentication")
                && !failure_settled(&store, record, &t.completed_at)
        })
    });
    if failed || pending || failed_activity {
        return Err(CliError::runtime(
            "auth-quarantined",
            "the Claude runtime is quarantined after authentication failure",
            Some(serde_json::json!({"id":record.id,"retryable":false,
                "stage":"admission","pasted":false,"submitted":false})),
        ));
    }
    Ok(())
}

pub(crate) fn guard_input(
    context: &CliContext,
    record: &SessionRecord,
    text: Option<&str>,
    keys: &[SpecialKey],
) -> Result<(), CliError> {
    if text.is_some()
        || keys
            .iter()
            .any(|k| matches!(k, SpecialKey::Enter | SpecialKey::Printable(_)))
    {
        ensure_input_allowed(context, record)?;
    }
    Ok(())
}

fn save_receipt(
    context: &CliContext,
    record: &SessionRecord,
    store: &mut Store,
    index: usize,
    receipt: &Receipt,
) -> Result<(), CliError> {
    store.incidents[index].same_account_recovery = Some(receipt.clone());
    save(context, record, store)
}

fn blocked(receipt: &mut Receipt, reason: &str) {
    receipt.state = "blocked".into();
    receipt.last_reason = Some(reason.into());
}

fn account_matches(record: &SessionRecord, account: &str) -> bool {
    crate::claude_account::config_dir_for_runtime(record).is_some()
        && !record.extra.contains_key("claude_account_next")
        && crate::claude_account::view_for_record(record)
            .and_then(|v| v.selected_account)
            .as_deref()
            == Some(account)
}

fn conversation(record: &SessionRecord) -> Option<&str> {
    record
        .provider_resume
        .as_ref()
        .filter(|p| p.provider == "claude")
        .map(|p| p.session_id.as_str())
}

pub(crate) fn tick(
    context: &CliContext,
    expected: &SessionRecord,
    tmux: &Path,
    now: i64,
) -> Result<(), CliError> {
    if expected.agent != "claude" {
        return Ok(());
    }
    // A second daemon can observe the same incident, but cannot own its effects.
    let _operation_lock = crate::coordination::lock_private_store(
        context,
        &format!("auth-recovery-{}.lock", expected.id),
    )?;
    let record_lock = crate::acquire_session_record_lock_timed(
        context,
        &expected.id,
        Duration::from_millis(250),
    )?;
    let mut record = crate::load_session_record(context, &expected.id)?;
    crate::ensure_same_session_identity(expected, &record)?;
    let mut store = read(context, &record)?;
    let Some(index) = store.incidents.iter().rposition(|i| {
        same_runtime(i, &record) && i.status == "auth_failed"
            || i.same_account_recovery.as_ref().is_some_and(active)
    }) else {
        return Ok(());
    };
    let incident = store.incidents[index].clone();
    let mut receipt = incident
        .same_account_recovery
        .clone()
        .unwrap_or_else(|| Receipt {
            schema_version: "agent-session.claude-auth-recovery.v1".into(),
            operation_id: uuid::Uuid::new_v4().to_string(),
            state: "waiting_generation".into(),
            started_at_epoch: now,
            deadline_epoch: now.saturating_add(WAIT_SECONDS),
            polls: 0,
            last_reason: None,
            provider_session_id: conversation(&record).unwrap_or_default().into(),
            baseline: generation::baseline(&record),
            replacement: None,
            new_runtime_incarnation: None,
            new_runtime_generation: None,
            reread: "unsupported".into(),
            unknown_in_flight_effects: vec![],
            unknown_operation_count: 0,
            untracked_effects: "unknown".into(),
            replayed: false,
        });
    if !active(&receipt) {
        return Ok(());
    }
    // Commit identity and deadline before any broker/lifecycle effect.
    save_receipt(context, &record, &mut store, index, &receipt)?;
    let account = incident.account_nickname.as_deref().unwrap_or_default();
    if crate::validate_resume_metadata(&record).is_err() {
        blocked(&mut receipt, "resume_metadata_invalid");
    } else if conversation(&record) != Some(receipt.provider_session_id.as_str())
        || !account_matches(&record, account)
    {
        blocked(&mut receipt, "binding_or_conversation_changed");
    } else if same_runtime(&incident, &record) && incident.status != "auth_failed" {
        if incident.status == "recovered" && incident.recovery_result.as_deref() == Some("healthy")
        {
            receipt.state = "recovered_in_place".into();
            receipt.last_reason = None;
        } else {
            blocked(&mut receipt, "incident_settled");
        }
    } else if !same_runtime(&incident, &record) {
        // Reconcile a launch already persisted before its controller crashed.
        let resumed = record
            .runtime
            .as_ref()
            .is_some_and(|r| r.generation == incident.runtime_generation.saturating_add(1))
            && matches!(receipt.state.as_str(), "resuming" | "awaiting_ready")
            && generation::baseline(&record)
                .zip(receipt.baseline.as_ref())
                .is_some_and(|(new, old)| new.newer_than(old));
        if resumed {
            receipt.new_runtime_incarnation = record.runtime.as_ref().map(|r| r.launch_id.clone());
            receipt.new_runtime_generation = record.runtime.as_ref().map(|r| r.generation);
            receipt.state = "awaiting_ready".into();
        } else {
            blocked(&mut receipt, "runtime_superseded");
        }
    } else if receipt.state == "resuming" {
        // A launch may have happened without a durable new runtime identity.
        blocked(&mut receipt, "resume_outcome_unknown");
    } else if receipt
        .baseline
        .as_ref()
        .is_none_or(|b| b.account != account)
    {
        blocked(&mut receipt, "launch_generation_missing");
    } else if receipt.state == "waiting_generation" {
        if now < receipt.started_at_epoch
            || now >= receipt.deadline_epoch
            || receipt.polls >= MAX_POLLS
        {
            blocked(&mut receipt, "publication_deadline");
        } else {
            receipt.polls += 1;
            match generation::published(account) {
                Ok(Some(p)) if p.newer_than(receipt.baseline.as_ref().unwrap()) => {
                    receipt.replacement = Some(p);
                    receipt.state = "stopping".into();
                    receipt.last_reason = None;
                }
                Ok(_) => receipt.last_reason = Some("newer_publication_pending".into()),
                Err(e)
                    if e.details().and_then(|d| d["reason"].as_str())
                        == Some("capability_missing") =>
                {
                    blocked(&mut receipt, "capability_missing")
                }
                Err(_) => receipt.last_reason = Some("publication_unavailable".into()),
            }
        }
    }
    save_receipt(context, &record, &mut store, index, &receipt)?;
    if matches!(receipt.state.as_str(), "stopping" | "stopped") {
        let result = (|| {
            // Revalidate publication before stopping, including crash re-entry.
            let p = generation::published(account)?.ok_or_else(error)?;
            if !p.newer_than(receipt.baseline.as_ref().ok_or_else(error)?) {
                return Err(error());
            }
            if receipt
                .replacement
                .as_ref()
                .is_none_or(|old| p != *old && !p.newer_than(old))
            {
                return Err(error());
            }
            receipt.replacement = Some(p);
            let _fence = crate::restart::auth_recovery::fence(context, &record)?;
            crate::auto_resume::cancel_for_manual_input_locked(
                context,
                &record.id,
                &jiff::Timestamp::now().to_string(),
            )?;
            crate::stop_session_runtime_locked(context, &mut record, tmux)?;
            // The process is stopped before taking the final operation snapshot.
            if receipt.state == "stopping" {
                (
                    receipt.unknown_operation_count,
                    receipt.unknown_in_flight_effects,
                ) = crate::coordination::auth_effects::snapshot(context, &record)?;
            }
            receipt.state = "stopped".into();
            save_receipt(context, &record, &mut store, index, &receipt)?;
            crate::restart::auth_recovery::retire(context, &record, tmux)?;
            receipt.state = "resuming".into();
            save_receipt(context, &record, &mut store, index, &receipt)?;
            // Existing lifecycle verifies stopped identity, rematerializes the
            // bound account, and launches only canonical provider resume args.
            crate::resume_session_locked(context, record.clone(), tmux)?;
            record = crate::load_session_record(context, &record.id)?;
            receipt.new_runtime_incarnation = record.runtime.as_ref().map(|r| r.launch_id.clone());
            receipt.new_runtime_generation = record.runtime.as_ref().map(|r| r.generation);
            if !account_matches(&record, account)
                || conversation(&record) != Some(receipt.provider_session_id.as_str())
                || !generation::baseline(&record).is_some_and(|new| {
                    new.newer_than(receipt.baseline.as_ref().unwrap())
                        && receipt
                            .replacement
                            .as_ref()
                            .is_some_and(|p| new == *p || new.newer_than(p))
                })
                || record.runtime.as_ref().is_none_or(|r| {
                    r.launch_id == incident.runtime_incarnation
                        || r.generation != incident.runtime_generation.saturating_add(1)
                })
            {
                return Err(error());
            }
            receipt.replacement = generation::baseline(&record);
            receipt.state = "awaiting_ready".into();
            Ok(())
        })();
        if let Err(cause) = result {
            if let Ok(current) = crate::load_session_record(context, &record.id) {
                if !same_runtime(&incident, &current) {
                    receipt.new_runtime_incarnation =
                        current.runtime.as_ref().map(|r| r.launch_id.clone());
                    receipt.new_runtime_generation = current.runtime.as_ref().map(|r| r.generation);
                }
                record = current;
            }
            blocked(&mut receipt, cause.code());
        }
        save_receipt(context, &record, &mut store, index, &receipt)?;
    }
    drop(record_lock);
    if receipt.state == "awaiting_ready" {
        let ready = crate::restart::auth_recovery::ready(context, &record, tmux);
        let _lock = crate::acquire_session_record_lock(context, &record.id)?;
        let current = crate::load_session_record(context, &record.id)?;
        let mut store = read(context, &current)?;
        if !runtime_matches(&record, &current) {
            blocked(&mut receipt, "runtime_superseded");
        } else if ready.is_err() {
            blocked(&mut receipt, "runtime_not_ready");
        } else {
            receipt.state = "runtime_resumed".into();
        }
        // Reloading preserves incidents committed by the replacement runtime.
        if let Some(index) = store
            .incidents
            .iter()
            .position(|i| i.incident_id == incident.incident_id)
        {
            save_receipt(context, &current, &mut store, index, &receipt)?;
        }
    }
    Ok(())
}

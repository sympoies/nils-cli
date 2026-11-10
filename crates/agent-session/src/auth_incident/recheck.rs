//! Same-account credential preflight for operator and scheduled Claude rechecks.
//! Records `credentials_refreshed` only when the credentials are usable again.
use super::*;
use crate::claude_account::generation;

pub(super) fn refused(record: &SessionRecord, reason: &str) -> CliError {
    CliError::data(
        "claude-account-recheck-refused",
        "the account credentials could not be confirmed; the authentication quarantine was left unchanged",
        Some(serde_json::json!({"id":record.id,"reason":reason})),
    )
}

/// Incidents that currently fence input for this runtime: each clause mirrors
/// one input guard, so an incident the guard no longer enforces is not owned.
pub(super) fn quarantining(incident: &AuthIncident, record: &SessionRecord) -> bool {
    let current = same_runtime(incident, record);
    current && incident.status == "auth_failed"
        || incident
            .failover
            .as_ref()
            .is_some_and(|r| r.fences(current, record))
        || recovery::fences(incident, record)
}

/// Settle every incident quarantining `record` after `account`, the account
/// the session is already bound to, passes the credential preflight. A session
/// that is not quarantined is left untouched. An operation the controller owns
/// is never overridden, and a refused preflight leaves the store unchanged.
pub(crate) fn settle(
    context: &CliContext,
    record: &SessionRecord,
    account: &str,
) -> Result<(), CliError> {
    let mut store = read(context, record)?;
    let owned: Vec<usize> = (0..store.incidents.len())
        .filter(|&i| quarantining(&store.incidents[i], record))
        .collect();
    if owned.is_empty() {
        return Ok(());
    }
    if owned.iter().any(|&i| {
        let incident = &store.incidents[i];
        incident
            .failover
            .as_ref()
            .is_some_and(failover::Receipt::in_flight)
            || incident
                .same_account_recovery
                .as_ref()
                .is_some_and(recovery::active)
    }) {
        return Err(refused(record, "recovery_in_progress"));
    }
    crate::claude_account::materialize(account).map_err(|e| refused(record, e.code()))?;
    // Materialization is never evidence of repair: when the failed launch's
    // publication is known, a newer one must exist for the same account.
    let baseline = owned.iter().find_map(|&i| {
        store.incidents[i]
            .same_account_recovery
            .as_ref()
            .and_then(|r| r.baseline.clone())
    });
    if let Some(baseline) = baseline
        && !generation::published(account)
            .ok()
            .flatten()
            .is_some_and(|published| published.newer_than(&baseline))
    {
        return Err(refused(record, "credentials_not_republished"));
    }
    let at = jiff::Timestamp::now().to_string();
    for i in owned {
        let incident = &mut store.incidents[i];
        if let Some(r) = incident.failover.as_mut().filter(|r| r.quarantines()) {
            r.mark_credentials_refreshed();
        }
        if let Some(r) = incident.same_account_recovery.as_mut() {
            recovery::settle_receipt(r);
        }
        incident.status = "recovered".into();
        incident.settle(PATH_CREDENTIAL_RECHECK, "credentials_refreshed", &at);
        incident.recovery_notification = Notification::default();
    }
    save(context, record, &store)
}

pub(crate) struct CodexSnapshot {
    account: String,
    incident_ids: Vec<String>,
    failure_key: Option<String>,
}

/// Snapshot only incidents belonging to the selected account before a queued
/// Codex repair begins. Later incidents must never be settled by this apply.
pub(crate) fn codex_snapshot(
    context: &CliContext,
    id: &str,
    launch: &str,
) -> Result<Option<CodexSnapshot>, CliError> {
    let _lock = crate::acquire_session_record_lock(context, id)?;
    let record = crate::load_session_record(context, id)?;
    if record.agent != "codex"
        || record.runtime.as_ref().map(|r| r.launch_id.as_str()) != Some(launch)
    {
        return Ok(None);
    }
    let Some(account) = crate::codex_account::selected_account(&record) else {
        return Ok(None);
    };
    let store = read(context, &record)?;
    let ids = store
        .incidents
        .iter()
        .filter(|i| {
            quarantining(i, &record)
                && i.account_nickname.as_deref() == Some(account.as_str())
                && !i
                    .failover
                    .as_ref()
                    .is_some_and(failover::Receipt::in_flight)
                && !i
                    .same_account_recovery
                    .as_ref()
                    .is_some_and(recovery::active)
        })
        .map(|i| i.incident_id.clone())
        .collect::<Vec<_>>();
    Ok((!ids.is_empty()).then_some(CodexSnapshot {
        account,
        incident_ids: ids,
        failure_key: store.last_failure_key,
    }))
}

/// A confirmed live Codex bind settles only the incidents captured before it.
pub(crate) fn settle_codex(
    context: &CliContext,
    id: &str,
    launch: &str,
    account: &str,
    revision: u64,
    snapshot: CodexSnapshot,
) -> Result<(), CliError> {
    if snapshot.account != account {
        return Ok(());
    }
    let _lock = crate::acquire_session_record_lock(context, id)?;
    let record = crate::load_session_record(context, id)?;
    let view = crate::codex_account::view_for_record(&record);
    if record.runtime.as_ref().map(|r| r.launch_id.as_str()) != Some(launch)
        || view.state != "bound"
        || view.selected_account.as_deref() != Some(account)
        || view.applied_runtime_id.as_deref() != Some(launch)
        || view.revision != revision
    {
        return Ok(());
    }
    let mut store = read(context, &record)?;
    if store.last_failure_key != snapshot.failure_key {
        return Err(CliError::runtime(
            "codex-account-recheck-superseded",
            "a newer authentication failure occurred during account recheck",
            None,
        ));
    }
    let at = jiff::Timestamp::now().to_string();
    let mut changed = false;
    for incident in &mut store.incidents {
        if !snapshot.incident_ids.contains(&incident.incident_id)
            || !quarantining(incident, &record)
            || incident.account_nickname.as_deref() != Some(account)
            || incident
                .failover
                .as_ref()
                .is_some_and(failover::Receipt::in_flight)
        {
            continue;
        }
        if let Some(r) = incident.failover.as_mut().filter(|r| r.quarantines()) {
            r.mark_credentials_refreshed();
        }
        incident.status = "recovered".into();
        incident.settle(PATH_CREDENTIAL_RECHECK, "credentials_refreshed", &at);
        incident.recovery_notification = Notification::default();
        changed = true;
    }
    if changed {
        save(context, &record, &store)?;
    }
    Ok(())
}

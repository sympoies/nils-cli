//! Operator-initiated settlement of a Claude authentication quarantine.
//! A model turn is the only automatic exit, and none can be admitted while
//! quarantined, so a same-account switch re-runs the credential preflight and
//! records `credentials_refreshed` only when the credentials are usable again.
use super::*;
use crate::claude_account::generation;

fn refused(record: &SessionRecord, reason: &str) -> CliError {
    CliError::data(
        "claude-account-recheck-refused",
        "the account credentials could not be confirmed; the authentication quarantine was left unchanged",
        Some(serde_json::json!({"id":record.id,"reason":reason})),
    )
}

/// Incidents that currently fence input for this runtime, by the same
/// ownership rules `ensure_input_allowed` applies.
fn quarantining(incident: &AuthIncident, record: &SessionRecord) -> bool {
    let owned = same_runtime(incident, record)
        || incident
            .failover
            .as_ref()
            .is_some_and(|r| r.replacement_matches(record))
        || incident.same_account_recovery.as_ref().is_some_and(|r| {
            r.new_runtime_incarnation.as_deref()
                == record.runtime.as_ref().map(|v| v.launch_id.as_str())
        });
    owned
        && (incident.status == "auth_failed"
            || incident
                .failover
                .as_ref()
                .is_some_and(failover::Receipt::quarantines)
            || incident
                .same_account_recovery
                .as_ref()
                .is_some_and(|r| !recovery::settled(r)))
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
        incident.recovery_result = Some("credentials_refreshed".into());
        incident.recovery_observed_at = Some(at.clone());
        incident.recovery_notification = Notification::default();
    }
    save(context, record, &store)
}

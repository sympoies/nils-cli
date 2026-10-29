//! One policy-gated account switch per uninterrupted authentication failure chain.
//! Durable stage transitions precede effects; ambiguous stages are never replayed.
use super::*;
mod preflight;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    schema_version: String,
    operation_id: String,
    state: String,
    reason: Option<String>,
    source_account: String,
    source_binding_revision: u64,
    target_account: Option<String>,
    provider_session_id: String,
    started_at_epoch: i64,
    deadline_epoch: i64,
    proof: Option<preflight::Proof>,
    new_runtime_incarnation: Option<String>,
    new_runtime_generation: Option<u64>,
    unknown_operation_count: usize,
    unknown_in_flight_effects: Vec<crate::coordination::auth_effects::UnknownOperation>,
    untracked_effects: String,
    replayed: bool,
}

impl Receipt {
    pub(super) fn retains_chain(&self) -> bool {
        unsettled(self) || self.target_account.is_some() && self.state != "healthy"
    }
    pub(super) fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub(super) fn replacement_matches(&self, record: &SessionRecord) -> bool {
        self.new_runtime_incarnation.as_deref()
            == record.runtime.as_ref().map(|r| r.launch_id.as_str())
            && self.new_runtime_generation == record.runtime.as_ref().map(|r| r.generation)
    }
    /// Fences `record` as `ensure_input_allowed` does: unsettled, and owned by
    /// the failed runtime (`current`) or by its recorded replacement launch.
    pub(super) fn fences(&self, current: bool, record: &SessionRecord) -> bool {
        self.quarantines()
            && (current
                || self.new_runtime_incarnation.as_deref()
                    == record.runtime.as_ref().map(|r| r.launch_id.as_str()))
    }
    /// Still fences input: neither a switch nor a healthy turn settled it.
    pub(super) fn quarantines(&self) -> bool {
        !matches!(
            self.state.as_str(),
            "account_switched" | "healthy" | "credentials_refreshed"
        )
    }
    /// A controller-owned operation whose effects are not yet decided.
    pub(super) fn in_flight(&self) -> bool {
        unsettled(self)
    }
    pub(super) fn mark_healthy(&mut self) {
        self.state = "healthy".into();
        self.reason = None;
    }
    pub(super) fn mark_credentials_refreshed(&mut self) {
        self.state = "credentials_refreshed".into();
        self.reason = None;
    }
}

fn account(record: &SessionRecord) -> (Option<String>, u64) {
    if record.agent == "codex" {
        let view = crate::codex_account::view_for_record(record);
        (view.selected_account, view.revision)
    } else {
        crate::claude_account::view_for_record(record)
            .map_or((None, 0), |v| (v.selected_account, v.revision))
    }
}

fn conversation(record: &SessionRecord) -> Option<&str> {
    record
        .provider_resume
        .as_ref()
        .filter(|p| p.provider == record.agent && !p.session_id.is_empty())
        .map(|p| p.session_id.as_str())
}

fn unsettled(r: &Receipt) -> bool {
    matches!(r.state.as_str(), "candidate_preflight" | "switching")
}

pub(crate) fn ensure_input_allowed(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    if !matches!(record.agent.as_str(), "codex" | "claude") {
        return Ok(());
    }
    let store = read(context, record)?;
    let codex_binding =
        (record.agent == "codex").then(|| crate::codex_account::view_for_record(record));
    if store.incidents.iter().any(|i| {
        i.failover.as_ref().is_some_and(|r| {
            let superseded = codex_binding.as_ref().is_some_and(|v| {
                v.state == "bound"
                    && v.next.is_none()
                    && v.revision > r.source_binding_revision
                    && v.selected_account.as_deref() != Some(r.source_account.as_str())
                    && v.applied_runtime_id.as_deref()
                        == record.runtime.as_ref().map(|r| r.launch_id.as_str())
            });
            r.state != "account_switched"
                && r.state != "healthy"
                && r.state != "credentials_refreshed"
                && !superseded
                && (same_runtime(i, record)
                    || r.new_runtime_incarnation.as_deref()
                        == record.runtime.as_ref().map(|r| r.launch_id.as_str()))
        })
    }) {
        return Err(CliError::runtime(
            "auth-quarantined",
            "the runtime is quarantined during authentication recovery",
            Some(serde_json::json!({"id":record.id,"retryable":false,
                "stage":"admission","pasted":false,"submitted":false})),
        ));
    }
    Ok(())
}

/// Official refresh and model-health observations can arrive on a newer
/// incident than the refusal. Settle the owning receipts without reissuing
/// their effects or treating refreshed credentials as a healthy model turn.
pub(super) fn observe_result(
    store: &mut Store,
    record: &SessionRecord,
    result: &str,
    at: &str,
) -> bool {
    if !matches!(result, "healthy" | "credentials_refreshed") {
        return false;
    }
    let Ok(at_epoch) = at.parse::<jiff::Timestamp>() else {
        return false;
    };
    let selected = account(record).0;
    let mut changed = false;
    for incident in &mut store.incidents {
        if incident.provider != record.agent
            || !incident
                .observed_at
                .parse::<jiff::Timestamp>()
                .is_ok_and(|observed| at_epoch >= observed)
        {
            continue;
        }
        let runtime_matches = same_runtime(incident, record);
        let Some(r) = incident.failover.as_mut() else {
            continue;
        };
        if conversation(record) != Some(r.provider_session_id.as_str()) || r.state == "healthy" {
            continue;
        }
        if result == "credentials_refreshed"
            && (!(runtime_matches || r.replacement_matches(record))
                || !(selected.as_deref() == Some(r.source_account.as_str())
                    || selected == r.target_account))
        {
            continue;
        }
        if r.state == result {
            continue;
        }
        if result == "healthy" {
            r.mark_healthy();
        } else {
            r.mark_credentials_refreshed();
        }
        incident.status = "recovered".into();
        incident.settle(observed_path(result), result, at);
        incident.recovery_notification = Notification::default();
        changed = true;
    }
    changed
}

fn blocked(r: &mut Receipt, reason: &str) {
    r.state = "blocked_no_credential".into();
    r.reason = Some(reason.into());
}

fn persist(
    context: &CliContext,
    record: &SessionRecord,
    store: &mut Store,
    index: usize,
    r: &Receipt,
) -> Result<(), CliError> {
    let incident = &mut store.incidents[index];
    incident.failover = Some(r.clone());
    if r.state == "blocked_no_credential" || r.state == "account_switched" {
        // A changed result gets its own stable outbox identity. The original
        // same-account result may already have been delivered.
        if incident.recovery_result.as_deref() != Some(r.state.as_str()) {
            incident.settle(
                PATH_CONTROLLER,
                &r.state,
                &jiff::Timestamp::now().to_string(),
            );
            incident.recovery_notification = Notification::default();
        }
        if r.state == "account_switched" {
            incident.status = "account_switched".into();
        }
    }
    save(context, record, store)
}

fn same_account_finished(incident: &AuthIncident, record: &SessionRecord, now: i64) -> bool {
    if record.agent == "claude" {
        incident.same_account_recovery.as_ref().is_some_and(|r| {
            r.state == "blocked"
                && matches!(
                    r.last_reason.as_deref(),
                    Some(
                        "publication_deadline" | "capability_missing" | "launch_generation_missing"
                    )
                )
        })
    } else {
        incident
            .observed_at
            .parse::<jiff::Timestamp>()
            .is_ok_and(|at| now >= at.as_second().saturating_add(120))
    }
}

pub(crate) fn tick(
    context: &CliContext,
    expected: &SessionRecord,
    tmux: &Path,
    now: i64,
) -> Result<(), CliError> {
    if !matches!(expected.agent.as_str(), "codex" | "claude") {
        return Ok(());
    }
    let _operation = crate::coordination::lock_private_store(
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
    let Some(index) = store
        .incidents
        .iter()
        .rposition(|i| i.failover.as_ref().is_some_and(unsettled))
        .or_else(|| {
            store.incidents.iter().rposition(|i| {
                same_runtime(i, &record)
                    && matches!(i.status.as_str(), "auth_failed" | "recovery_failed")
            })
        })
    else {
        return Ok(());
    };
    let incident = store.incidents[index].clone();
    if let Some(mut receipt) = incident.failover.clone() {
        if !unsettled(&receipt) {
            return Ok(());
        }
        reconcile(&record, &incident, &mut receipt, now);
        return persist(context, &record, &mut store, index, &receipt);
    }
    if !runtime_matches(expected, &record) || !same_account_finished(&incident, &record, now) {
        return Ok(());
    }
    let (source, revision) = account(&record);
    let mut receipt = Receipt {
        schema_version: "agent-session.auth-failover.v1".into(),
        operation_id: uuid::Uuid::new_v4().to_string(),
        state: "candidate_preflight".into(),
        reason: None,
        source_account: incident.account_nickname.clone().unwrap_or_default(),
        source_binding_revision: revision,
        target_account: None,
        provider_session_id: conversation(&record).unwrap_or_default().into(),
        started_at_epoch: now,
        deadline_epoch: now.saturating_add(30),
        proof: None,
        new_runtime_incarnation: None,
        new_runtime_generation: None,
        unknown_operation_count: 0,
        unknown_in_flight_effects: vec![],
        untracked_effects: "unknown".into(),
        replayed: false,
    };
    persist(context, &record, &mut store, index, &receipt)?;
    // A healthy model turn, not refreshed tokens or runtime readiness, is the
    // only event that ends an authentication failure chain.
    let prior_switch = store.incidents[..index]
        .iter()
        .rev()
        .take_while(|i| i.recovery_result.as_deref() != Some("healthy"))
        .any(|i| {
            i.failover
                .as_ref()
                .is_some_and(|r| r.target_account.is_some())
        });
    let refusal = if prior_switch {
        Some("consecutive_fault")
    } else if source.as_deref() != Some(receipt.source_account.as_str()) || source.is_none() {
        Some("binding_changed")
    } else if conversation(&record).is_none() {
        Some("conversation_missing")
    } else if record.agent == "codex"
        && !crate::codex_app_server::auth_refresh::failover_safe(context, &record)
    {
        Some("same_account_outcome_unknown")
    } else {
        None
    };
    if let Some(reason) = refusal {
        blocked(&mut receipt, reason);
        return persist(context, &record, &mut store, index, &receipt);
    }
    let broker = if record.agent == "codex" {
        &crate::codex_account::BROKER
    } else {
        &crate::claude_account::BROKER
    };
    let selection = preflight::select(
        broker,
        &record,
        &receipt.operation_id,
        &receipt.source_account,
    );
    let (target, proof) = match selection {
        Ok(selected) => selected,
        Err(reason) => {
            blocked(&mut receipt, reason);
            return persist(context, &record, &mut store, index, &receipt);
        }
    };
    if !proof.fresh(jiff::Timestamp::now().as_second()) {
        blocked(&mut receipt, "capacity_unconfirmed_or_stale");
        return persist(context, &record, &mut store, index, &receipt);
    }
    receipt.target_account = Some(target.clone());
    receipt.proof = Some(proof);
    (
        receipt.unknown_operation_count,
        receipt.unknown_in_flight_effects,
    ) = crate::coordination::auth_effects::snapshot(context, &record)?;
    receipt.state = "switching".into();
    persist(context, &record, &mut store, index, &receipt)?;
    let result = if record.agent == "codex" {
        crate::auto_resume::cancel_for_account_switch_locked(
            context,
            &record.id,
            &jiff::Timestamp::now().to_string(),
        )
        .and_then(|()| {
            crate::codex_account::queue_auth_failover_locked(
                context,
                &mut record,
                &target,
                &receipt.operation_id,
            )
        })
    } else {
        switch_claude(context, &mut record, tmux, &target, &mut receipt)
    };
    if let Err(cause) = result {
        blocked(&mut receipt, cause.code());
    }
    // Stop/resume can persist lifecycle evidence even on failure.
    record = crate::load_session_record(context, &record.id)?;
    if !same_runtime(&incident, &record) {
        receipt.new_runtime_incarnation = record.runtime.as_ref().map(|r| r.launch_id.clone());
        receipt.new_runtime_generation = record.runtime.as_ref().map(|r| r.generation);
    }
    persist(context, &record, &mut store, index, &receipt)?;
    drop(record_lock);
    if record.agent == "claude" && receipt.state == "switching" {
        let ready = crate::restart::auth_recovery::ready(context, &record, tmux);
        let _lock = crate::acquire_session_record_lock(context, &record.id)?;
        let current = crate::load_session_record(context, &record.id)?;
        let mut store = read(context, &current)?;
        if !runtime_matches(&record, &current) || ready.is_err() {
            blocked(&mut receipt, "runtime_not_ready");
        } else {
            reconcile(&current, &incident, &mut receipt, now);
        }
        if let Some(index) = store
            .incidents
            .iter()
            .position(|i| i.incident_id == incident.incident_id)
        {
            persist(context, &current, &mut store, index, &receipt)?;
        }
    }
    Ok(())
}

fn reconcile(record: &SessionRecord, incident: &AuthIncident, r: &mut Receipt, now: i64) {
    let (current, revision) = account(record);
    if conversation(record) != Some(r.provider_session_id.as_str()) {
        blocked(r, "conversation_changed");
    } else if r.state == "candidate_preflight" {
        blocked(r, "preflight_outcome_unknown");
    } else if current == r.target_account && revision > r.source_binding_revision {
        let ready = if record.agent == "codex" {
            let v = crate::codex_account::view_for_record(record);
            same_runtime(incident, record)
                && v.state == "bound"
                && v.applied_runtime_id.as_deref() == Some(incident.runtime_incarnation.as_str())
                && v.next.is_none()
        } else {
            r.new_runtime_incarnation.as_deref()
                == record.runtime.as_ref().map(|v| v.launch_id.as_str())
                && record
                    .runtime
                    .as_ref()
                    .is_some_and(|v| v.generation == incident.runtime_generation.saturating_add(1))
                && crate::startup_projection_for_view(record).is_some_and(|p| p.state == "ready")
        };
        if ready {
            r.state = "account_switched".into();
            r.reason = None;
        } else if now >= r.deadline_epoch {
            blocked(r, "switch_deadline");
        }
    } else if !same_runtime(incident, record) {
        blocked(r, "runtime_superseded");
    } else if now < r.started_at_epoch || now >= r.deadline_epoch {
        blocked(r, "switch_deadline");
    } else {
        let queued = if record.agent == "codex" {
            crate::codex_account::view_for_record(record)
                .next
                .and_then(|v| v.account)
        } else {
            crate::claude_account::queued_next_account(record)
        };
        if queued != r.target_account {
            blocked(r, "switch_outcome_unknown");
        }
    }
}

fn switch_claude(
    context: &CliContext,
    record: &mut SessionRecord,
    tmux: &Path,
    target: &str,
    receipt: &mut Receipt,
) -> Result<(), CliError> {
    crate::validate_resume_metadata(record)?;
    if crate::claude_account::has_queued_next(record) {
        return Err(CliError::runtime(
            "auth-failover-account-intent-present",
            "an account switch is already pending",
            None,
        ));
    }
    crate::claude_account::queue_next(record, target)?;
    crate::claude_account::preflight_next(record)?;
    if !receipt
        .proof
        .as_ref()
        .is_some_and(|p| p.fresh(jiff::Timestamp::now().as_second()))
    {
        return Err(CliError::runtime(
            "auth-failover-preflight-expired",
            "authentication failover preflight expired",
            None,
        ));
    }
    let _fence = crate::restart::auth_recovery::fence(context, record)?;
    crate::auto_resume::cancel_for_account_switch_locked(
        context,
        &record.id,
        &jiff::Timestamp::now().to_string(),
    )?;
    crate::write_session_record(context, record)?;
    crate::stop_session_runtime_locked(context, record, tmux)?;
    (
        receipt.unknown_operation_count,
        receipt.unknown_in_flight_effects,
    ) = crate::coordination::auth_effects::snapshot(context, record)?;
    // The existing explicit switch owns verified retirement, canonical resume,
    // new runtime fencing and failure projection.
    crate::session_account::resume_after_switch_stop(context, &record.id, tmux)?;
    Ok(())
}

/// The record and account-mutation locks are already held by the explicit
/// queued switch path. Manual intents have different identities and retain
/// their existing behavior.
pub(crate) fn pre_apply_codex(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    let Some(next) = crate::codex_account::next_account_identity(record)? else {
        return Ok(());
    };
    let mut store = read(context, record)?;
    let Some(index) = store.incidents.iter().rposition(|i| {
        i.failover
            .as_ref()
            .is_some_and(|r| next.intent_id.as_deref() == Some(r.operation_id.as_str()))
    }) else {
        return Ok(());
    };
    let incident = store.incidents[index].clone();
    let mut receipt = incident.failover.unwrap();
    let now = jiff::Timestamp::now().as_second();
    let refusal = if receipt.state != "switching" {
        Some("switch_not_admitted")
    } else if !same_runtime(&store.incidents[index], record)
        || conversation(record) != Some(receipt.provider_session_id.as_str())
        || account(record)
            != (
                Some(receipt.source_account.clone()),
                receipt.source_binding_revision,
            )
    {
        Some("binding_or_runtime_changed")
    } else if now < receipt.started_at_epoch || now >= receipt.deadline_epoch {
        Some("switch_deadline")
    } else {
        None
    };
    let decision = refusal.map_or_else(
        || {
            preflight::select(
                &crate::codex_account::BROKER,
                record,
                &receipt.operation_id,
                &receipt.source_account,
            )
        },
        Err,
    );
    let completed_at = jiff::Timestamp::now().as_second();
    match decision {
        Ok((target, proof))
            if Some(&target) == receipt.target_account.as_ref()
                && target == next.account
                && proof.fresh(completed_at) =>
        {
            receipt.proof = Some(proof);
            persist(context, record, &mut store, index, &receipt)
        }
        other => {
            blocked(&mut receipt, other.err().unwrap_or("candidate_changed"));
            persist(context, record, &mut store, index, &receipt)?;
            Err(CliError::runtime(
                "auth-failover-preflight-refused",
                "authentication failover no longer has fresh policy and capacity authorization",
                None,
            ))
        }
    }
}

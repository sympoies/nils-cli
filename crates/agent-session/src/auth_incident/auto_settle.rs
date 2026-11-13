//! Bounded daemon rechecks. This controller performs no lifecycle or input effects.
use super::*;

const OFFSETS: [i64; 3] = [30, 120, 600];

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    runtime_incarnation: String,
    runtime_generation: u64,
    started_at_epoch: i64,
    state: String,
    refusal: Option<String>,
    attempts: Vec<Attempt>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    attempted_at_epoch: i64,
    result: String,
}

impl Receipt {
    pub(super) fn requires_same_account_restart(&self) -> bool {
        self.state == "exhausted"
            && self.attempts.last().is_some_and(|attempt| {
                matches!(
                    attempt.result.as_str(),
                    "provider_verification_required" | "runtime_adoption_required"
                )
            })
    }
}

pub(super) fn matches(incident: &AuthIncident, record: &SessionRecord) -> bool {
    incident.auto_settle.as_ref().is_some_and(|receipt| {
        record.runtime.as_ref().is_some_and(|runtime| {
            receipt.runtime_incarnation == runtime.launch_id
                && receipt.runtime_generation == runtime.generation
        })
    })
}

fn exhaust(incident: &mut AuthIncident, now: i64) {
    incident.auto_settle.as_mut().unwrap().state = "exhausted".into();
    incident.settle(
        PATH_CONTROLLER,
        "recheck_exhausted",
        &jiff::Timestamp::from_second(now).unwrap().to_string(),
    );
    incident.recovery_notification = Notification::default();
}

pub(crate) fn tick(
    context: &CliContext,
    expected: &SessionRecord,
    now: i64,
) -> Result<(), CliError> {
    if expected.agent != "claude" {
        return Ok(());
    }
    let _lock = crate::acquire_session_record_lock_timed(
        context,
        &expected.id,
        Duration::from_millis(250),
    )?;
    let record = crate::load_session_record(context, &expected.id)?;
    if !runtime_matches(&record, expected) {
        return Err(recheck::refused(&record, "runtime_changed"));
    }
    crate::ensure_same_session_identity(expected, &record)?;
    let Some(runtime) = record.runtime.as_ref() else {
        return Ok(());
    };
    let mut store = read(context, &record)?;
    let before_reconcile = serde_json::to_vec(&store).map_err(|_| error())?;
    for incident in &mut store.incidents {
        if !recheck::quarantining(incident, &record) {
            let recovered = incident.status == "recovered";
            if let Some(receipt) = incident.auto_settle.as_mut()
                && receipt.state == "scheduled"
            {
                // An operator/model result, or a successful preflight saved
                // before daemon interruption, also terminates this schedule.
                // Keep `started` outcomes intact when their result is unknown.
                receipt.state = if recovered { "settled" } else { "cancelled" }.into();
            }
        }
    }
    if serde_json::to_vec(&store).map_err(|_| error())? != before_reconcile {
        save(context, &record, &store)?;
    }
    let Some(index) = store.incidents.iter().rposition(|incident| {
        incident.provider == "claude" && recheck::quarantining(incident, &record)
    }) else {
        return Ok(());
    };
    let before = serde_json::to_vec(&store).map_err(|_| error())?;
    let incident = &mut store.incidents[index];
    if !matches(incident, &record) {
        // A carried quarantine gets a schedule for the replacement runtime.
        // A daemon restart with the same runtime retains the consumed budget.
        let started = if same_runtime(incident, &record) {
            &incident.observed_at
        } else {
            &runtime.started_at
        };
        incident.auto_settle = Some(Receipt {
            runtime_incarnation: runtime.launch_id.clone(),
            runtime_generation: runtime.generation,
            started_at_epoch: started
                .parse::<jiff::Timestamp>()
                .map_or(now, |at| at.as_second()),
            state: "scheduled".into(),
            refusal: None,
            attempts: vec![],
        });
    }
    let selected =
        crate::claude_account::view_for_record(&record).and_then(|view| view.selected_account);
    let account = incident.account_nickname.clone();
    let refusal = if record.extra.contains_key("claude_account_next") {
        Some("next_account_queued")
    } else if account.is_none()
        || selected != account
        || crate::claude_account::config_dir_for_runtime(&record).is_none()
    {
        Some("account_changed")
    } else {
        None
    };
    if let Some(reason) = refusal {
        let receipt = incident.auto_settle.as_mut().unwrap();
        receipt.state = "cancelled".into();
        receipt.refusal = Some(reason.into());
        if serde_json::to_vec(&store).map_err(|_| error())? != before {
            save(context, &record, &store)?;
        }
        return Ok(());
    }
    let receipt = incident.auto_settle.as_mut().unwrap();
    if receipt.state != "scheduled" {
        if serde_json::to_vec(&store).map_err(|_| error())? != before {
            save(context, &record, &store)?;
        }
        return Ok(());
    }
    if receipt.attempts.len() >= OFFSETS.len() {
        // A crash after committing the last attempt consumes it, rather than
        // repeating an unbounded credential operation after daemon recovery.
        exhaust(incident, now);
        return save(context, &record, &store);
    }
    if now
        < receipt
            .started_at_epoch
            .saturating_add(OFFSETS[receipt.attempts.len()])
    {
        if serde_json::to_vec(&store).map_err(|_| error())? != before {
            save(context, &record, &store)?;
        }
        return Ok(());
    }
    receipt.attempts.push(Attempt {
        attempted_at_epoch: now,
        result: "started".into(),
    });
    save(context, &record, &store)?;
    // The same record lock used by observation and explicit account switching
    // remains held through preflight and settlement. Never bypass its fences.
    let result = recheck::settle(context, &record, account.as_deref().unwrap());
    let mut store = read(context, &record)?;
    let incident = &mut store.incidents[index];
    let receipt = incident.auto_settle.as_mut().unwrap();
    match result {
        Ok(()) => {
            receipt.attempts.last_mut().unwrap().result = "credentials_refreshed".into();
            receipt.state = "settled".into();
        }
        Err(error) => {
            receipt.attempts.last_mut().unwrap().result = match error
                .details()
                .and_then(|details| details["reason"].as_str())
            {
                Some("recovery_in_progress") => "recovery_in_progress",
                Some("credentials_not_republished") => "credentials_not_republished",
                Some("provider_verification_required") => "provider_verification_required",
                Some("runtime_adoption_required") => "runtime_adoption_required",
                _ if error.code() == "claude-account-recheck-refused" => {
                    "credential_preflight_refused"
                }
                _ => "recheck_unavailable",
            }
            .into();
            if receipt.attempts.len() == OFFSETS.len() {
                exhaust(incident, now);
            }
        }
    }
    save(context, &record, &store)
}

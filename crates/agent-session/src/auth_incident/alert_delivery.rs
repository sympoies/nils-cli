//! Daemon-independent deliverable authentication alerts and consumer receipts.
use super::*;
use std::collections::BTreeMap;

const FILE: &str = "auth-alerts.json";
const SCHEMA: &str = "agent-session.auth-alerts.v1";
const MAX_ALERTS: usize = 64;
const MAX_BYTES: usize = 128 * 1024;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Person,
    Watchdog,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Delivery {
    state: String,
    delivered_at: Option<String>,
    receipt: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Alert {
    alert_id: String,
    session_id: String,
    incident_id: String,
    runtime_incarnation: String,
    runtime_generation: u64,
    provider: String,
    phase: String,
    result: String,
    recovery_action: Option<String>,
    recorded_at: String,
    required_targets: Vec<Target>,
    delivery: BTreeMap<Target, Delivery>,
    state: String,
    delivered_at: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Alerts {
    schema_version: String,
    alerts: Vec<Alert>,
}

fn unavailable() -> CliError {
    CliError::runtime(
        "auth-alert-store-unavailable",
        "authentication alert delivery storage is unavailable",
        None,
    )
}

fn read_alerts(context: &CliContext, id: &str) -> Result<Alerts, CliError> {
    let bytes = match fs::read(session_dir(context, id).join(FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Alerts {
                schema_version: SCHEMA.into(),
                alerts: vec![],
            });
        }
        Err(_) => return Err(unavailable()),
    };
    if bytes.len() > MAX_BYTES {
        return Err(unavailable());
    }
    let alerts: Alerts = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if alerts.schema_version != SCHEMA || alerts.alerts.len() > MAX_ALERTS {
        return Err(unavailable());
    }
    Ok(alerts)
}

fn save_alerts(context: &CliContext, id: &str, alerts: &Alerts) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(alerts).map_err(|_| unavailable())?;
    if bytes.len() > MAX_BYTES {
        return Err(unavailable());
    }
    write_atomic(
        &session_dir(context, id).join(FILE),
        &bytes,
        SECRET_FILE_MODE,
    )
    .map_err(|_| unavailable())
}

fn targets() -> Result<Vec<Target>, CliError> {
    let configured = match std::env::var("AGENT_SESSION_AUTH_ALERT_TARGETS") {
        Ok(value) => value,
        Err(std::env::VarError::NotPresent) => "person".into(),
        Err(_) => String::new(),
    };
    match configured.as_str() {
        "person" => Ok(vec![Target::Person]),
        "person,watchdog" => Ok(vec![Target::Person, Target::Watchdog]),
        _ => Err(CliError::data(
            "auth-alert-targets-invalid",
            "authentication alert targets must be person or person,watchdog",
            None,
        )),
    }
}

pub(super) fn enqueue(
    context: &CliContext,
    incident: &AuthIncident,
    recovery: bool,
) -> Result<(), CliError> {
    let _lock = crate::acquire_session_record_lock_timed(
        context,
        &incident.session_id,
        Duration::from_millis(250),
    )?;
    let record = crate::load_session_record(context, &incident.session_id)?;
    // Delivery runs outside the record lock. Resolve its current obligation
    // before retaining an independently consumable alert from that snapshot.
    let store = read(context, &record)?;
    let Some(current) = store
        .incidents
        .iter()
        .find(|current| current.incident_id == incident.incident_id)
    else {
        return Ok(());
    };
    let notification = if recovery {
        &current.recovery_notification
    } else {
        &current.notification
    };
    if notification.delivered_at.is_some()
        || crate::lineage::effective_parent(&record).is_some()
        || recovery && current.recovery_result != incident.recovery_result
    {
        return Ok(());
    }
    // A missing current owner also fences the first unavailable attempt,
    // whose notification receipt has not yet been merged into the store.
    let incident = current;
    let result = if recovery {
        incident.recovery_result.as_deref().unwrap_or("unknown")
    } else {
        "authentication"
    };
    let alert_id = if recovery {
        key(&incident.incident_id, &format!("recovery:{result}"))
    } else {
        incident.incident_id.clone()
    };
    let mut alerts = read_alerts(context, &record.id)?;
    if alerts.alerts.iter().any(|alert| alert.alert_id == alert_id) {
        return Ok(());
    }
    if alerts.alerts.len() == MAX_ALERTS {
        let Some(index) = alerts
            .alerts
            .iter()
            .position(|alert| alert.state == "delivered")
        else {
            return Err(unavailable());
        };
        alerts.alerts.remove(index);
    }
    let required_targets = targets()?;
    let delivery = required_targets
        .iter()
        .map(|&target| {
            (
                target,
                Delivery {
                    state: "pending".into(),
                    delivered_at: None,
                    receipt: None,
                },
            )
        })
        .collect();
    alerts.alerts.push(Alert {
        alert_id,
        session_id: record.id.clone(),
        incident_id: incident.incident_id.clone(),
        runtime_incarnation: incident.runtime_incarnation.clone(),
        runtime_generation: incident.runtime_generation,
        provider: incident.provider.clone(),
        phase: if recovery { "recovery" } else { "incident" }.into(),
        result: result.into(),
        recovery_action: if recovery
            && matches!(
                result,
                "healthy" | "credentials_refreshed" | "account_switched" | "runtime_replaced"
            ) {
            None
        } else if recovery
            && result == "recheck_exhausted"
            && incident
                .auto_settle
                .as_ref()
                .is_some_and(auto_settle::Receipt::requires_same_account_restart)
        {
            Some("restart_same_account".into())
        } else {
            Some("same_account_credential_recheck".into())
        },
        recorded_at: jiff::Timestamp::now().to_string(),
        required_targets,
        delivery,
        state: "pending".into(),
        delivered_at: None,
    });
    save_alerts(context, &record.id, &alerts)
}

pub(crate) fn list(context: &CliContext, limit: usize) -> Result<Value, CliError> {
    let entries = match fs::read_dir(context.state_dir.join("sessions")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(serde_json::json!({"alerts":[], "truncated":false}));
        }
        Err(_) => return Err(unavailable()),
    };
    let mut ids = entries
        .map(|entry| {
            entry
                .map_err(|_| unavailable())?
                .file_name()
                .into_string()
                .map_err(|_| unavailable())
        })
        .collect::<Result<Vec<_>, _>>()?;
    ids.sort();
    let mut alerts = Vec::new();
    for id in ids {
        crate::validate_id(&id)?;
        let _lock =
            crate::acquire_session_record_lock_timed(context, &id, Duration::from_millis(250))?;
        alerts.extend(read_alerts(context, &id)?.alerts);
    }
    // Pending work is always first; delivered audit cannot starve a consumer.
    alerts.sort_by(|a, b| {
        (a.state == "delivered", &a.recorded_at, &a.alert_id).cmp(&(
            b.state == "delivered",
            &b.recorded_at,
            &b.alert_id,
        ))
    });
    let truncated = alerts.len() > limit;
    alerts.truncate(limit);
    Ok(serde_json::json!({"alerts":alerts, "truncated":truncated}))
}

pub(crate) fn acknowledge(
    context: &CliContext,
    id: &str,
    alert_id: &str,
    target: Target,
    proof: &str,
) -> Result<Alert, CliError> {
    crate::validate_id(id)?;
    if !proof.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }) {
        return Err(CliError::usage(
            "auth-alert-receipt-invalid",
            "delivery requires a lowercase SHA-256 receipt digest",
            None,
        ));
    }
    let _lock = crate::acquire_session_record_lock_timed(context, id, Duration::from_millis(250))?;
    crate::load_session_record(context, id)?;
    let mut alerts = read_alerts(context, id)?;
    let alert = alerts
        .alerts
        .iter_mut()
        .find(|alert| alert.alert_id == alert_id)
        .ok_or_else(|| {
            CliError::data(
                "auth-alert-not-found",
                "the authentication alert does not exist",
                None,
            )
        })?;
    let delivery = alert.delivery.get_mut(&target).ok_or_else(|| {
        CliError::data(
            "auth-alert-target-unconfigured",
            "the target is not required by this alert",
            None,
        )
    })?;
    if let Some(prior) = delivery.receipt.as_deref() {
        if prior != proof {
            return Err(CliError::data(
                "auth-alert-receipt-conflict",
                "the target already has a different delivery receipt",
                None,
            ));
        }
        return Ok(alert.clone());
    }
    let now = jiff::Timestamp::now().to_string();
    delivery.state = "delivered".into();
    delivery.delivered_at = Some(now.clone());
    delivery.receipt = Some(proof.into());
    if alert.required_targets.iter().all(|target| {
        alert
            .delivery
            .get(target)
            .is_some_and(|delivery| delivery.state == "delivered")
    }) {
        alert.state = "delivered".into();
        alert.delivered_at = Some(now);
    }
    let result = alert.clone();
    save_alerts(context, id, &alerts)?;
    Ok(result)
}

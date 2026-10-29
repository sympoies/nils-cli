//! Independent alert for an incident whose owner route is missing.
//!
//! A session with no effective parent cannot receive its own incident through
//! the mailbox. The alert goes to the daemon-independent observation spool
//! (`control-plane-observation-v1`) instead, so a consumer needs neither the
//! affected runtime nor the serve daemon. Recording it is not delivery: the
//! notification receipt keeps `delivered_at` null until a real owner receives
//! the mailbox message.

use nils_common::observation::{self, Component, Event, FieldError, Severity};
use serde::{Deserialize, Serialize};

use super::AuthIncident;
use crate::CliContext;

const ROUTE: &str = "observation_spool";
const RECORDED: &str = "recorded";
const UNAVAILABLE: &str = "unavailable";
const OWNER_ACTION: &str = "agent-session lineage adopt <CHILD> --by <SESSION>";

/// Outcome of the latest alert attempt for one notification.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    route: String,
    state: String,
    recorded_at: Option<String>,
}

impl Receipt {
    fn recorded(&self) -> bool {
        self.state == RECORDED
    }
}

/// Alert every undelivered notification that proved no owner route. A recorded
/// alert is never repeated; an unavailable spool is retried on the next tick.
pub(super) fn route_unowned(context: &CliContext, incidents: &mut [AuthIncident]) {
    for incident in incidents {
        for recovery in [false, true] {
            if recovery && incident.recovery_result.is_none() {
                continue;
            }
            let notification = if recovery {
                &incident.recovery_notification
            } else {
                &incident.notification
            };
            if notification.delivered_at.is_some()
                || notification.degraded_reason.as_deref() != Some("owner_unavailable")
                || notification
                    .escalation
                    .as_ref()
                    .is_some_and(Receipt::recorded)
            {
                continue;
            }
            let receipt = record(context, incident, recovery);
            let notification = if recovery {
                &mut incident.recovery_notification
            } else {
                &mut incident.notification
            };
            notification.escalation = Some(receipt);
        }
    }
}

fn record(context: &CliContext, incident: &AuthIncident, recovery: bool) -> Receipt {
    let now = jiff::Timestamp::now();
    let appended = event(incident, recovery, now.as_second())
        .map_err(|_| ())
        .and_then(|event| observation::append(&context.state_dir, &event).map_err(|_| ()))
        .is_ok();
    Receipt {
        route: ROUTE.into(),
        state: if appended { RECORDED } else { UNAVAILABLE }.into(),
        recorded_at: appended.then(|| now.to_string()),
    }
}

/// Fixed classification only. The correlation digest is the incident identity
/// (or, for a settlement, that identity bound to the fixed result), so a
/// consumer deduplicates repeats and joins the typed incident from list/board.
fn event(incident: &AuthIncident, recovery: bool, now: i64) -> Result<Event, FieldError> {
    let result = incident.recovery_result.as_deref().unwrap_or("unknown");
    let (code, severity, correlation) = if recovery {
        let severity = match result {
            "healthy" | "credentials_refreshed" => Severity::Info,
            _ => Severity::Warn,
        };
        let digest = super::key(&incident.incident_id, &format!("recovery:{result}"));
        ("auth-recovery-owner-unavailable", severity, digest)
    } else {
        (
            "auth-owner-unavailable",
            Severity::Error,
            incident.incident_id.clone(),
        )
    };
    let event = Event::new(
        Component::AgentSession,
        "auth-incident",
        code,
        severity,
        env!("CARGO_PKG_VERSION"),
        now,
    )?
    .with_provider(
        &incident.provider,
        if recovery {
            "ProviderAuthRecoveryResult"
        } else {
            "ProviderAuthIncident"
        },
    )?
    .with_disposition("owner-unavailable")?
    .with_runtime_generation(&incident.runtime_generation.to_string())?
    .with_correlation(&format!("sha256:{correlation}"))?;
    if recovery {
        Ok(event)
    } else {
        event.with_recovery_action(OWNER_ACTION)
    }
}

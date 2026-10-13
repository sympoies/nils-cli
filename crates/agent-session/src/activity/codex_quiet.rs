use super::*;

const QUIET_SECONDS: i64 = 300;
const REASON: &str = "codex_progress_quiet";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub(crate) struct QuietMetadata {
    pub(super) quiet_threshold_seconds: u64,
    pub(super) quiet_since: String,
    pub(super) quiet_signal: String,
}

pub(super) fn metadata(diagnostic: &ActivityDiagnosticView) -> Option<QuietMetadata> {
    metadata_fields(&diagnostic.reason, &diagnostic.extra)
}

fn metadata_fields(reason: &str, fields: &Map<String, Value>) -> Option<QuietMetadata> {
    if reason != REASON || fields.get("quiet_threshold_seconds")?.as_u64()? != QUIET_SECONDS as u64
    {
        return None;
    }
    let since = fields.get("quiet_since")?.as_str()?;
    if since.len() > 64 {
        return None;
    }
    let since = since.parse::<jiff::Timestamp>().ok()?.to_string();
    let signal = fields.get("quiet_signal")?.as_str()?;
    if !matches!(
        signal,
        "current_turn.last_progress_at" | "current_turn.started_at"
    ) {
        return None;
    }
    Some(QuietMetadata {
        quiet_threshold_seconds: QUIET_SECONDS as u64,
        quiet_since: since,
        quiet_signal: signal.to_string(),
    })
}

pub(crate) fn diagnostic_for_board(fields: &Map<String, Value>) -> Option<Value> {
    let quiet = metadata_fields(fields.get("reason")?.as_str()?, fields)?;
    let mut value = serde_json::to_value(quiet).ok()?;
    value["reason"] = json!(REASON);
    Some(value)
}

/// Age is an uncertainty signal, never a completion or input authorization.
pub(crate) fn notification_maybe_idle(state: &TurnState) -> bool {
    state.phase == TurnPhase::Waiting
        || (state.phase == TurnPhase::Unknown
            && state.source.confidence == Confidence::Inferred
            && state.diagnostic.as_ref().and_then(metadata).is_some())
}

/// Read-only projection also heals old snapshots on the first post-upgrade
/// read; polling does not rewrite the journal, revision, or progress clock.
pub(super) fn project(record: &SessionRecord, state: &mut TurnState, at: &str) {
    if record.agent != "codex" || state.phase != TurnPhase::Working {
        return;
    }
    let Some(current) = state
        .current_turn
        .as_ref()
        .filter(|turn| turn.attention.is_none())
    else {
        return;
    };
    let (since, signal) = current
        .last_progress_at
        .as_deref()
        .map(|at| (at, "current_turn.last_progress_at"))
        .unwrap_or((&current.started_at, "current_turn.started_at"));
    let (Ok(since), Ok(at)) = (
        since.parse::<jiff::Timestamp>(),
        at.parse::<jiff::Timestamp>(),
    ) else {
        return;
    };
    let Ok(deadline) = since.checked_add(jiff::SignedDuration::from_secs(QUIET_SECONDS)) else {
        return;
    };
    if at < deadline {
        return;
    }
    state.phase = TurnPhase::Unknown;
    state.phase_changed_at = deadline.to_string();
    state.source.confidence = Confidence::Inferred;
    state.diagnostic = Some(ActivityDiagnosticView {
        reason: REASON.into(),
        extra: Map::from_iter([
            ("quiet_threshold_seconds".into(), json!(QUIET_SECONDS)),
            ("quiet_since".into(), json!(since.to_string())),
            ("quiet_signal".into(), json!(signal)),
        ]),
    });
}

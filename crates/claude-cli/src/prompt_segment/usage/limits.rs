//! The per-model weekly windows and the "Limit resets" section of
//! `claude-cli usage`.
//!
//! Both come from the same usage response as the base windows, parsed by the
//! shared `diag rate-limits` readers (`rate_limits::scoped` and
//! `rate_limits::status`), so the two commands cannot disagree. Grant ids are
//! host-local and never leave this module; `next` marks the grant a
//! redemption would spend.

use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;

use super::{UsageWindow, usage_window};
use crate::prompt_segment::render;
use crate::rate_limits::scoped::parse_weekly_scoped;
use crate::rate_limits::status::{Grant, GrantedResets, SessionReset, parse_limit_resets};

const WEEKLY_MINUTES: i64 = 10_080;
const FULL_CLEARS: &str = "seven_day";

#[derive(Debug, Clone, Serialize)]
pub(super) struct UsageLimitResets {
    session: SessionResetView,
    grants: Vec<GrantView>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionResetView {
    state: SessionResetState,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_available_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionResetState {
    Available,
    /// The program block is `null` or missing from the response.
    NotOffered,
    NotAvailable,
}

#[derive(Debug, Clone, Serialize)]
struct GrantView {
    kind: GrantKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    resets_left: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    resets_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ends_at: Option<i64>,
    usable_now: bool,
    next: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum GrantKind {
    Full,
    Model,
}

/// One window per `weekly_scoped` limit: key `weekly_scoped:<model>`.
pub(super) fn scoped_windows(value: &Value, reference_epoch: Option<i64>) -> Vec<UsageWindow> {
    parse_weekly_scoped(value)
        .into_iter()
        .map(|limit| {
            let window = render::Window {
                used_percent: limit.used_percent,
                remaining_percent: render::remaining_percent(limit.used_percent),
                resets_at: limit.resets_at,
            };
            usage_window(
                &format!("weekly_scoped:{}", limit.model),
                &format!("Weekly ({})", limit.model),
                WEEKLY_MINUTES,
                &window,
                reference_epoch,
            )
        })
        .collect()
}

/// The normalized reset programs of a usage response body.
pub(super) fn limit_resets(value: &Value) -> UsageLimitResets {
    let parsed = parse_limit_resets(value);
    UsageLimitResets {
        session: session_view(parsed.juniper_tide.as_ref()),
        grants: parsed
            .cedar_ember
            .as_ref()
            .map(grant_views)
            .unwrap_or_default(),
    }
}

fn session_view(session: Option<&SessionReset>) -> SessionResetView {
    match session {
        None => SessionResetView {
            state: SessionResetState::NotOffered,
            reason: None,
            next_available_at: None,
        },
        Some(session) => SessionResetView {
            state: if session.available {
                SessionResetState::Available
            } else {
                SessionResetState::NotAvailable
            },
            reason: session.ineligible_reason.clone(),
            next_available_at: session.next_available_at,
        },
    }
}

fn grant_views(resets: &GrantedResets) -> Vec<GrantView> {
    resets
        .grants
        .iter()
        .map(|grant| GrantView {
            kind: grant_kind(grant),
            label: grant.label.clone(),
            resets_left: grant.resets_left,
            resets_total: grant.resets_total,
            ends_at: grant.ends_at,
            usable_now: grant.usable_now,
            next: resets.next_grant_id.as_deref() == Some(grant.id.as_str()),
        })
        .collect()
}

/// `full` when the grant clears the weekly limit, else a model-scoped grant.
/// The upstream `type` is not consulted: it does not tell the two apart.
fn grant_kind(grant: &Grant) -> GrantKind {
    if grant.clears.iter().any(|clears| clears == FULL_CLEARS) {
        GrantKind::Full
    } else {
        GrantKind::Model
    }
}

pub(super) fn render_text(resets: &UsageLimitResets) -> String {
    let mut lines = vec!["Limit resets".to_string()];
    lines.push(format!("  session: {}", session_text(&resets.session)));
    if resets.grants.is_empty() {
        lines.push("  grants: none".to_string());
    }
    lines.extend(
        resets
            .grants
            .iter()
            .map(|grant| format!("  grant: {}", grant_text(grant))),
    );
    lines.join("\n")
}

fn session_text(session: &SessionResetView) -> String {
    match session.state {
        SessionResetState::Available => "available".to_string(),
        SessionResetState::NotOffered => "not offered".to_string(),
        SessionResetState::NotAvailable => {
            let mut text = "not available".to_string();
            if let Some(reason) = &session.reason {
                text.push_str(&format!(" ({reason})"));
            }
            if let Some(next) = session.next_available_at {
                text.push_str(&format!(", next {}", time_text(next)));
            }
            text
        }
    }
}

fn grant_text(grant: &GrantView) -> String {
    let kind = match grant.kind {
        GrantKind::Full => "full",
        GrantKind::Model => "model",
    };
    let label = grant
        .label
        .as_deref()
        .map(|label| format!(" \"{label}\""))
        .unwrap_or_default();
    let count = match grant.resets_total {
        Some(total) => format!("{}/{total}", grant.resets_left),
        None => grant.resets_left.to_string(),
    };
    let mut parts = vec![format!("{kind}{label} {count} left")];
    if let Some(ends_at) = grant.ends_at {
        parts.push(format!("expires {}", time_text(ends_at)));
    }
    parts.push(if grant.usable_now {
        "usable now".to_string()
    } else {
        "not usable now".to_string()
    });
    if grant.next {
        parts.push("next".to_string());
    }
    parts.join(", ")
}

/// `YYYY-MM-DDTHH:MMZ` (UTC), so the text does not depend on the host zone.
fn time_text(epoch: i64) -> String {
    DateTime::from_timestamp(epoch, 0)
        .map(|time| time.format("%Y-%m-%dT%H:%MZ").to_string())
        .unwrap_or_else(|| epoch.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn an_offered_session_reset_is_available() {
        let resets = limit_resets(&json!({
            "juniper_tide": { "eligible": true, "available": true, "arm": "reset" }
        }));

        assert_eq!(resets.session.state, SessionResetState::Available);
        assert_eq!(session_text(&resets.session), "available");
    }
}

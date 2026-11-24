//! Presentation shared by the shell segment and rate-limit diagnostics.

use serde::Deserialize;
use serde_json::{Map, Value};

use super::status::{CEDAR_EMBER, GrantedResets};

pub(crate) const ULTRA: &str = "Ultra";

pub(crate) fn plan(oauth: &Map<String, Value>) -> Option<&'static str> {
    (oauth.get("subscriptionType").and_then(Value::as_str) == Some("max")
        && oauth
            .get("rateLimitTier")
            .and_then(Value::as_str)
            .is_some_and(|tier| tier.ends_with("_max_20x")))
    .then_some(ULTRA)
}

/// A grant can be pending even when its use requires reaching a rate limit.
pub(crate) fn remaining(resets: &GrantedResets) -> Option<i64> {
    if !resets.eligible {
        return Some(0);
    }
    let now = chrono::Utc::now().timestamp();
    resets
        .grants
        .iter()
        .filter(|grant| {
            !grant.paused
                && grant.starts_at.is_none_or(|start| start <= now)
                && grant.ends_at.is_none_or(|end| end > now)
        })
        .try_fold(0_i64, |sum, grant| {
            sum.checked_add(i64::try_from(grant.resets_left).ok()?)
        })
}

pub(super) fn diagnostic_remaining(limit_resets: Option<&Value>) -> Option<i64> {
    let cedar = GrantedResets::deserialize(limit_resets?.get(CEDAR_EMBER)?).ok()?;
    remaining(&cedar)
}

pub(crate) fn note(
    plan: Option<&str>,
    remaining: Option<i64>,
    show_unknown: bool,
) -> Option<String> {
    let resets = match remaining {
        Some(1) => Some("1 reset available".to_string()),
        Some(count) if count > 0 => Some(format!("{count} resets available")),
        None if show_unknown => Some("reset availability unknown".to_string()),
        _ => None,
    };
    match (plan, resets) {
        (Some(plan), Some(resets)) => Some(format!("{plan} · {resets}")),
        (Some(plan), None) => Some(plan.to_string()),
        (None, resets) => resets,
    }
}

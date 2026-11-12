//! Per-model weekly limits: the `weekly_scoped` entries of the usage
//! response's `limits[]`, their allowance from `weekly_scoped_shares[]`, and
//! their cache lines.

use nils_common::rate_limits::{RateLimitWindow, schema};
use nils_common::usage_time::reset_epoch_seconds_from_str;
use serde_json::{Number, Value};

use crate::prompt_segment::render::remaining_percent;

const KIND: &str = "weekly_scoped";
/// Result window label: `weekly_scoped:<model>`.
pub(super) const LABEL_PREFIX: &str = "weekly_scoped:";
const CACHE_LINE_PREFIX: &str = "scoped_window=";

/// One `weekly_scoped` limit, as read from the usage response.
pub(crate) struct ScopedLimit {
    pub model: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
    /// The matching `weekly_scoped_shares[]` allowance (`limit_index` is the
    /// entry's position in `limits[]`).
    pub allowance_percent_of_weekly: Option<Number>,
}

/// Every `weekly_scoped` entry of `limits[]` with a model name and a percent.
pub(crate) fn parse_weekly_scoped(value: &Value) -> Vec<ScopedLimit> {
    let usage = value.get("usage").unwrap_or(value);
    let Some(limits) = usage.get("limits").and_then(Value::as_array) else {
        return Vec::new();
    };
    let shares = usage
        .get("weekly_scoped_shares")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    limits
        .iter()
        .enumerate()
        .filter(|(_, limit)| limit.get("kind").and_then(Value::as_str) == Some(KIND))
        .filter_map(|(index, limit)| {
            let model = limit.pointer("/scope/model/display_name")?.as_str()?.trim();
            if model.is_empty() || model.chars().any(char::is_control) {
                return None;
            }
            let percent = limit.get("percent")?.as_f64().filter(|p| p.is_finite())?;
            let allowance = shares
                .iter()
                .find(|share| {
                    share.get("limit_index").and_then(Value::as_u64) == Some(index as u64)
                })
                .and_then(|share| share.get("allowance_percent_of_weekly"))
                .and_then(|value| match value {
                    Value::Number(number) => Some(number.clone()),
                    _ => None,
                });
            Some(ScopedLimit {
                model: model.to_string(),
                used_percent: percent.clamp(0.0, 100.0),
                resets_at: limit
                    .get("resets_at")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                allowance_percent_of_weekly: allowance,
            })
        })
        .collect()
}

fn scoped_window(
    model: &str,
    used_percent: i64,
    remaining_percent: i64,
    reset_at_epoch: Option<i64>,
    allowance: Option<Number>,
    weekly_minutes: i64,
) -> RateLimitWindow {
    RateLimitWindow {
        label: format!("{LABEL_PREFIX}{model}"),
        window_minutes: Some(weekly_minutes),
        used_percent,
        remaining_percent,
        reset_at_epoch,
        kind: Some(KIND.to_string()),
        model: Some(model.to_string()),
        allowance_percent_of_weekly: Some(allowance),
    }
}

impl ScopedLimit {
    pub(super) fn window(&self, weekly_minutes: i64) -> RateLimitWindow {
        scoped_window(
            &self.model,
            schema::percent_i64(self.used_percent),
            remaining_percent(self.used_percent),
            self.resets_at
                .as_deref()
                .and_then(|raw| reset_epoch_seconds_from_str(raw, None))
                .filter(|epoch| *epoch > 0),
            self.allowance_percent_of_weekly.clone(),
            weekly_minutes,
        )
    }
}

pub(super) fn is_scoped(window: &RateLimitWindow) -> bool {
    window.kind.as_deref() == Some(KIND)
}

/// Cache lines for the scoped windows:
/// `scoped_window=<remaining>,<reset>,<allowance or empty>,<model>`.
pub(super) fn cache_lines(windows: &[RateLimitWindow]) -> String {
    windows
        .iter()
        .filter(|window| is_scoped(window))
        .filter_map(|window| {
            Some(format!(
                "\n{CACHE_LINE_PREFIX}{},{},{},{}",
                window.remaining_percent,
                window.reset_at_epoch.unwrap_or_default(),
                window
                    .allowance_percent_of_weekly
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map(Number::to_string)
                    .unwrap_or_default(),
                window.model.as_deref()?
            ))
        })
        .collect()
}

/// The scoped windows stored in a cache file's content.
pub(super) fn windows_from_cache(content: &str, weekly_minutes: i64) -> Vec<RateLimitWindow> {
    content
        .lines()
        .filter_map(|line| line.strip_prefix(CACHE_LINE_PREFIX))
        .filter_map(|rest| {
            let mut parts = rest.splitn(4, ',');
            let remaining = parts.next()?.parse::<i64>().ok()?;
            let reset = parts.next()?.parse::<i64>().ok()?;
            let allowance = parts.next()?.parse::<Number>().ok();
            let model = parts.next().filter(|model| !model.is_empty())?;
            Some(scoped_window(
                model,
                schema::remaining_to_used_percent(remaining),
                remaining,
                Some(reset).filter(|epoch| *epoch > 0),
                allowance,
                weekly_minutes,
            ))
        })
        .collect()
}

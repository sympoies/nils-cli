//! Result schema shared by every provider's `diag rate-limits` command.
//!
//! Field order is part of the contract: providers emit these structs directly,
//! so reordering a field changes their byte-level JSON output.

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use super::driver::ProviderSpec;
use super::values::{CacheEntry, WeeklyValues};
use crate::diag_output::{self, ErrorEnvelope};
use crate::provider_usage::ProviderUsageReason;

/// Local-time layout of `weekly_reset_local` and the table's reset column.
pub const LOCAL_DATETIME_WITH_OFFSET: &str = "%m-%d %H:%M %:z";
/// Local-time layout of one-line and single-target reset times.
pub const LOCAL_DATETIME: &str = "%m-%d %H:%M";

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitSummary {
    pub non_weekly_label: Option<String>,
    pub non_weekly_remaining: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: Option<i64>,
    pub weekly_reset_epoch: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_reset_local: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitWindow {
    pub label: String,
    /// Window length when the provider reports a fixed one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<i64>,
    pub used_percent: i64,
    pub remaining_percent: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at_epoch: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResetCredits {
    pub available_count: i64,
}

/// One target's result in single, `--all`, and `--async` JSON output.
#[derive(Debug, Clone, Serialize)]
pub struct RateLimitResult {
    pub provider: String,
    pub name: String,
    pub target_file: String,
    pub status: String,
    pub ok: bool,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<ProviderUsageReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<RateLimitSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows: Option<Vec<RateLimitWindow>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_credits: Option<ResetCredits>,
    /// Provider-owned limit-reset status (Claude only); absent elsewhere.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_resets: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorEnvelope>,
}

/// Who a result describes: the provider, the display name, and the file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIdentity {
    pub provider: String,
    pub name: String,
    pub target_file: String,
}

impl RateLimitResult {
    /// A successful result with windows.
    pub fn ok(
        identity: TargetIdentity,
        source: &str,
        summary: RateLimitSummary,
        windows: Vec<RateLimitWindow>,
    ) -> Self {
        Self {
            provider: identity.provider,
            name: identity.name,
            target_file: identity.target_file,
            status: "ok".to_string(),
            ok: true,
            source: source.to_string(),
            reason_code: None,
            summary: Some(summary),
            windows: Some(windows),
            reset_credits: None,
            limit_resets: None,
            raw_usage: None,
            error: None,
        }
    }

    /// A failed result. It carries no windows.
    pub fn error(
        identity: TargetIdentity,
        source: &str,
        code: &str,
        message: String,
        details: Option<Value>,
        reason_code: Option<ProviderUsageReason>,
    ) -> Self {
        Self {
            provider: identity.provider,
            name: identity.name,
            target_file: identity.target_file,
            status: "error".to_string(),
            ok: false,
            source: source.to_string(),
            reason_code,
            summary: None,
            windows: None,
            reset_credits: None,
            limit_resets: None,
            raw_usage: None,
            error: Some(ErrorEnvelope {
                code: code.to_string(),
                message,
                details,
            }),
        }
    }

    /// The benign "no active rate-limit window" result: a success with no windows.
    pub fn no_window(identity: TargetIdentity, reset_credits: Option<ResetCredits>) -> Self {
        Self {
            provider: identity.provider,
            name: identity.name,
            target_file: identity.target_file,
            status: "ok".to_string(),
            ok: true,
            source: "network".to_string(),
            reason_code: None,
            summary: None,
            windows: Some(Vec::new()),
            reset_credits,
            limit_resets: None,
            raw_usage: None,
            error: None,
        }
    }

    /// A successful result built from a cache entry.
    pub fn from_cache(
        identity: TargetIdentity,
        source: &str,
        entry: &CacheEntry,
        format_with_offset: impl Fn(i64) -> Option<String>,
    ) -> Self {
        Self::ok(
            identity,
            source,
            summary_from_cache(entry, format_with_offset),
            windows_from_cache(entry),
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitSingleEnvelope {
    pub schema_version: String,
    pub command: String,
    pub mode: String,
    pub ok: bool,
    pub result: RateLimitResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct RateLimitCollectionEnvelope {
    pub schema_version: String,
    pub command: String,
    pub mode: String,
    pub ok: bool,
    pub results: Vec<RateLimitResult>,
}

pub fn emit_single_envelope(spec: &ProviderSpec, ok: bool, result: RateLimitResult) -> Result<()> {
    diag_output::emit_json(&RateLimitSingleEnvelope {
        schema_version: spec.schema_version.to_string(),
        command: spec.command.to_string(),
        mode: "single".to_string(),
        ok,
        result,
    })
}

pub fn emit_collection_envelope(
    spec: &ProviderSpec,
    mode: &str,
    ok: bool,
    results: Vec<RateLimitResult>,
) -> Result<()> {
    diag_output::emit_json(&RateLimitCollectionEnvelope {
        schema_version: spec.schema_version.to_string(),
        command: spec.command.to_string(),
        mode: mode.to_string(),
        ok,
        results,
    })
}

pub fn summary_from_weekly_values(
    weekly: &WeeklyValues,
    format_with_offset: impl Fn(i64) -> Option<String>,
) -> RateLimitSummary {
    RateLimitSummary {
        non_weekly_label: weekly
            .non_weekly
            .as_ref()
            .map(|window| window.label.clone()),
        non_weekly_remaining: weekly.non_weekly.as_ref().map(|window| window.remaining),
        non_weekly_reset_epoch: weekly
            .non_weekly
            .as_ref()
            .map(|window| window.reset_epoch)
            .filter(|epoch| *epoch > 0),
        weekly_remaining: weekly.weekly.as_ref().map(|window| window.remaining),
        weekly_reset_epoch: weekly.weekly.as_ref().map(|window| window.reset_epoch),
        weekly_reset_local: weekly
            .weekly
            .as_ref()
            .and_then(|window| format_with_offset(window.reset_epoch)),
    }
}

pub fn summary_from_cache(
    entry: &CacheEntry,
    format_with_offset: impl Fn(i64) -> Option<String>,
) -> RateLimitSummary {
    RateLimitSummary {
        non_weekly_label: entry.non_weekly_label.clone(),
        non_weekly_remaining: entry.non_weekly_remaining,
        non_weekly_reset_epoch: entry.non_weekly_reset_epoch,
        weekly_remaining: entry.weekly_remaining,
        weekly_reset_epoch: entry.weekly_reset_epoch,
        weekly_reset_local: entry.weekly_reset_epoch.and_then(format_with_offset),
    }
}

pub fn windows_from_cache(entry: &CacheEntry) -> Vec<RateLimitWindow> {
    let mut windows = Vec::new();
    if let (Some(label), Some(remaining)) = (&entry.non_weekly_label, entry.non_weekly_remaining) {
        windows.push(RateLimitWindow {
            label: label.clone(),
            window_minutes: None,
            used_percent: remaining_to_used_percent(remaining),
            remaining_percent: remaining,
            reset_at_epoch: entry.non_weekly_reset_epoch,
        });
    }
    if let (Some(remaining), Some(reset_epoch)) = (entry.weekly_remaining, entry.weekly_reset_epoch)
    {
        windows.push(RateLimitWindow {
            label: "Weekly".to_string(),
            window_minutes: None,
            used_percent: remaining_to_used_percent(remaining),
            remaining_percent: remaining,
            reset_at_epoch: Some(reset_epoch).filter(|epoch| *epoch > 0),
        });
    }
    windows
}

/// Rounds a used percentage to an integer in `0..=100`.
pub fn percent_i64(percent: f64) -> i64 {
    if percent.is_finite() {
        (percent.round() as i64).clamp(0, 100)
    } else {
        0
    }
}

pub fn remaining_to_used_percent(remaining: i64) -> i64 {
    (100 - remaining).clamp(0, 100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn identity() -> TargetIdentity {
        TargetIdentity {
            provider: "example".to_string(),
            name: "alpha".to_string(),
            target_file: "alpha.json".to_string(),
        }
    }

    #[test]
    fn result_field_order_is_stable() {
        let result = RateLimitResult::error(
            identity(),
            "network",
            "request-failed",
            "boom".to_string(),
            None,
            Some(ProviderUsageReason::RateLimited),
        );
        assert_eq!(
            serde_json::to_string(&result).expect("json"),
            r#"{"provider":"example","name":"alpha","target_file":"alpha.json","status":"error","ok":false,"source":"network","reason_code":"rate_limited","error":{"code":"request-failed","message":"boom"}}"#
        );
    }

    #[test]
    fn cache_windows_derive_used_percent_and_skip_missing_minutes() {
        let entry = CacheEntry {
            fetched_at_epoch: Some(1),
            non_weekly_label: Some("5h".to_string()),
            non_weekly_remaining: Some(70),
            non_weekly_reset_epoch: None,
            weekly_remaining: Some(40),
            weekly_reset_epoch: Some(0),
        };
        assert_eq!(
            serde_json::to_string(&windows_from_cache(&entry)).expect("json"),
            r#"[{"label":"5h","used_percent":30,"remaining_percent":70},{"label":"Weekly","used_percent":60,"remaining_percent":40}]"#
        );
        assert_eq!(percent_i64(f64::NAN), 0);
        assert_eq!(percent_i64(140.2), 100);
    }
}

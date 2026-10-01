//! The Claude Code usage-status read shared by `diag rate-limits` and
//! `auth reset-rate-limits`, and the normalized `limit_resets` it carries.
//!
//! The usage endpoint only evaluates the two limit-reset programs for a
//! Claude Code client, so this read sends Claude Code's own status query and a
//! `claude-cli/<version> (external, cli)` User-Agent. Each program block is
//! parsed defensively: a missing or malformed block becomes `null` and never
//! fails the usage read. Upstream `event_props`, `percent_used`, and
//! `blocking` are dropped.

use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use nils_common::env as shared_env;
use nils_common::usage_time::reset_epoch_seconds_from_str;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::agent::oneshot::claude_binary;
use crate::process::output_with_limits_retry_io;
use crate::prompt_segment::client::{self, RequestFailure};

/// Claude Code's own status query: both programs, without spend data.
pub(crate) const STATUS_QUERY: &str = "at_wall=1&skip_spend=1";
pub(crate) const CLAUDE_CODE_VERSION_ENV: &str = "CLAUDE_RATE_LIMITS_CLAUDE_CODE_VERSION";
/// Used when no valid version is pinned and none can be detected.
pub(crate) const FALLBACK_CLAUDE_CODE_VERSION: &str = "2.1.284";
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
const VERSION_PROBE_LIMIT: usize = 4 * 1024;

const MAX_GRANTS: usize = 16;
const MAX_NAMES: usize = 16;
const MAX_LABEL_CHARS: usize = 80;
const UNKNOWN_REASON: &str = "unknown";

pub(crate) const JUNIPER_TIDE: &str = "juniper_tide";
pub(crate) const CEDAR_EMBER: &str = "cedar_ember";

/// Reads the usage status with the Claude Code query and User-Agent.
pub(crate) fn request_status(access_token: &str) -> Result<String, RequestFailure> {
    client::request_usage_with(access_token, Some(STATUS_QUERY), &default_user_agent())
}

/// `claude-cli/<Claude Code version> (external, cli)`.
pub(crate) fn default_user_agent() -> String {
    format!("claude-cli/{} (external, cli)", claude_code_version())
}

/// The pinned version, else the installed Claude Code's, else the fallback.
/// Detection runs at most once per process.
fn claude_code_version() -> String {
    if let Some(pinned) =
        shared_env::env_non_empty(CLAUDE_CODE_VERSION_ENV).filter(|value| is_semver(value.trim()))
    {
        return pinned.trim().to_string();
    }
    static DETECTED: OnceLock<String> = OnceLock::new();
    DETECTED
        .get_or_init(|| {
            detect_claude_code_version().unwrap_or_else(|| FALLBACK_CLAUDE_CODE_VERSION.to_string())
        })
        .clone()
}

fn detect_claude_code_version() -> Option<String> {
    let mut command = Command::new(claude_binary());
    command.arg("--version").stdin(Stdio::null());
    let output =
        output_with_limits_retry_io(&mut command, VERSION_PROBE_TIMEOUT, VERSION_PROBE_LIMIT, 1)
            .ok()?;
    if !output.status.success() {
        return None;
    }
    leading_semver(&String::from_utf8_lossy(&output.stdout))
}

/// The leading `MAJOR.MINOR.PATCH` of `claude --version` output.
fn leading_semver(text: &str) -> Option<String> {
    let first = text.split_whitespace().next()?;
    is_semver(first).then(|| first.to_string())
}

fn is_semver(value: &str) -> bool {
    let parts: Vec<&str> = value.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty() && part.len() <= 9 && part.bytes().all(|b| b.is_ascii_digit())
        })
}

// --- normalized limit-reset status -------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct LimitResets {
    pub(crate) juniper_tide: Option<SessionReset>,
    pub(crate) cedar_ember: Option<GrantedResets>,
}

/// `juniper_tide`: the weekly session-limit reset offered at the wall.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct SessionReset {
    pub(crate) available: bool,
    pub(crate) eligible: bool,
    pub(crate) ineligible_reason: Option<String>,
    pub(crate) arm: Option<String>,
    pub(crate) resets_per_week: Option<u64>,
    pub(crate) next_available_at: Option<i64>,
    pub(crate) weekly_resets_at: Option<i64>,
}

/// `cedar_ember`: granted resets, each with an id and an expiry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct GrantedResets {
    pub(crate) available: bool,
    pub(crate) eligible: bool,
    pub(crate) ineligible_reason: Option<String>,
    pub(crate) at_limit: Option<bool>,
    pub(crate) exhausted: Vec<String>,
    pub(crate) next_grant_id: Option<String>,
    pub(crate) grants: Vec<Grant>,
    pub(crate) cooldown_until: Option<i64>,
    pub(crate) weekly_resets_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Grant {
    pub(crate) id: String,
    pub(crate) label: Option<String>,
    pub(crate) resets_left: u64,
    pub(crate) resets_total: Option<u64>,
    pub(crate) starts_at: Option<i64>,
    pub(crate) ends_at: Option<i64>,
    pub(crate) clears: Vec<String>,
    pub(crate) paused: bool,
    pub(crate) usable_now: bool,
    pub(crate) use_requires_limit: bool,
}

impl GrantedResets {
    /// The grant `next_grant_id` names, when it can be used now.
    pub(crate) fn next_usable_grant(&self) -> Option<&Grant> {
        let id = self.next_grant_id.as_deref()?;
        self.grants
            .iter()
            .find(|grant| grant.id == id)
            .filter(|grant| grant.resets_left > 0 && !grant.paused)
    }

    pub(crate) fn next_grant(&self) -> Option<&Grant> {
        let id = self.next_grant_id.as_deref()?;
        self.grants.iter().find(|grant| grant.id == id)
    }
}

/// Normalizes both programs from a usage body. Never fails.
pub(crate) fn parse_limit_resets(body: &Value) -> LimitResets {
    LimitResets {
        juniper_tide: body
            .get(JUNIPER_TIDE)
            .and_then(Value::as_object)
            .map(parse_session_reset),
        cedar_ember: body
            .get(CEDAR_EMBER)
            .and_then(Value::as_object)
            .map(parse_granted_resets),
    }
}

fn parse_session_reset(object: &Map<String, Value>) -> SessionReset {
    let eligible = is_true(object.get("eligible"));
    let arm = optional_token(object.get("arm"));
    SessionReset {
        available: eligible
            && is_true(object.get("available"))
            && arm.as_deref() != Some("control"),
        eligible,
        ineligible_reason: optional_token(object.get("ineligible_reason")),
        arm,
        resets_per_week: object.get("resets_per_week").and_then(Value::as_u64),
        next_available_at: timestamp(object.get("next_available_at")),
        weekly_resets_at: timestamp(object.get("weekly_resets_at")),
    }
}

fn parse_granted_resets(object: &Map<String, Value>) -> GrantedResets {
    let eligible = is_true(object.get("eligible"));
    let grants: Vec<Grant> = object
        .get("grants")
        .and_then(Value::as_array)
        .map(|grants| {
            grants
                .iter()
                .filter_map(parse_grant)
                .take(MAX_GRANTS)
                .collect()
        })
        .unwrap_or_default();
    let next_grant_id = object
        .get("next_grant_id")
        .and_then(Value::as_str)
        .filter(|id| grants.iter().any(|grant| grant.id == *id))
        .map(str::to_string);
    let mut resets = GrantedResets {
        available: false,
        eligible,
        ineligible_reason: optional_token(object.get("ineligible_reason")),
        at_limit: object.get("at_limit").and_then(Value::as_bool),
        exhausted: names(object.get("exhausted")),
        next_grant_id,
        grants,
        cooldown_until: timestamp(object.get("cooldown_until")),
        weekly_resets_at: timestamp(object.get("weekly_resets_at")),
    };
    resets.available = eligible && resets.next_usable_grant().is_some();
    resets
}

fn parse_grant(value: &Value) -> Option<Grant> {
    let object = value.as_object()?;
    let id = object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| is_grant_id(id))?;
    let resets_left = object.get("resets_left").and_then(Value::as_u64)?;
    Some(Grant {
        id: id.to_string(),
        label: label(object.get("label")),
        resets_left,
        resets_total: object.get("resets_total").and_then(Value::as_u64),
        starts_at: timestamp(object.get("starts_at")),
        ends_at: timestamp(object.get("ends_at")),
        clears: names(object.get("clears")),
        paused: is_true(object.get("paused")),
        usable_now: is_true(object.get("usable_now")),
        use_requires_limit: object
            .get("use_requires_limit")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn is_true(value: Option<&Value>) -> bool {
    value.and_then(Value::as_bool) == Some(true)
}

/// `^[a-z0-9_-]{1,40}$`, the upstream grant-id grammar.
pub(crate) fn is_grant_id(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// `^[a-z0-9_]{1,40}$`, the bounded token grammar for reasons and names.
pub(crate) fn is_token(value: &str) -> bool {
    (1..=40).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A string becomes itself when it is a bounded token, else `unknown`;
/// anything else is `None`.
pub(crate) fn optional_token(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    Some(if is_token(text) {
        text.to_string()
    } else {
        UNKNOWN_REASON.to_string()
    })
}

/// Bounded tokens from a string array; other entries are dropped.
fn names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|name| is_token(name))
                .take(MAX_NAMES)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn label(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()
        && text.chars().count() <= MAX_LABEL_CHARS
        && !text.chars().any(char::is_control))
    .then(|| text.to_string())
}

/// An ISO-8601 timestamp string as epoch seconds; anything else is `None`.
pub(crate) fn timestamp(value: Option<&Value>) -> Option<i64> {
    value
        .and_then(Value::as_str)
        .and_then(|raw| reset_epoch_seconds_from_str(raw, None))
        .filter(|epoch| *epoch > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn claude_code_version_output_parses_its_leading_semver() {
        assert_eq!(
            leading_semver("2.1.284 (Claude Code)\n").as_deref(),
            Some("2.1.284")
        );
        for bad in [
            "",
            "Claude Code 2.1.284",
            "2.1",
            "2.1.x (Claude Code)",
            "v2.1.284",
        ] {
            assert_eq!(leading_semver(bad), None, "{bad}");
        }
    }

    #[test]
    fn grants_are_capped_and_next_grant_must_name_a_kept_grant() {
        let grants: Vec<Value> = (0..20)
            .map(|index| json!({ "id": format!("g{index}"), "resets_left": 1 }))
            .collect();
        let parsed = parse_limit_resets(&json!({
            "cedar_ember": { "eligible": true, "grants": grants, "next_grant_id": "g17" }
        }));
        let cedar = parsed.cedar_ember.expect("cedar");
        assert_eq!(cedar.grants.len(), MAX_GRANTS);
        assert_eq!(cedar.next_grant_id, None);
        assert!(!cedar.available);
        assert_eq!(parsed.juniper_tide, None);
    }

    #[test]
    fn a_paused_or_empty_next_grant_is_not_available() {
        for grant in [
            json!({ "id": "g1", "resets_left": 0 }),
            json!({ "id": "g1", "resets_left": 2, "paused": true }),
        ] {
            let parsed = parse_limit_resets(&json!({
                "cedar_ember": { "eligible": true, "grants": [grant], "next_grant_id": "g1" }
            }));
            let cedar = parsed.cedar_ember.expect("cedar");
            assert_eq!(cedar.next_grant_id.as_deref(), Some("g1"));
            assert!(!cedar.available);
        }
    }
}

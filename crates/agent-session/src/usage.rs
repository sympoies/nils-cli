//! `GET /usage/v1` and `POST /codex/reset/v1`: provider usage and the earned
//! Codex rate-limit reset, backed by the `codex-cli` and `claude-cli` provider
//! CLIs. The contract lives in `docs/specs/serve-api-v1.md` under "Provider
//! usage and Codex reset".
//!
//! Only allowlisted fields cross the boundary: account nicknames, a bounded
//! plan tier, numeric window fields, fixed reason codes, and fixed note and
//! error strings owned by this module. Helper messages, provider account ids,
//! emails, and paths never reach a response.

use std::collections::VecDeque;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::http::StatusCode;
use nils_common::provider_usage::ProviderUsageReason;
use serde_json::{Map, Number, Value, json};
use tokio::sync::watch;

pub(crate) const USAGE_SCHEMA_VERSION: &str = "agent-session.provider-usage.v1";
/// The reset result keeps the schema id its console consumer already checks.
pub(crate) const RESET_SCHEMA_VERSION: &str = "agent-console.codex-rate-limit-reset.v1";
const RESET_CLI_SCHEMA_VERSION: &str = "codex-cli.account.reset-rate-limits.v1";
const RESET_CLI_COMMAND: &str = "account reset-rate-limits";

const REFRESH_ENV: &str = "AGENT_SESSION_USAGE_V1_REFRESH_SECONDS";
const RESET_ACCOUNTS_ENV: &str = "AGENT_SESSION_CODEX_RESET_ACCOUNTS";
const CLAUDE_INNER_TIMEOUT_ENV: &str = "CLAUDE_PROMPT_SEGMENT_CLAUDE_TIMEOUT_SECONDS";

/// A completed snapshot is served as fresh for this long; the next read after
/// it starts one background refresh.
const DEFAULT_REFRESH_SECONDS: u64 = 60;
const MAX_REFRESH_SECONDS: u64 = 300;
/// Windows whose snapshot is older than this, or more than the skew ahead of
/// the clock, are hidden and the entry is marked stale.
const MAX_STALE_SECONDS: i64 = 600;
const FUTURE_SKEW_SECONDS: i64 = 5;
/// How long a forced refresh or a cold read waits for its refresh before
/// serving what the cache holds.
const REFRESH_WAIT: Duration = Duration::from_secs(7);
/// A reset answers within this budget from its arrival, so a consumer with an
/// 8-second client timeout sees the outcome; a refresh that takes longer is
/// served as the last snapshot with a refreshing note.
const RESET_RESPONSE_BUDGET: Duration = Duration::from_secs(6);
const HELPER_TIMEOUT: Duration = Duration::from_secs(30);
/// `claude-cli` may fall back to a PTY probe; leave room to kill it cleanly.
const CLAUDE_INNER_TIMEOUT_SECONDS: u64 = 25;
const RESET_TIMEOUT: Duration = Duration::from_secs(30);
const HELPER_OUTPUT_LIMIT: u64 = 1024 * 1024;
const RESET_OUTPUT_LIMIT: u64 = 64 * 1024;
const MAX_RESET_BODY_BYTES: usize = 16 * 1024;
const RESET_REPLAY_CAPACITY: usize = 256;
const RESET_REPLAY_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_ACCOUNTS: usize = 64;
const MAX_WINDOWS: usize = 8;

const CODEX_PLAN_TYPES: &[&str] = &[
    "free",
    "go",
    "plus",
    "pro",
    "team",
    "business",
    "enterprise",
    "edu",
];
const RESET_OUTCOMES: &[&str] = &["reset", "nothing_to_reset", "no_credit", "already_redeemed"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Provider {
    Codex,
    Claude,
}

impl Provider {
    const fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
        }
    }

    const fn refreshing_note(self) -> &'static str {
        match self {
            Self::Codex => "Refreshing Codex usage; showing the last completed result.",
            Self::Claude => "Refreshing Claude usage; showing the last completed result.",
        }
    }

    const fn backoff_note(self) -> &'static str {
        match self {
            Self::Codex => {
                "Codex usage refresh failed; showing the last completed result until retry."
            }
            Self::Claude => {
                "Claude usage refresh failed; showing the last completed result until retry."
            }
        }
    }

    const fn loading_note(self) -> &'static str {
        match self {
            Self::Codex => "Codex usage is loading.",
            Self::Claude => "Claude usage is loading.",
        }
    }

    const fn expired_note(self) -> &'static str {
        match self {
            Self::Codex => "Codex usage is unavailable right now.",
            Self::Claude => "Couldn't fetch live Claude usage right now.",
        }
    }
}

const CLAUDE_NOTE_STALE_LAST_GOOD: &str =
    "Showing last known Claude usage (live fetch is failing).";
const CLAUDE_NOTE_NO_WINDOWS: &str = "Claude usage returned no windows.";

/// The fixed, user-facing text for each reason code.
fn reason_message(provider: Provider, reason: ProviderUsageReason) -> &'static str {
    use ProviderUsageReason as R;
    match (provider, reason) {
        (Provider::Codex, R::AuthRequired) => "Sign in to Codex to view usage.",
        (Provider::Claude, R::AuthRequired) => "Sign in to Claude to view usage.",
        (Provider::Codex, R::AuthExpired) => "Your Codex sign-in has expired. Sign in again.",
        (Provider::Claude, R::AuthExpired) => "Your Claude sign-in has expired. Sign in again.",
        (Provider::Codex, R::BillingPastDue) => {
            "Your Codex subscription payment is past due. Pay the overdue invoice to restore access, or contact your company admin."
        }
        (Provider::Claude, R::BillingPastDue) => {
            "Your Claude subscription payment is past due. Pay the overdue invoice to restore access, or contact your company admin."
        }
        (Provider::Codex, R::SubscriptionInactive) => {
            "Your Codex subscription is inactive. Renew it or contact your company admin to restore access."
        }
        (Provider::Claude, R::SubscriptionInactive) => {
            "Your Claude subscription is inactive. Renew it or contact your company admin to restore access."
        }
        (Provider::Codex, R::OrganizationDisabled) => {
            "Your organization has disabled Codex access. Contact your company admin to restore access."
        }
        (Provider::Claude, R::OrganizationDisabled) => {
            "Your organization has disabled Claude access. Use an Anthropic API key instead, or contact your company admin to restore access."
        }
        (Provider::Codex, R::PermissionDenied) => {
            "Your account does not have access to Codex. Contact your company admin."
        }
        (Provider::Claude, R::PermissionDenied) => {
            "Your account does not have access to Claude. Contact your company admin."
        }
        (Provider::Codex, R::RateLimited) => {
            "Codex usage is temporarily rate limited. Try again later."
        }
        (Provider::Claude, R::RateLimited) => {
            "Claude usage is temporarily rate limited. Try again later."
        }
        (Provider::Codex, R::Timeout) => "Codex usage refresh timed out.",
        (Provider::Claude, R::Timeout) => "Claude usage refresh timed out.",
        (Provider::Codex, R::ServiceUnavailable | R::Unknown) => {
            "Codex usage is unavailable right now."
        }
        (Provider::Claude, R::ServiceUnavailable | R::Unknown) => {
            "Claude usage is unavailable right now."
        }
    }
}

/// One provider entry in the edge-facing shape.
#[derive(Clone, Debug)]
struct Entry {
    provider: Provider,
    account: Option<String>,
    ok: bool,
    stale: bool,
    plan: Option<String>,
    windows: Vec<Value>,
    updated_at: Option<i64>,
    note: Option<&'static str>,
    error: Option<&'static str>,
    reason: Option<ProviderUsageReason>,
    reset_credits: Option<u64>,
}

impl Entry {
    fn new(provider: Provider, ok: bool) -> Self {
        Self {
            provider,
            account: None,
            ok,
            stale: false,
            plan: None,
            windows: Vec::new(),
            updated_at: None,
            note: None,
            error: None,
            reason: None,
            reset_credits: None,
        }
    }

    /// No completed refresh to show: Codex reports a failure, Claude a
    /// degraded sign-in or availability state, as the replaced helper did.
    fn unavailable(provider: Provider, reason: ProviderUsageReason) -> Self {
        let message = reason_message(provider, reason);
        let mut entry = Self::new(provider, provider == Provider::Claude);
        entry.stale = true;
        entry.note = Some(message);
        entry.reason = Some(reason);
        if provider == Provider::Codex {
            entry.error = Some(message);
        }
        entry
    }

    fn loading(provider: Provider) -> Self {
        let mut entry = Self::new(provider, true);
        entry.stale = true;
        entry.note = Some(provider.loading_note());
        entry
    }

    /// Hide windows from a snapshot that is too old or dated in the future.
    fn expire(&mut self, now: i64) {
        let expired = self
            .updated_at
            .is_none_or(|at| now - at >= MAX_STALE_SECONDS || at - now > FUTURE_SKEW_SECONDS);
        if !expired {
            return;
        }
        self.stale = true;
        self.windows.clear();
        self.note = self
            .reason
            .map(|reason| reason_message(self.provider, reason))
            .or(self.note)
            .or(Some(self.provider.expired_note()));
    }

    fn to_json(&self) -> Value {
        json!({
            "provider": self.provider.id(),
            "account": self.account,
            "label": self.provider.label(),
            "ok": self.ok,
            "stale": self.stale,
            "plan": self.plan,
            "windows": self.windows,
            "updated_at": self.updated_at,
            "note": self.note,
            "error": self.error,
            "reason_code": self.reason.map(ProviderUsageReason::as_str),
            "reset_credits": self.reset_credits.map(|count| json!({ "available_count": count })),
        })
    }
}

/// The outcome of one provider CLI run.
enum Refresh {
    Completed(Vec<Entry>),
    Failed(ProviderUsageReason),
}

// --- provider CLI runner -------------------------------------------------------

struct HelperOutput {
    status: Option<i32>,
    stdout: Vec<u8>,
}

enum HelperError {
    Spawn,
    Timeout,
    TooLarge,
}

/// Run a provider CLI with a closed stdin, a deadline, and bounded stdout. On
/// the deadline the whole process group is killed; stderr is discarded.
fn run_helper(
    program: &str,
    args: &[&str],
    envs: &[(&str, String)],
    timeout: Duration,
    limit: u64,
) -> Result<HelperOutput, HelperError> {
    let mut child = Command::new(program)
        .args(args)
        .envs(envs.iter().map(|(key, value)| (*key, value.as_str())))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| HelperError::Spawn)?;
    let mut pipe = child.stdout.take().ok_or(HelperError::Spawn)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stdout = Vec::new();
        let _ = (&mut pipe).take(limit + 1).read_to_end(&mut stdout);
        // Keep draining so an oversized writer exits instead of blocking.
        let _ = std::io::copy(&mut pipe, &mut std::io::sink());
        let _ = sender.send(stdout);
    });
    let kill_group = |child: &mut std::process::Child| {
        // SAFETY: signalling our own child's process group has no memory effects.
        unsafe {
            libc::kill(-(child.id() as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    };
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if Instant::now() >= deadline => {
                kill_group(&mut child);
                return Err(HelperError::Timeout);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => {
                kill_group(&mut child);
                return Err(HelperError::Spawn);
            }
        }
    };
    // A descendant that inherited stdout must not hold the reader past the
    // deadline.
    let remaining = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(100));
    let stdout = match receiver.recv_timeout(remaining) {
        Ok(stdout) => stdout,
        Err(_) => {
            kill_group(&mut child);
            return Err(HelperError::Timeout);
        }
    };
    if stdout.len() as u64 > limit {
        return Err(HelperError::TooLarge);
    }
    Ok(HelperOutput { status, stdout })
}

fn helper_failure(error: HelperError) -> ProviderUsageReason {
    match error {
        HelperError::Timeout => ProviderUsageReason::Timeout,
        HelperError::Spawn | HelperError::TooLarge => ProviderUsageReason::ServiceUnavailable,
    }
}

// --- field validation ----------------------------------------------------------

/// Codex account nicknames: the grammar the reset CLI accepts as `<nick>.json`.
pub(crate) fn valid_nickname(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// A canonical lowercase RFC 4122 UUID with a version from 1 to 8.
fn canonical_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
        && (b'1'..=b'8').contains(&bytes[14])
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

fn safe_token(value: &str, max: usize, extra: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || extra.contains(&byte))
}

fn reason_code(value: Option<&Value>) -> Option<ProviderUsageReason> {
    value
        .and_then(Value::as_str)
        .and_then(ProviderUsageReason::from_code)
}

/// A helper's classified reason: the result's own code, then the error
/// details, then the few error codes with a known meaning.
fn helper_reason(value: &Value) -> Option<ProviderUsageReason> {
    reason_code(value.get("reason_code")).or_else(|| {
        let error = value.get("error")?;
        reason_code(
            error
                .get("details")
                .and_then(|details| details.get("reason_code")),
        )
        .or_else(|| match error.get("code").and_then(Value::as_str)? {
            "missing-access-token" | "auth-unavailable" => Some(ProviderUsageReason::AuthRequired),
            "helper-timeout" | "request-timeout" => Some(ProviderUsageReason::Timeout),
            _ => None,
        })
    })
}

/// A finite percentage clamped to 0..=100, keeping integers integral.
fn percent(value: f64, integral: bool) -> Option<Number> {
    if !value.is_finite() {
        return None;
    }
    let clamped = value.clamp(0.0, 100.0);
    if integral {
        Some(Number::from(clamped as i64))
    } else {
        Number::from_f64(clamped)
    }
}

fn percent_field(value: Option<&Value>) -> Option<Number> {
    let number = value?.as_number()?;
    percent(number.as_f64()?, number.is_i64() || number.is_u64())
}

fn epoch_field(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().filter(|v| v.is_finite()).map(|v| v as i64)),
        Value::String(text) => text.trim().parse::<i64>().ok().or_else(|| {
            text.parse::<jiff::Timestamp>()
                .ok()
                .map(|at| at.as_second())
        }),
        _ => None,
    }
    .map(|epoch| {
        if epoch > 1_000_000_000_000 {
            epoch / 1000
        } else {
            epoch
        }
    })
    .filter(|epoch| *epoch > 0)
}

fn window(
    key: &str,
    label: &str,
    used: Option<Number>,
    minutes: Option<Number>,
    resets_at: Option<i64>,
) -> Value {
    json!({
        "key": key,
        "label": label,
        "used_percent": used,
        "window_minutes": minutes,
        "resets_at": resets_at,
    })
}

// --- Codex -----------------------------------------------------------------------

const CODEX_DIAG_ARGS: &[&str] = &[
    "diag",
    "rate-limits",
    "--all",
    "--format",
    "json",
    "--no-refresh-auth",
];

/// `(key, label, window_minutes)` for a codex-cli window label: `Weekly`, or a
/// provider duration such as `5h` or `1d`.
fn codex_window_spec(label: &str) -> Option<(String, String, Number)> {
    if label.eq_ignore_ascii_case("weekly") {
        return Some((
            "weekly".to_string(),
            "Weekly".to_string(),
            Number::from(10_080),
        ));
    }
    let (digits, unit) = label.split_at(label.len().checked_sub(1)?);
    if digits.is_empty()
        || digits.len() > 9
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let count: u64 = digits.parse().ok()?;
    let minutes = match unit.to_ascii_lowercase().as_str() {
        "w" => Number::from(count * 10_080),
        "d" => Number::from(count * 1_440),
        "h" => Number::from(count * 60),
        "m" => Number::from(count),
        "s" if count.is_multiple_of(60) => Number::from(count / 60),
        "s" => Number::from_f64(count as f64 / 60.0)?,
        _ => return None,
    };
    Some((label.to_ascii_lowercase(), label.to_string(), minutes))
}

fn codex_window(
    label: Option<&Value>,
    used: Option<Number>,
    resets_at: Option<i64>,
) -> Option<Value> {
    let (key, label, minutes) = codex_window_spec(label?.as_str()?)?;
    Some(window(&key, &label, Some(used?), Some(minutes), resets_at))
}

fn codex_windows(result: &Value) -> Vec<Value> {
    if let Some(windows) = result.get("windows") {
        return windows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|raw| {
                codex_window(
                    raw.get("label"),
                    percent_field(raw.get("used_percent")),
                    epoch_field(raw.get("reset_at_epoch")),
                )
            })
            .take(MAX_WINDOWS)
            .collect();
    }
    // Older helpers report only the remaining-percent summary.
    let Some(summary) = result.get("summary") else {
        return Vec::new();
    };
    let weekly = Value::String("Weekly".to_string());
    [
        (
            summary.get("non_weekly_label"),
            summary.get("non_weekly_remaining"),
            summary.get("non_weekly_reset_epoch"),
        ),
        (
            Some(&weekly),
            summary.get("weekly_remaining"),
            summary.get("weekly_reset_epoch"),
        ),
    ]
    .into_iter()
    .filter_map(|(label, remaining, resets_at)| {
        let remaining = remaining?.as_number()?;
        let integral = remaining.is_i64() || remaining.is_u64();
        let used = percent(100.0 - remaining.as_f64()?.clamp(0.0, 100.0), integral);
        codex_window(label, used, epoch_field(resets_at))
    })
    .collect()
}

fn codex_entry(result: &Value, updated_at: i64) -> Option<Entry> {
    let name = result.get("name").and_then(Value::as_str)?;
    if !valid_nickname(name) {
        return None;
    }
    let mut entry = Entry::new(Provider::Codex, true);
    entry.account = Some(name.to_string());
    entry.updated_at = Some(updated_at);
    let ok = result.get("ok").and_then(Value::as_bool) == Some(true)
        && result.get("status").and_then(Value::as_str) == Some("ok");
    if !ok {
        let reason = helper_reason(result);
        entry.ok = false;
        entry.reason = reason;
        entry.error = Some(reason_message(
            Provider::Codex,
            reason.unwrap_or(ProviderUsageReason::ServiceUnavailable),
        ));
        return Some(entry);
    }
    entry.windows = codex_windows(result);
    entry.plan = result
        .get("raw_usage")
        .and_then(|raw| raw.get("plan_type"))
        .and_then(Value::as_str)
        .filter(|plan| CODEX_PLAN_TYPES.contains(plan))
        .map(str::to_string);
    entry.reset_credits = result
        .get("reset_credits")
        .and_then(|credits| credits.get("available_count"))
        .and_then(Value::as_u64);
    Some(entry)
}

fn parse_codex(output: &HelperOutput, now: i64) -> Refresh {
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Refresh::Failed(ProviderUsageReason::ServiceUnavailable);
    };
    if value.get("ok").and_then(Value::as_bool) != Some(true) {
        return Refresh::Failed(
            helper_reason(&value).unwrap_or(ProviderUsageReason::ServiceUnavailable),
        );
    }
    let entries: Vec<Entry> = value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|result| codex_entry(result, now))
        .take(MAX_ACCOUNTS)
        .collect();
    if entries.is_empty() {
        return Refresh::Failed(ProviderUsageReason::ServiceUnavailable);
    }
    Refresh::Completed(entries)
}

fn refresh_codex() -> Refresh {
    match run_helper(
        "codex-cli",
        CODEX_DIAG_ARGS,
        &[],
        HELPER_TIMEOUT,
        HELPER_OUTPUT_LIMIT,
    ) {
        Ok(output) => parse_codex(&output, now_epoch()),
        Err(error) => Refresh::Failed(helper_failure(error)),
    }
}

// --- Claude ----------------------------------------------------------------------

fn claude_windows(result: &Value) -> Vec<Value> {
    result
        .get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|raw| {
            let key = raw
                .get("key")
                .and_then(Value::as_str)
                .filter(|key| safe_token(key, 32, b"_-"))?;
            let label = raw
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| safe_token(label, 32, b" ._-"))
                .unwrap_or(key);
            let minutes = raw
                .get("window_minutes")
                .and_then(Value::as_number)
                .filter(|minutes| minutes.as_f64().is_some_and(|v| v.is_finite() && v > 0.0))
                .cloned();
            let resets_at = epoch_field(raw.get("resets_at_epoch"))
                .or_else(|| epoch_field(raw.get("resets_at")));
            Some(window(
                key,
                label,
                percent_field(raw.get("used_percent")),
                minutes,
                resets_at,
            ))
        })
        .take(MAX_WINDOWS)
        .collect()
}

fn claude_entry(result: &Value, now: i64) -> Entry {
    let reason = reason_code(result.get("reason_code"));
    let windows = claude_windows(result);
    let stale = result.get("stale").and_then(Value::as_bool) == Some(true);
    let mut entry = Entry::new(Provider::Claude, true);
    entry.reason = reason;
    entry.plan = result
        .get("plan")
        .and_then(Value::as_str)
        .filter(|plan| safe_token(plan, 64, b"._-"))
        .map(str::to_string);
    entry.updated_at =
        epoch_field(result.get("updated_at")).or((!stale && !windows.is_empty()).then_some(now));
    let reason_note = reason.map(|reason| reason_message(Provider::Claude, reason));
    if windows.is_empty() {
        entry.stale = true;
        entry.note = reason_note.or(Some(CLAUDE_NOTE_NO_WINDOWS));
    } else {
        entry.stale = stale;
        entry.note = reason_note.or(stale.then_some(CLAUDE_NOTE_STALE_LAST_GOOD));
        entry.windows = windows;
    }
    entry
}

fn parse_claude(output: &HelperOutput, now: i64) -> Refresh {
    let Ok(value) = serde_json::from_slice::<Value>(&output.stdout) else {
        return Refresh::Failed(ProviderUsageReason::ServiceUnavailable);
    };
    let result = value.get("result").filter(|result| result.is_object());
    if output.status == Some(0)
        && value.get("ok").and_then(Value::as_bool) == Some(true)
        && let Some(result) = result
    {
        return Refresh::Completed(vec![claude_entry(result, now)]);
    }
    // A failed run is still a completed answer when it classified a reason,
    // such as a signed-out account.
    match result
        .and_then(helper_reason)
        .or_else(|| helper_reason(&value))
    {
        Some(reason) => Refresh::Completed(vec![Entry::unavailable(Provider::Claude, reason)]),
        None => Refresh::Failed(ProviderUsageReason::ServiceUnavailable),
    }
}

fn refresh_claude() -> Refresh {
    let inner = std::env::var(CLAUDE_INNER_TIMEOUT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map_or(CLAUDE_INNER_TIMEOUT_SECONDS, |seconds| {
            seconds.min(CLAUDE_INNER_TIMEOUT_SECONDS)
        });
    match run_helper(
        "claude-cli",
        &["usage", "--format", "json", "--source", "auto"],
        &[(CLAUDE_INNER_TIMEOUT_ENV, inner.to_string())],
        HELPER_TIMEOUT,
        HELPER_OUTPUT_LIMIT,
    ) {
        Ok(output) => parse_claude(&output, now_epoch()),
        Err(error) => Refresh::Failed(helper_failure(error)),
    }
}

fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

// --- stale-while-refresh cache -----------------------------------------------------

struct Snapshot {
    entries: Vec<Entry>,
    /// False for a placeholder standing in for a refresh that never succeeded.
    good: bool,
    completed_at: Instant,
}

#[derive(Default)]
struct SlotState {
    last: Option<Snapshot>,
    fresh_until: Option<Instant>,
    retry_after: Option<Instant>,
    refreshing: bool,
    /// A forced read arrived while a refresh was already running, so one more
    /// run starts when it completes.
    rerun: bool,
    completions: u64,
}

/// One provider's cache: a fresh snapshot is served as is; an expired one is
/// served as stale last-good while at most one background refresh runs.
struct Slot {
    provider: Provider,
    refresh_interval: Duration,
    state: Mutex<SlotState>,
    completed: watch::Sender<u64>,
}

impl Slot {
    fn new(provider: Provider, refresh_interval: Duration) -> Arc<Self> {
        Arc::new(Self {
            provider,
            refresh_interval,
            state: Mutex::new(SlotState::default()),
            completed: watch::channel(0).0,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotState> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Serve the cache. A forced read (or a cold one) waits up to `wait` for a
    /// refresh that started after it arrived; a forced read also stops treating
    /// the current snapshot as fresh.
    async fn read(self: &Arc<Self>, force: bool, wait: Duration) -> Vec<Entry> {
        let mut completed = self.completed.subscribe();
        let (target, should_wait) = {
            let mut state = self.lock();
            let now = Instant::now();
            let fresh = state.fresh_until.is_some_and(|until| now < until);
            let may_retry = state.retry_after.is_none_or(|after| now >= after);
            let mut target = state.completions;
            if force {
                target = self.invalidate_locked(&mut state);
            } else if !fresh && !state.refreshing && may_retry {
                self.start_refresh(&mut state);
            }
            (target, force || state.last.is_none())
        };
        if should_wait {
            let _ = tokio::time::timeout(wait, completed.wait_for(|count| *count > target)).await;
        }
        self.project()
    }

    /// Stop serving the current snapshot as fresh and make sure a refresh
    /// starts after now. Returns the completion count that refresh exceeds.
    fn invalidate(self: &Arc<Self>) -> u64 {
        let mut state = self.lock();
        self.invalidate_locked(&mut state)
    }

    fn invalidate_locked(self: &Arc<Self>, state: &mut SlotState) -> u64 {
        state.fresh_until = None;
        if state.refreshing {
            // The running refresh predates this request; wait for the next.
            state.rerun = true;
            state.completions + 1
        } else {
            self.start_refresh(state);
            state.completions
        }
    }

    /// Wait up to `wait` for completion `target` to pass, then serve the cache.
    async fn read_after(&self, target: u64, wait: Duration) -> Vec<Entry> {
        let mut completed = self.completed.subscribe();
        let _ = tokio::time::timeout(wait, completed.wait_for(|count| *count > target)).await;
        self.project()
    }

    fn start_refresh(self: &Arc<Self>, state: &mut SlotState) {
        state.refreshing = true;
        let slot = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            loop {
                let refresh = match slot.provider {
                    Provider::Codex => refresh_codex(),
                    Provider::Claude => refresh_claude(),
                };
                if !slot.complete(refresh) {
                    break;
                }
            }
        });
    }

    /// Record a finished refresh; true when a queued rerun must start now.
    fn complete(&self, refresh: Refresh) -> bool {
        let mut state = self.lock();
        let now = Instant::now();
        match refresh {
            Refresh::Completed(entries) => {
                state.last = Some(Snapshot {
                    entries,
                    good: true,
                    completed_at: now,
                });
                state.fresh_until = Some(now + self.refresh_interval);
                state.retry_after = None;
            }
            Refresh::Failed(reason) => {
                let keep_last_good = state.last.as_ref().is_some_and(|last| {
                    last.good
                        && now.duration_since(last.completed_at)
                            < Duration::from_secs(MAX_STALE_SECONDS as u64)
                });
                if !keep_last_good {
                    state.last = Some(Snapshot {
                        entries: vec![Entry::unavailable(self.provider, reason)],
                        good: false,
                        completed_at: now,
                    });
                }
                state.fresh_until = None;
                state.retry_after = Some(now + self.refresh_interval);
            }
        }
        let rerun = std::mem::take(&mut state.rerun);
        state.refreshing = rerun;
        state.completions += 1;
        self.completed.send_replace(state.completions);
        rerun
    }

    fn project(&self) -> Vec<Entry> {
        let state = self.lock();
        let Some(last) = state.last.as_ref() else {
            return vec![Entry::loading(self.provider)];
        };
        let fresh = state
            .fresh_until
            .is_some_and(|until| Instant::now() < until);
        let note = if state.refreshing {
            self.provider.refreshing_note()
        } else {
            self.provider.backoff_note()
        };
        let now = now_epoch();
        last.entries
            .iter()
            .cloned()
            .map(|mut entry| {
                entry.expire(now);
                if !fresh && last.good && entry.ok && !entry.stale {
                    entry.stale = true;
                    entry.note = Some(note);
                }
                entry
            })
            .collect()
    }
}

// --- service -------------------------------------------------------------------------

/// A handler failure, rendered by serve as its standard error envelope.
pub(crate) struct UsageApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: &'static str,
}

const fn api_error(status: StatusCode, code: &'static str, message: &'static str) -> UsageApiError {
    UsageApiError {
        status,
        code,
        message,
    }
}

struct RecordedReset {
    key: String,
    account: String,
    outcome: &'static str,
    windows_reset: Option<u64>,
    at: Instant,
}

pub(crate) struct UsageService {
    codex: Arc<Slot>,
    claude: Arc<Slot>,
    reset_accounts: Vec<String>,
    /// Serializes resets and remembers recent outcomes by idempotency key.
    resets: Arc<tokio::sync::Mutex<VecDeque<RecordedReset>>>,
}

impl UsageService {
    pub(crate) fn from_environment() -> Self {
        let refresh_seconds = std::env::var(REFRESH_ENV)
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok())
            .filter(|seconds| (1..=MAX_REFRESH_SECONDS).contains(seconds))
            .unwrap_or(DEFAULT_REFRESH_SECONDS);
        let refresh_interval = Duration::from_secs(refresh_seconds);
        let mut reset_accounts = Vec::new();
        let raw = std::env::var(RESET_ACCOUNTS_ENV).unwrap_or_default();
        for account in raw.split(|c: char| c == ',' || c.is_whitespace()) {
            if account.is_empty() {
                continue;
            }
            if !valid_nickname(account) {
                eprintln!("warning: {RESET_ACCOUNTS_ENV} ignored an invalid account nickname");
            } else if !reset_accounts.iter().any(|known| known == account) {
                reset_accounts.push(account.to_string());
            }
        }
        Self {
            codex: Slot::new(Provider::Codex, refresh_interval),
            claude: Slot::new(Provider::Claude, refresh_interval),
            reset_accounts,
            resets: Arc::new(tokio::sync::Mutex::new(VecDeque::new())),
        }
    }

    /// `data.usage` for `GET /usage/v1`: Codex accounts, then Claude.
    pub(crate) async fn snapshot(&self, force: bool) -> Value {
        self.snapshot_with(force, force, REFRESH_WAIT).await
    }

    async fn snapshot_with(&self, force_codex: bool, force_claude: bool, wait: Duration) -> Value {
        let (codex, claude) = tokio::join!(
            self.codex.read(force_codex, wait),
            self.claude.read(force_claude, wait)
        );
        usage_json(&codex, &claude)
    }

    /// `POST /codex/reset/v1`: consume one earned reset for an allowlisted
    /// account, then return the outcome with a refreshed usage snapshot.
    pub(crate) async fn codex_reset(
        &self,
        machine: &str,
        body: &[u8],
    ) -> Result<Value, UsageApiError> {
        let started = Instant::now();
        if self.reset_accounts.is_empty() {
            return Err(api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "codex-reset-not-configured",
                "no Codex account is allowlisted for rate-limit resets",
            ));
        }
        let (account, key) = parse_reset_request(body)?;
        if !self.reset_accounts.contains(&account) {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                "codex-reset-account-not-allowed",
                "the requested Codex account is not allowlisted for resets",
            ));
        }
        // The lookup, the CLI run, and the record happen in one task that owns
        // the lock, so a disconnected caller cannot release it mid-run or lose
        // the outcome a same-key retry must replay.
        let resets = Arc::clone(&self.resets);
        let codex = Arc::clone(&self.codex);
        let task = tokio::spawn(async move {
            let mut resets = resets.lock_owned().await;
            resets.retain(|recorded| recorded.at.elapsed() < RESET_REPLAY_TTL);
            match resets.iter().find(|recorded| recorded.key == key) {
                Some(recorded) if recorded.account != account => Err(api_error(
                    StatusCode::CONFLICT,
                    "idempotency-key-reused",
                    "the idempotency key was already used for another account",
                )),
                Some(recorded) => Ok((recorded.outcome, recorded.windows_reset, None)),
                None => {
                    let (run_account, run_key) = (account.clone(), key.clone());
                    let (outcome, windows_reset) = tokio::task::spawn_blocking(move || {
                        run_codex_reset(&run_account, &run_key)
                    })
                    .await
                    .map_err(|_| task_failed())??;
                    if resets.len() >= RESET_REPLAY_CAPACITY {
                        resets.pop_front();
                    }
                    resets.push_back(RecordedReset {
                        key,
                        account,
                        outcome,
                        windows_reset,
                        at: Instant::now(),
                    });
                    // Invalidate before releasing the lock, so even a replay
                    // after a disconnect never sees pre-reset numbers as fresh.
                    Ok((outcome, windows_reset, Some(codex.invalidate())))
                }
            }
        });
        let (outcome, windows_reset, refresh) = task.await.map_err(|_| task_failed())??;
        let replayed = refresh.is_none();
        let wait = RESET_RESPONSE_BUDGET
            .saturating_sub(started.elapsed())
            .min(REFRESH_WAIT);
        let usage = match refresh {
            Some(target) => {
                let (codex, claude) = tokio::join!(
                    self.codex.read_after(target, wait),
                    self.claude.read(false, wait)
                );
                usage_json(&codex, &claude)
            }
            None => self.snapshot_with(false, false, wait).await,
        };
        let mut result = Map::new();
        result.insert("schema_version".into(), json!(RESET_SCHEMA_VERSION));
        result.insert("outcome".into(), json!(outcome));
        if let Some(windows_reset) = windows_reset {
            result.insert("windows_reset".into(), json!(windows_reset));
        }
        result.insert("replayed".into(), json!(replayed));
        result.insert("machine".into(), json!(machine));
        result.insert("usage".into(), usage);
        Ok(Value::Object(result))
    }
}

fn usage_json(codex: &[Entry], claude: &[Entry]) -> Value {
    let providers: Vec<Value> = codex.iter().chain(claude).map(Entry::to_json).collect();
    json!({ "schema_version": USAGE_SCHEMA_VERSION, "providers": providers })
}

fn task_failed() -> UsageApiError {
    api_error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "serve-task-failed",
        "internal task failed",
    )
}

fn parse_reset_request(body: &[u8]) -> Result<(String, String), UsageApiError> {
    let invalid = || {
        api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid-request",
            "body must contain only a safe account and a canonical lowercase idempotency_key UUID",
        )
    };
    if body.len() > MAX_RESET_BODY_BYTES {
        return Err(invalid());
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| invalid())?;
    let object = value
        .as_object()
        .filter(|object| object.len() == 2)
        .ok_or_else(invalid)?;
    let account = object
        .get("account")
        .and_then(Value::as_str)
        .filter(|a| valid_nickname(a));
    let key = object
        .get("idempotency_key")
        .and_then(Value::as_str)
        .filter(|k| canonical_uuid(k));
    match (account, key) {
        (Some(account), Some(key)) => Ok((account.to_string(), key.to_string())),
        _ => Err(invalid()),
    }
}

fn run_codex_reset(account: &str, key: &str) -> Result<(&'static str, Option<u64>), UsageApiError> {
    let secret = format!("{account}.json");
    let output = run_helper(
        "codex-cli",
        &[
            "account",
            "reset-rate-limits",
            "--yes",
            "--idempotency-key",
            key,
            "--format",
            "json",
            &secret,
        ],
        &[],
        RESET_TIMEOUT,
        RESET_OUTPUT_LIMIT,
    )
    .map_err(|error| match error {
        HelperError::Timeout => api_error(
            StatusCode::GATEWAY_TIMEOUT,
            "codex-reset-timeout",
            "the Codex reset did not finish in time; retry with the same idempotency key",
        ),
        HelperError::Spawn => api_error(
            StatusCode::BAD_GATEWAY,
            "codex-reset-unavailable",
            "the Codex reset command is unavailable",
        ),
        HelperError::TooLarge => reset_invalid(),
    })?;
    parse_reset_output(&output)
}

fn reset_invalid() -> UsageApiError {
    api_error(
        StatusCode::BAD_GATEWAY,
        "codex-reset-invalid-response",
        "the Codex reset command returned an invalid response",
    )
}

fn parse_reset_output(output: &HelperOutput) -> Result<(&'static str, Option<u64>), UsageApiError> {
    if output.status != Some(0) {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "codex-reset-failed",
            "the Codex reset request could not be completed",
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|_| reset_invalid())?;
    let result = value.get("result");
    let outcome = result
        .and_then(|result| result.get("outcome"))
        .and_then(Value::as_str)
        .and_then(|outcome| {
            RESET_OUTCOMES
                .iter()
                .find(|known| **known == outcome)
                .copied()
        });
    let windows_reset = result.and_then(|result| result.get("windows_reset"));
    let valid_envelope = value.get("schema_version").and_then(Value::as_str)
        == Some(RESET_CLI_SCHEMA_VERSION)
        && value.get("command").and_then(Value::as_str) == Some(RESET_CLI_COMMAND)
        && value.get("ok").and_then(Value::as_bool) == Some(true);
    match (valid_envelope, outcome, windows_reset) {
        (true, Some(outcome), None) => Ok((outcome, None)),
        (true, Some(outcome), Some(count)) => count
            .as_u64()
            .map(|count| (outcome, Some(count)))
            .ok_or_else(reset_invalid),
        _ => Err(reset_invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn nickname_and_uuid_grammars_are_strict() {
        for good in ["alpha", "a.b_c-1", &"a".repeat(64)] {
            assert!(valid_nickname(good), "{good}");
        }
        for bad in ["", ".alpha", "../alpha", "a/b", "a b", &"a".repeat(65)] {
            assert!(!valid_nickname(bad), "{bad}");
        }
        assert!(canonical_uuid("0b5f4c1e-8d2a-4c7b-9e3f-2a1b0c9d8e7f"));
        for bad in [
            "0B5F4C1E-8D2A-4C7B-9E3F-2A1B0C9D8E7F",
            "0b5f4c1e-8d2a-0c7b-9e3f-2a1b0c9d8e7f",
            "0b5f4c1e-8d2a-4c7b-7e3f-2a1b0c9d8e7f",
            "0b5f4c1e8d2a4c7b9e3f2a1b0c9d8e7f",
        ] {
            assert!(!canonical_uuid(bad), "{bad}");
        }
    }

    #[test]
    fn codex_window_labels_follow_the_duration_grammar() {
        assert_eq!(
            codex_window_spec("5h"),
            Some(("5h".into(), "5h".into(), Number::from(300)))
        );
        assert_eq!(
            codex_window_spec("WEEKLY"),
            Some(("weekly".into(), "Weekly".into(), Number::from(10_080)))
        );
        assert_eq!(
            codex_window_spec("1D"),
            Some(("1d".into(), "1D".into(), Number::from(1_440)))
        );
        assert_eq!(
            codex_window_spec("90s"),
            Some(("90s".into(), "90s".into(), Number::from_f64(1.5).unwrap()))
        );
        for bad in ["", "h", "05h", "5x", " 5h", "burst window", "1234567890h"] {
            assert_eq!(codex_window_spec(bad), None, "{bad}");
        }
    }

    #[test]
    fn helper_timeouts_and_spawn_failures_map_to_fixed_reasons() {
        assert_eq!(
            helper_failure(HelperError::Timeout),
            ProviderUsageReason::Timeout
        );
        assert_eq!(
            helper_failure(HelperError::Spawn),
            ProviderUsageReason::ServiceUnavailable
        );
        let entry = Entry::unavailable(Provider::Codex, ProviderUsageReason::Timeout).to_json();
        assert_eq!(entry["ok"], false);
        assert_eq!(entry["reason_code"], "timeout");
        assert_eq!(entry["error"], "Codex usage refresh timed out.");
    }

    #[test]
    fn expired_or_future_snapshots_hide_their_windows() {
        let now = 2_000_000_000;
        let mut entry = Entry::new(Provider::Claude, true);
        entry.windows = vec![json!({ "key": "5h" })];
        for updated_at in [
            None,
            Some(now - MAX_STALE_SECONDS),
            Some(now + FUTURE_SKEW_SECONDS + 1),
        ] {
            let mut candidate = entry.clone();
            candidate.updated_at = updated_at;
            candidate.expire(now);
            assert!(candidate.windows.is_empty(), "{updated_at:?}");
            assert!(candidate.stale);
            assert_eq!(
                candidate.note,
                Some("Couldn't fetch live Claude usage right now.")
            );
        }
        let mut current = entry.clone();
        current.updated_at = Some(now - MAX_STALE_SECONDS + 1);
        current.expire(now);
        assert_eq!(current.windows.len(), 1);
        assert!(!current.stale);
    }

    #[test]
    fn an_expired_last_good_snapshot_carries_the_expired_note() {
        let slot = Slot::new(Provider::Codex, Duration::from_secs(60));
        {
            let mut state = slot.lock();
            let mut entry = Entry::new(Provider::Codex, true);
            entry.account = Some("alpha".to_string());
            entry.windows = vec![json!({ "key": "5h" })];
            entry.updated_at = Some(now_epoch() - MAX_STALE_SECONDS);
            state.last = Some(Snapshot {
                entries: vec![entry],
                good: true,
                completed_at: Instant::now(),
            });
            state.refreshing = true;
        }
        let entries = slot.project();
        assert!(entries[0].windows.is_empty());
        assert!(entries[0].stale);
        assert_eq!(entries[0].note, Some(Provider::Codex.expired_note()));
    }

    #[test]
    fn reset_output_requires_the_versioned_cli_envelope() {
        let output = |status: i32, body: Value| HelperOutput {
            status: Some(status),
            stdout: serde_json::to_vec(&body).unwrap(),
        };
        let envelope = |result: Value| {
            json!({
                "schema_version": RESET_CLI_SCHEMA_VERSION,
                "command": RESET_CLI_COMMAND,
                "ok": true,
                "result": result,
            })
        };
        assert!(matches!(
            parse_reset_output(&output(0, envelope(json!({ "outcome": "no_credit" })))),
            Ok(("no_credit", None))
        ));
        for (status, body) in [
            (1, envelope(json!({ "outcome": "reset" }))),
            (0, envelope(json!({ "outcome": "consumed" }))),
            (
                0,
                envelope(json!({ "outcome": "reset", "windows_reset": -1 })),
            ),
            (0, json!({ "ok": true, "result": { "outcome": "reset" } })),
        ] {
            assert!(parse_reset_output(&output(status, body)).is_err());
        }
    }
}

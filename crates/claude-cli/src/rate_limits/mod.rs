//! `claude-cli diag rate-limits`: Claude OAuth usage per profile, in the shared
//! `diag rate-limits` result shape.
//!
//! Targets are the profiles in `CLAUDE_SECRET_DIR` (`<name>.json`) or, with no
//! target, the active login. Each target's stored access token reads
//! `GET /api/oauth/usage` once. Tokens are never refreshed, rewritten, or
//! printed; an expired token is reported without a request.
//!
//! The read sends Claude Code's status query and User-Agent (`status.rs`), so
//! every network result also carries the normalized `limit_resets`. Cached
//! and cache-fallback results omit it.

mod cache;
pub(crate) mod status;

use anyhow::Result;
use chrono::{Local, TimeZone, Utc};
use nils_common::diag_output;
use nils_common::env as shared_env;
use nils_common::provider_usage::ProviderUsageReason;
use nils_common::rate_limits::driver::{self, CacheFallbackPolicy};
use nils_common::rate_limits::schema::{self, LOCAL_DATETIME, LOCAL_DATETIME_WITH_OFFSET};
use nils_common::rate_limits::values::{self as shared_values, normalize_one_line};
use nils_common::rate_limits::{
    OneLineFetch, ProgressSink, ProviderSpec, RC_NO_RATE_LIMIT_WINDOW, RateLimitResult,
    RateLimitWindow, RateLimitsProvider, ResetEpochs, RunOptions, TargetIdentity, WeeklyValues,
    WindowValues,
};
use nils_common::usage_time::reset_epoch_seconds_from_str;
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

use crate::auth::{keychain, store};
use crate::prompt_segment::client::RequestFailure;
use crate::prompt_segment::render::{self as usage_render, Window};

pub use nils_common::rate_limits::RunOptions as RateLimitsOptions;

const SCHEMA_VERSION: &str = "claude-cli.diag.rate-limits.v1";
const COMMAND: &str = "diag rate-limits";
const TOOL: &str = "claude-rate-limits";
const FIVE_HOUR_LABEL: &str = "5h";
const WEEKLY_LABEL: &str = "Weekly";
const FIVE_HOUR_MINUTES: i64 = 300;
const WEEKLY_MINUTES: i64 = 10_080;
/// Row and cache name of an active login that no profile holds.
const ACTIVE_NAME: &str = "active";
const ASYNC_JSON_NO_CACHE_FALLBACK_ENV: &str = "CLAUDE_RATE_LIMITS_ASYNC_JSON_NO_CACHE_FALLBACK";

static CLAUDE_SPEC: ProviderSpec = ProviderSpec {
    provider: "claude",
    schema_version: SCHEMA_VERSION,
    command: COMMAND,
    tool: TOOL,
    table_title: "Claude rate limits for all accounts",
    secret_dir_env: store::SECRET_DIR_ENV,
    usage: "claude-cli diag rate-limits [--cached] [--format text|json] [--one-line] [--all] [--async [--watch] [--jobs N]] [profile]",
    default_all_env: Some("CLAUDE_RATE_LIMITS_DEFAULT_ALL_ENABLED"),
    watch_max_rounds_env: "CLAUDE_RATE_LIMITS_WATCH_MAX_ROUNDS",
    watch_interval_env: "CLAUDE_RATE_LIMITS_WATCH_INTERVAL_SECONDS",
};

pub fn run(options: &RateLimitsOptions) -> i32 {
    driver::run(&ClaudeRateLimits, options).unwrap_or(1)
}

struct ClaudeRateLimits;

struct ClaudeProgress(Progress);

impl ProgressSink for ClaudeProgress {
    fn set_message(&self, message: String) {
        self.0.set_message(message);
    }

    fn inc(&self, delta: u64) {
        self.0.inc(delta);
    }

    fn finish_and_clear(self: Box<Self>) {
        self.0.finish_and_clear();
    }
}

/// A target's stored OAuth access token.
struct Login {
    access_token: String,
    expires_at_ms: Option<i64>,
}

/// One live usage read.
enum Fetch {
    Windows {
        values: WeeklyValues,
        windows: Vec<RateLimitWindow>,
        limit_resets: Value,
    },
    NoWindow {
        limit_resets: Value,
    },
    Failed {
        code: &'static str,
        message: String,
        reason: Option<ProviderUsageReason>,
    },
}

impl RateLimitsProvider for ClaudeRateLimits {
    fn spec(&self) -> &ProviderSpec {
        &CLAUDE_SPEC
    }

    fn now_epoch(&self) -> i64 {
        Utc::now().timestamp()
    }

    fn format_local(&self, epoch: i64, format: &str) -> Option<String> {
        format_local(epoch, format)
    }

    fn progress(&self, total: usize, prefix: &str) -> Option<Box<dyn ProgressSink>> {
        Some(Box::new(ClaudeProgress(Progress::new(
            total as u64,
            ProgressOptions::default()
                .with_prefix(prefix)
                .with_finish(ProgressFinish::Clear),
        ))))
    }

    fn secret_dir(&self) -> PathBuf {
        store::secret_dir().unwrap_or_default()
    }

    fn async_json_targets(
        &self,
    ) -> std::result::Result<Vec<PathBuf>, driver::TargetDiscoveryError> {
        let secret_dir = self.secret_dir();
        let profile_dir_missing = std::fs::metadata(&secret_dir)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
        let active = read_active_login().ok();
        let active_path = active.as_ref().and_then(|_| store::credentials_file());
        let mut targets =
            match driver::collect_json_targets_from_dir(&CLAUDE_SPEC, &secret_dir, true) {
                Ok(targets) => targets,
                Err(error) if profile_dir_missing || error.1.contains("no secrets found") => {
                    return match active_path {
                        Some(path) => Ok(vec![path]),
                        None => Err(error),
                    };
                }
                Err(error) => return Err(error),
            };

        if let (Some(active), Some(active_path)) = (active, active_path) {
            let represented = targets.iter().any(|target| {
                read_profile_login(target)
                    .is_ok_and(|profile| profile.access_token == active.access_token)
            });
            if !represented {
                targets.push(active_path);
            }
        }
        targets.sort();
        Ok(targets)
    }

    fn identity(&self, target: &Path) -> TargetIdentity {
        TargetIdentity {
            provider: "claude".to_string(),
            name: target_name(target),
            target_file: target_file_name(target),
        }
    }

    fn clear_cache(&self) -> std::result::Result<(), String> {
        cache::clear()
    }

    fn run_single(&self, args: &RunOptions, cached: bool, one_line: bool) -> Result<i32> {
        run_single(self, args, cached, one_line)
    }

    fn json_result(
        &self,
        target: &Path,
        cached: bool,
        fallback: CacheFallbackPolicy,
    ) -> RateLimitResult {
        let fallback = if shared_env::env_truthy(ASYNC_JSON_NO_CACHE_FALLBACK_ENV) && !cached {
            CacheFallbackPolicy::NoWindow
        } else {
            fallback
        };
        json_result(self, target, cached, fallback)
    }

    fn async_one_line(&self, target: &Path, cached: bool) -> OneLineFetch {
        let name = target_name(target);
        if cached {
            return cached_one_line(&name);
        }
        let mut errors = Vec::new();
        match fetch(target) {
            Fetch::Windows { values, .. } => {
                write_cache(&name, &values);
                return OneLineFetch {
                    line: shared_values::one_line_from_values(&values, format_local_datetime)
                        .map(normalize_one_line),
                    ..Default::default()
                };
            }
            Fetch::NoWindow { .. } => {
                if let Some(fetch) = stale_one_line(&name, &mut errors) {
                    return fetch;
                }
                return OneLineFetch {
                    rc: RC_NO_RATE_LIMIT_WINDOW,
                    no_window: true,
                    err: errors.join("\n"),
                    ..Default::default()
                };
            }
            Fetch::Failed { message, .. } => errors.push(message),
        }
        if let Some(mut fetch) = stale_one_line(&name, &mut errors) {
            errors.push(format!("{TOOL}-async: falling back to cache"));
            fetch.err = errors.join("\n");
            return fetch;
        }
        OneLineFetch {
            rc: 1,
            err: errors.join("\n"),
            ..Default::default()
        }
    }

    fn sequential_one_line(&self, target: &Path, cached: bool, debug: bool) -> OneLineFetch {
        let name = target_name(target);
        if cached {
            let fetch = cached_one_line(&name);
            if debug && !fetch.err.is_empty() {
                eprintln!("{}", fetch.err);
            }
            return OneLineFetch {
                line: fetch.line,
                ..Default::default()
            };
        }
        match fetch(target) {
            Fetch::Windows { values, .. } => {
                write_cache(&name, &values);
                OneLineFetch {
                    line: shared_values::one_line_from_values(&values, format_local_datetime),
                    ..Default::default()
                }
            }
            Fetch::NoWindow { .. } => OneLineFetch {
                line: cache::read_allow_stale(&name, now_epoch())
                    .ok()
                    .and_then(|read| {
                        shared_values::one_line_from_cache(&read.entry, format_local_datetime)
                    }),
                no_window: true,
                ..Default::default()
            },
            Fetch::Failed {
                message, reason, ..
            } => {
                if reason == Some(ProviderUsageReason::RateLimited) {
                    let mut errors = Vec::new();
                    if let Some(mut result) = stale_one_line(&name, &mut errors) {
                        result.stale = true;
                        return result;
                    }
                }
                if debug {
                    eprintln!("{message}");
                }
                OneLineFetch::default()
            }
        }
    }

    fn row_reset_epochs(
        &self,
        target: &Path,
        cached: bool,
        _fill_from_stale_cache: bool,
    ) -> ResetEpochs {
        let name = target_name(target);
        let entry = if cached {
            cache::read_for_cached_mode(&name, now_epoch()).ok()
        } else {
            cache::read_allow_stale(&name, now_epoch())
                .ok()
                .map(|read| read.entry)
        };
        entry
            .map(|entry| ResetEpochs {
                non_weekly: entry.non_weekly_reset_epoch.filter(|epoch| *epoch > 0),
                weekly: entry.weekly_reset_epoch.filter(|epoch| *epoch > 0),
            })
            .unwrap_or_default()
    }

    /// The recorded current default `auth current` reports, when listed.
    fn current_name(&self, targets: &[PathBuf]) -> Option<String> {
        let current = store::read_current().ok().flatten()?;
        targets
            .iter()
            .map(|target| target_name(target))
            .find(|name| *name == current)
    }
}

fn run_single(
    provider: &ClaudeRateLimits,
    args: &RunOptions,
    cached: bool,
    one_line: bool,
) -> Result<i32> {
    let target = match args.secret.as_deref() {
        Some(name) => {
            if store::validate_profile_name(name).is_err() {
                return emit_single_error(
                    args.json,
                    "invalid-profile-name",
                    format!("{TOOL}: invalid profile name"),
                    None,
                    64,
                );
            }
            let target = provider.secret_dir().join(format!("{name}.json"));
            if !target.is_file() {
                return emit_single_error(
                    args.json,
                    "target-not-found",
                    format!("{TOOL}: profile '{name}' not found"),
                    Some(serde_json::json!({ "target_file": target_file_name(&target) })),
                    1,
                );
            }
            target
        }
        None => match store::credentials_file() {
            Some(path) => path,
            None => {
                return emit_single_error(
                    args.json,
                    "config-dir-unresolved",
                    format!("{TOOL}: cannot resolve the Claude config directory"),
                    None,
                    1,
                );
            }
        },
    };

    if cached {
        return match cache::read_for_cached_mode(&target_name(&target), now_epoch()) {
            Ok(entry) => {
                if let Some(line) =
                    shared_values::one_line_from_cache(&entry, format_local_datetime)
                {
                    println!("{line}");
                }
                Ok(0)
            }
            Err(err) => {
                eprintln!("{err}");
                Ok(1)
            }
        };
    }

    let result = json_result(provider, &target, false, CacheFallbackPolicy::NoWindow);
    let rc = if result.ok { 0 } else { 1 };
    if args.json {
        schema::emit_single_envelope(&CLAUDE_SPEC, result.ok, result)?;
        return Ok(rc);
    }
    if let Some(error) = &result.error {
        eprintln!("{}", error.message);
        return Ok(rc);
    }

    let summary = result.summary.as_ref();
    if one_line {
        if let Some(summary) = summary
            && let Some(line) = shared_values::format_one_line_output(
                summary.non_weekly_label.as_deref(),
                summary.non_weekly_remaining,
                summary.weekly_remaining,
                summary.weekly_reset_epoch,
                format_local_datetime,
            )
        {
            println!("{line}");
        }
        return Ok(rc);
    }

    let windows = result.windows.as_deref().unwrap_or_default();
    if windows.is_empty() {
        println!("No active rate-limit window");
        return Ok(rc);
    }
    println!("Rate limits remaining");
    for window in windows {
        let reset = window
            .reset_at_epoch
            .and_then(format_local_datetime)
            .unwrap_or_else(|| "?".to_string());
        println!("{} {}% • {}", window.label, window.remaining_percent, reset);
    }
    Ok(rc)
}

/// Reports a single-target failure as a JSON error envelope or on stderr.
fn emit_single_error(
    json: bool,
    code: &str,
    message: String,
    details: Option<Value>,
    exit_code: i32,
) -> Result<i32> {
    if json {
        diag_output::emit_error(SCHEMA_VERSION, COMMAND, code, message, details)?;
    } else {
        eprintln!("{message}");
    }
    Ok(exit_code)
}

fn json_result(
    provider: &ClaudeRateLimits,
    target: &Path,
    cached: bool,
    fallback: CacheFallbackPolicy,
) -> RateLimitResult {
    let identity = provider.identity(target);
    let name = identity.name.clone();
    if cached {
        let mut result = from_cache(identity, "cache", &name, true);
        if read_target_login(target)
            .is_ok_and(|login| crate::prompt_segment::client::backoff_active(&login.access_token))
        {
            result.reason_code = Some(ProviderUsageReason::RateLimited);
        }
        return result;
    }

    match fetch(target) {
        Fetch::Windows {
            values,
            windows,
            limit_resets,
        } => {
            write_cache(&name, &values);
            let summary = schema::summary_from_weekly_values(&values, format_local_with_offset);
            let mut result = sanitize(RateLimitResult::ok(identity, "network", summary, windows));
            result.limit_resets = Some(limit_resets);
            result
        }
        Fetch::NoWindow { limit_resets } => {
            let fallback_result = from_cache(identity.clone(), "cache-fallback", &name, false);
            if fallback_result.ok {
                return fallback_result;
            }
            let mut result = RateLimitResult::no_window(identity, None);
            result.limit_resets = Some(limit_resets);
            result
        }
        Fetch::Failed {
            code,
            message,
            reason,
        } => {
            if fallback == CacheFallbackPolicy::AnyFailure
                || (reason == Some(ProviderUsageReason::RateLimited)
                    && !shared_env::env_truthy(ASYNC_JSON_NO_CACHE_FALLBACK_ENV))
            {
                let mut fallback_result =
                    from_cache(identity.clone(), "cache-fallback", &name, false);
                if fallback_result.ok {
                    if reason == Some(ProviderUsageReason::RateLimited) {
                        fallback_result.reason_code = reason;
                    }
                    return fallback_result;
                }
            }
            RateLimitResult::error(identity, "network", code, message, None, reason)
        }
    }
}

fn from_cache(
    identity: TargetIdentity,
    source: &str,
    name: &str,
    enforce_ttl: bool,
) -> RateLimitResult {
    let entry = if enforce_ttl {
        cache::read_for_cached_mode(name, now_epoch())
    } else {
        cache::read_allow_stale(name, now_epoch()).map(|read| read.entry)
    };
    match entry {
        Ok(entry) => sanitize(RateLimitResult::from_cache(
            identity,
            source,
            &entry,
            format_local_with_offset,
        )),
        Err(message) => {
            RateLimitResult::error(identity, source, "cache-read-failed", message, None, None)
        }
    }
}

/// Adds the fixed Claude window lengths and drops a missing weekly reset.
fn sanitize(mut result: RateLimitResult) -> RateLimitResult {
    if let Some(summary) = result.summary.as_mut()
        && summary.weekly_reset_epoch.is_some_and(|epoch| epoch <= 0)
    {
        summary.weekly_reset_epoch = None;
        summary.weekly_reset_local = None;
    }
    for window in result.windows.iter_mut().flatten() {
        window.window_minutes = window_minutes(&window.label);
    }
    result
}

fn window_minutes(label: &str) -> Option<i64> {
    match label {
        FIVE_HOUR_LABEL => Some(FIVE_HOUR_MINUTES),
        WEEKLY_LABEL => Some(WEEKLY_MINUTES),
        _ => None,
    }
}

fn cached_one_line(name: &str) -> OneLineFetch {
    match cache::read_for_cached_mode(name, now_epoch()) {
        Ok(entry) => OneLineFetch {
            line: shared_values::one_line_from_cache(&entry, format_local_datetime),
            ..Default::default()
        },
        Err(err) => OneLineFetch {
            rc: 1,
            err,
            ..Default::default()
        },
    }
}

fn stale_one_line(name: &str, errors: &mut Vec<String>) -> Option<OneLineFetch> {
    match cache::read_allow_stale(name, now_epoch()) {
        Ok(read) => {
            shared_values::one_line_from_cache(&read.entry, format_local_datetime).map(|line| {
                OneLineFetch {
                    line: Some(normalize_one_line(line)),
                    stale: read.stale,
                    ..Default::default()
                }
            })
        }
        Err(err) => {
            errors.push(err);
            None
        }
    }
}

fn write_cache(name: &str, values: &WeeklyValues) {
    let _ = cache::write(name, now_epoch(), values);
}

fn fetch(target: &Path) -> Fetch {
    let login = match read_target_login(target) {
        Ok(login) => login,
        Err(message) => {
            return Fetch::Failed {
                code: "missing-access-token",
                message,
                reason: Some(ProviderUsageReason::AuthRequired),
            };
        }
    };
    if login
        .expires_at_ms
        .is_some_and(|expires_at| expires_at <= Utc::now().timestamp_millis())
    {
        return Fetch::Failed {
            code: "access-token-expired",
            message: format!("{TOOL}: access token expired; refresh the login and retry"),
            reason: Some(ProviderUsageReason::AuthExpired),
        };
    }

    let body = match status::request_status(&login.access_token) {
        Ok(body) => body,
        Err(failure) => {
            let reason = request_failure_reason(&failure);
            return Fetch::Failed {
                code: "request-failed",
                message: format!("{TOOL}: usage request failed ({})", reason.as_str()),
                reason: Some(reason),
            };
        }
    };
    parse_usage_body(&body)
}

fn request_failure_reason(failure: &RequestFailure) -> ProviderUsageReason {
    match failure {
        RequestFailure::Backoff => ProviderUsageReason::RateLimited,
        RequestFailure::Client => ProviderUsageReason::Unknown,
        RequestFailure::Transport { timeout: true } => ProviderUsageReason::Timeout,
        RequestFailure::Transport { timeout: false } => ProviderUsageReason::ServiceUnavailable,
        RequestFailure::Http { status, .. } => match status {
            401 => ProviderUsageReason::AuthExpired,
            403 => ProviderUsageReason::PermissionDenied,
            429 => ProviderUsageReason::RateLimited,
            _ => ProviderUsageReason::ServiceUnavailable,
        },
    }
}

fn parse_usage_body(body: &str) -> Fetch {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return Fetch::Failed {
            code: "invalid-usage-payload",
            message: format!("{TOOL}: invalid usage payload"),
            reason: None,
        };
    };
    let limit_resets =
        serde_json::to_value(status::parse_limit_resets(&value)).unwrap_or(Value::Null);
    let Some(usage) = usage_render::parse_usage_value(&value) else {
        return if value.is_object() {
            Fetch::NoWindow { limit_resets }
        } else {
            Fetch::Failed {
                code: "invalid-usage-payload",
                message: format!("{TOOL}: invalid usage payload"),
                reason: None,
            }
        };
    };

    let window_values = |window: &Window, label: &str| WindowValues {
        label: label.to_string(),
        remaining: window.remaining_percent,
        reset_epoch: reset_epoch(window).unwrap_or_default(),
    };
    let values = WeeklyValues {
        weekly: usage
            .seven_day
            .as_ref()
            .map(|window| window_values(window, WEEKLY_LABEL)),
        non_weekly: usage
            .five_hour
            .as_ref()
            .map(|window| window_values(window, FIVE_HOUR_LABEL)),
    };
    let windows = [
        (usage.five_hour.as_ref(), FIVE_HOUR_LABEL, FIVE_HOUR_MINUTES),
        (usage.seven_day.as_ref(), WEEKLY_LABEL, WEEKLY_MINUTES),
    ]
    .into_iter()
    .filter_map(|(window, label, minutes)| {
        let window = window?;
        Some(RateLimitWindow {
            label: label.to_string(),
            window_minutes: Some(minutes),
            used_percent: schema::percent_i64(window.used_percent),
            remaining_percent: window.remaining_percent,
            reset_at_epoch: reset_epoch(window),
        })
    })
    .collect();
    Fetch::Windows {
        values,
        windows,
        limit_resets,
    }
}

fn reset_epoch(window: &Window) -> Option<i64> {
    window
        .resets_at
        .as_deref()
        .and_then(|raw| reset_epoch_seconds_from_str(raw, None))
        .filter(|epoch| *epoch > 0)
}

fn is_active_target(target: &Path) -> bool {
    store::credentials_file().as_deref() == Some(target)
}

fn read_target_login(target: &Path) -> std::result::Result<Login, String> {
    if is_active_target(target) {
        read_active_login()
    } else {
        read_profile_login(target)
    }
}

fn read_profile_login(target: &Path) -> std::result::Result<Login, String> {
    let object = store::read_json_object(target, "profile-invalid")
        .map_err(|err| format!("{TOOL}: {}", err.message))?
        .unwrap_or_default();
    login_from_credentials(&object)
        .ok_or_else(|| format!("{TOOL}: no access token in {}", target_file_name(target)))
}

/// The active login Claude Code reads: the credentials file, else the Keychain.
fn read_active_login() -> std::result::Result<Login, String> {
    let path = store::credentials_file()
        .ok_or_else(|| format!("{TOOL}: cannot resolve the Claude config directory"))?;
    let mut object = store::read_json_object(&path, "active-credentials-invalid")
        .map_err(|err| format!("{TOOL}: {}", err.message))?;
    if object
        .as_ref()
        .is_none_or(|object| !object.contains_key("claudeAiOauth"))
        && keychain::enabled()
    {
        object = keychain::read_item().map_err(|err| format!("{TOOL}: {}", err.message))?;
    }
    object
        .as_ref()
        .and_then(login_from_credentials)
        .ok_or_else(|| format!("{TOOL}: no active Claude Code login"))
}

fn login_from_credentials(object: &Map<String, Value>) -> Option<Login> {
    let oauth = object.get("claudeAiOauth")?.as_object()?;
    let access_token = store::non_empty_str(oauth.get("accessToken"))?.trim();
    Some(Login {
        access_token: access_token.to_string(),
        expires_at_ms: oauth.get("expiresAt").and_then(Value::as_i64),
    })
}

/// Row, result, and cache name: the profile name, or for the active login the
/// profile that holds the same access token (else `active`).
fn target_name(target: &Path) -> String {
    if !is_active_target(target) {
        return target_file_name(target)
            .trim_end_matches(".json")
            .to_string();
    }
    let Ok(active) = read_active_login() else {
        return ACTIVE_NAME.to_string();
    };
    let secret_dir = store::secret_dir().unwrap_or_default();
    let profiles =
        driver::collect_json_targets_from_dir(&CLAUDE_SPEC, &secret_dir, false).unwrap_or_default();
    if let Some(profile) = profiles.iter().find(|profile| {
        read_profile_login(profile).is_ok_and(|login| login.access_token == active.access_token)
    }) {
        return target_name(profile);
    }

    let profile_names = profiles
        .iter()
        .map(|profile| {
            target_file_name(profile)
                .trim_end_matches(".json")
                .to_string()
        })
        .collect::<Vec<_>>();
    if !profile_names.iter().any(|name| name == ACTIVE_NAME) {
        return ACTIVE_NAME.to_string();
    }

    if !profile_names.iter().any(|name| name == "active-login") {
        return "active-login".to_string();
    }
    let mut suffix = 2_u64;
    loop {
        let candidate = format!("active-login-{suffix}");
        if !profile_names.iter().any(|name| name == &candidate) {
            return candidate;
        }
        suffix += 1;
    }
}

fn target_file_name(target: &Path) -> String {
    target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn now_epoch() -> i64 {
    Utc::now().timestamp()
}

fn format_local(epoch: i64, format: &str) -> Option<String> {
    let datetime = Local.timestamp_opt(epoch, 0).single()?;
    Some(datetime.format(format).to_string())
}

fn format_local_datetime(epoch: i64) -> Option<String> {
    format_local(epoch, LOCAL_DATETIME)
}

fn format_local_with_offset(epoch: i64) -> Option<String> {
    format_local(epoch, LOCAL_DATETIME_WITH_OFFSET)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn all_accounts_mark_the_recorded_current_default_like_auth_current() {
        use nils_test_support::{EnvGuard, GlobalStateLock};
        let lock = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let secrets = tmp.path().join("secrets");
        let config = tmp.path().join("config");
        std::fs::create_dir_all(&secrets).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        let credentials = |token: &str| {
            serde_json::json!({ "claudeAiOauth": { "accessToken": token } }).to_string()
        };
        std::fs::write(secrets.join("alpha.json"), credentials("token-alpha")).unwrap();
        std::fs::write(secrets.join("beta.json"), credentials("token-beta")).unwrap();
        // The active login still carries alpha's token, but beta is the
        // recorded current default that `auth current` reports.
        std::fs::write(config.join(".credentials.json"), credentials("token-alpha")).unwrap();
        std::fs::write(secrets.join("current"), "beta\n").unwrap();
        let _secrets = EnvGuard::set(&lock, "CLAUDE_SECRET_DIR", secrets.to_str().unwrap());
        let _config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", config.to_str().unwrap());
        let _keychain = EnvGuard::set(&lock, "CLAUDE_AUTH_KEYCHAIN", "off");
        let targets = vec![secrets.join("alpha.json"), secrets.join("beta.json")];

        assert_eq!(
            ClaudeRateLimits.current_name(&targets).as_deref(),
            Some("beta")
        );
        std::fs::write(secrets.join("current"), "gamma\n").unwrap();
        assert_eq!(ClaudeRateLimits.current_name(&targets), None);
        std::fs::remove_file(secrets.join("current")).unwrap();
        assert_eq!(ClaudeRateLimits.current_name(&targets), None);
    }

    #[test]
    fn http_statuses_map_to_the_diag_reason_codes() {
        let http = |status| RequestFailure::Http {
            status,
            body: "Your subscription payment is past due".to_string(),
        };
        assert_eq!(
            request_failure_reason(&http(401)),
            ProviderUsageReason::AuthExpired
        );
        assert_eq!(
            request_failure_reason(&http(403)),
            ProviderUsageReason::PermissionDenied
        );
        assert_eq!(
            request_failure_reason(&http(429)),
            ProviderUsageReason::RateLimited
        );
        for status in [402, 404, 500, 503] {
            assert_eq!(
                request_failure_reason(&http(status)),
                ProviderUsageReason::ServiceUnavailable
            );
        }
    }

    #[test]
    fn multi_profile_collection_gets_a_progress_bar() {
        let progress = ClaudeRateLimits
            .progress(2, "claude-rate-limits ")
            .expect("progress sink");
        progress.set_message("alpha".to_string());
        progress.inc(1);
        progress.finish_and_clear();
    }

    #[test]
    fn usage_body_maps_claude_windows_and_treats_null_windows_as_benign() {
        let Fetch::Windows {
            values, windows, ..
        } = parse_usage_body(
            r#"{"five_hour":{"utilization":12.6,"resets_at":null},"seven_day":{"utilization":99.5,"resets_at":"2023-11-20T17:06:40Z"}}"#,
        )
        else {
            panic!("expected windows");
        };
        assert_eq!(values.non_weekly.expect("5h").remaining, 87);
        assert_eq!(values.weekly.expect("weekly").reset_epoch, 1_700_500_000);
        assert_eq!(
            serde_json::to_value(&windows).expect("json"),
            serde_json::json!([
                {"label": "5h", "window_minutes": 300, "used_percent": 13, "remaining_percent": 87},
                {"label": "Weekly", "window_minutes": 10080, "used_percent": 100, "remaining_percent": 0, "reset_at_epoch": 1_700_500_000}
            ])
        );
        assert!(matches!(
            parse_usage_body(r#"{"five_hour":null,"seven_day":null}"#),
            Fetch::NoWindow { .. }
        ));
        assert!(matches!(
            parse_usage_body("not json"),
            Fetch::Failed {
                code: "invalid-usage-payload",
                ..
            }
        ));
    }
}

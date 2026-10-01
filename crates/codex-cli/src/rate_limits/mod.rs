use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::auth;
use crate::diag_output;
use crate::provider_profile::CODEX_PROVIDER_PROFILE;
use crate::rate_limits::client::{UsageRequest, fetch_usage_with_reset_credits};
use nils_common::env as shared_env;
use nils_common::fs;
use nils_common::provider_runtime::persistence::{
    SyncSecretsError, TimestampPolicy, sync_auth_to_matching_secrets,
};
use nils_common::provider_usage::ProviderUsageReason;
use nils_common::rate_limits::driver::{self, collect_json_targets_from_dir, no_targets_error};
use nils_common::rate_limits::schema as shared_schema;
#[cfg(test)]
use nils_common::rate_limits::values::parse_one_line_output;
use nils_common::rate_limits::values::{self as shared_values, normalize_one_line};
use nils_common::rate_limits::{
    CacheFallbackPolicy, OneLineFetch, ProgressSink, ProviderSpec, RC_NO_RATE_LIMIT_WINDOW,
    RateLimitResult, RateLimitSummary, RateLimitWindow, RateLimitsProvider, ResetCredits,
    ResetEpochs, RunOptions, TargetDiscoveryError, TargetIdentity,
};
use nils_term::progress::{Progress, ProgressFinish, ProgressOptions};

pub use nils_common::rate_limits_ansi as ansi;
pub mod cache;
pub mod client;
pub mod render;
pub mod writeback;

pub type RateLimitsOptions = RunOptions;

type RateLimitJsonResult = RateLimitResult;
type AsyncFetchResult = OneLineFetch;

const DIAG_SCHEMA_VERSION: &str = "codex-cli.diag.rate-limits.v1";
const DIAG_COMMAND: &str = "diag rate-limits";

static CODEX_SPEC: ProviderSpec = ProviderSpec {
    provider: "codex",
    schema_version: DIAG_SCHEMA_VERSION,
    command: DIAG_COMMAND,
    tool: "codex-rate-limits",
    table_title: "Codex rate limits for all accounts",
    secret_dir_env: "CODEX_SECRET_DIR",
    usage: "codex-rate-limits [-c] [-d] [--cached] [--no-refresh-auth] [--json] [--one-line] [--all] [secret.json]",
    default_all_env: Some("CODEX_RATE_LIMITS_DEFAULT_ALL_ENABLED"),
    watch_max_rounds_env: "CODEX_RATE_LIMITS_WATCH_MAX_ROUNDS",
    watch_interval_env: "CODEX_RATE_LIMITS_WATCH_INTERVAL_SECONDS",
};

fn refresh_on_401_enabled(no_refresh_auth: bool) -> bool {
    !no_refresh_auth && shared_env::env_truthy(CODEX_PROVIDER_PROFILE.env.auto_refresh_enabled)
}

pub fn run(args: &RateLimitsOptions) -> Result<i32> {
    let provider = CodexRateLimits {
        no_refresh_auth: args.no_refresh_auth,
    };
    driver::run(&provider, args)
}

/// Codex's usage client, cache, and secret model for the shared driver.
struct CodexRateLimits {
    no_refresh_auth: bool,
}

struct CodexProgress(Progress);

impl ProgressSink for CodexProgress {
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

impl RateLimitsProvider for CodexRateLimits {
    fn spec(&self) -> &ProviderSpec {
        &CODEX_SPEC
    }

    fn now_epoch(&self) -> i64 {
        Utc::now().timestamp()
    }

    fn format_local(&self, epoch: i64, format: &str) -> Option<String> {
        render::format_epoch_local(epoch, format)
    }

    fn progress(&self, total: usize, prefix: &str) -> Option<Box<dyn ProgressSink>> {
        Some(Box::new(CodexProgress(Progress::new(
            total as u64,
            ProgressOptions::default()
                .with_prefix(prefix)
                .with_finish(ProgressFinish::Clear),
        ))))
    }

    fn secret_dir(&self) -> PathBuf {
        crate::paths::resolve_secret_dir().unwrap_or_default()
    }

    fn json_targets(&self) -> std::result::Result<Vec<PathBuf>, TargetDiscoveryError> {
        collect_secret_files()
    }

    fn identity(&self, target: &Path) -> TargetIdentity {
        codex_identity(target)
    }

    fn clear_cache(&self) -> std::result::Result<(), String> {
        cache::clear_prompt_segment_cache().map_err(|err| err.to_string())
    }

    fn prepare_collection(&self, debug: bool) {
        maybe_sync_all_mode_auth_silent(debug);
    }

    fn run_single(&self, args: &RunOptions, cached: bool, one_line: bool) -> Result<i32> {
        run_single_mode(args, cached, one_line, args.json)
    }

    fn json_result(
        &self,
        target: &Path,
        cached: bool,
        fallback: CacheFallbackPolicy,
    ) -> RateLimitResult {
        collect_json_result_for_secret(target, cached, self.no_refresh_auth, fallback)
    }

    fn async_one_line(&self, target: &Path, cached: bool) -> OneLineFetch {
        let secret_name = target_file_name(target);
        async_fetch_one_line(target, cached, self.no_refresh_auth, &secret_name)
    }

    fn sequential_one_line(&self, target: &Path, cached: bool, debug: bool) -> OneLineFetch {
        let result =
            single_one_line(target, cached, self.no_refresh_auth, debug).unwrap_or_default();
        OneLineFetch {
            line: result.line,
            no_window: result.no_window,
            reset_credits_available: result.reset_credits_available,
            ..Default::default()
        }
    }

    fn row_reset_epochs(
        &self,
        target: &Path,
        cached: bool,
        fill_from_stale_cache: bool,
    ) -> ResetEpochs {
        row_reset_epochs(target, cached, fill_from_stale_cache)
    }

    fn current_name(&self, targets: &[PathBuf]) -> Option<String> {
        current_secret_basename(targets)
    }
}

/// Reset epochs of a filled table row: the cache in `--cached` mode, else the
/// usage the fetch wrote back into the secret file.
fn row_reset_epochs(target: &Path, cached: bool, fill_from_stale_cache: bool) -> ResetEpochs {
    let mut epochs = ResetEpochs::default();
    if cached {
        if let Ok(cache_entry) = cache::read_cache_entry_for_cached_mode(target) {
            epochs.non_weekly = cache_entry.non_weekly_reset_epoch;
            epochs.weekly = cache_entry.weekly_reset_epoch;
        }
        return epochs;
    }
    if let Ok(values) = crate::json::read_json(target) {
        epochs.non_weekly =
            crate::json::i64_at(&values, &["codex_rate_limits", "non_weekly_reset_at_epoch"]);
        epochs.weekly =
            crate::json::i64_at(&values, &["codex_rate_limits", "weekly_reset_at_epoch"]);
    }
    if fill_from_stale_cache
        && (epochs.non_weekly.is_none() || epochs.weekly.is_none())
        && let Ok(cache_read) = cache::read_cache_entry_allow_stale(target)
    {
        let cache_entry = cache_read.entry;
        if epochs.non_weekly.is_none() {
            epochs.non_weekly = cache_entry.non_weekly_reset_epoch;
        }
        if epochs.weekly.is_none() {
            epochs.weekly = cache_entry.weekly_reset_epoch;
        }
    }
    epochs
}

fn codex_identity(target_file: &Path) -> TargetIdentity {
    TargetIdentity {
        provider: "codex".to_string(),
        name: secret_display_name(target_file),
        target_file: target_file_name(target_file),
    }
}

fn emit_single_envelope(ok: bool, result: RateLimitJsonResult) -> Result<()> {
    shared_schema::emit_single_envelope(&CODEX_SPEC, ok, result)
}

fn collect_secret_files() -> std::result::Result<Vec<PathBuf>, TargetDiscoveryError> {
    if std::env::var_os(CODEX_PROVIDER_PROFILE.env.secret_dir).is_some() {
        let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
        return collect_json_targets_from_dir(&CODEX_SPEC, &secret_dir, true);
    }

    let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
    if secret_dir.is_dir()
        && let Ok(secret_files) = collect_json_targets_from_dir(&CODEX_SPEC, &secret_dir, false)
        && !secret_files.is_empty()
    {
        return Ok(secret_files);
    }

    if let Some(auth_file) = existing_active_auth_file() {
        return Ok(vec![auth_file]);
    }

    if let Some(auth_file) = official_codex_auth_file() {
        return Ok(vec![auth_file]);
    }

    Err(no_targets_error(&CODEX_SPEC, &secret_dir))
}

fn existing_active_auth_file() -> Option<PathBuf> {
    crate::paths::resolve_auth_file().filter(|path| path.is_file())
}

fn official_codex_auth_file() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("auth.json"))
        .filter(|path| path.is_file())
    {
        return Some(path);
    }

    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".codex").join("auth.json"))
        .filter(|path| path.is_file())
}

fn is_official_codex_auth_file(target_file: &Path) -> bool {
    official_codex_auth_file().as_deref() == Some(target_file)
}

fn should_writeback_usage(target_file: &Path) -> bool {
    !is_official_codex_auth_file(target_file)
}

fn collect_json_result_for_secret(
    target_file: &Path,
    cached_mode: bool,
    no_refresh_auth: bool,
    cache_fallback: CacheFallbackPolicy,
) -> RateLimitJsonResult {
    if cached_mode {
        return collect_json_from_cache(target_file, "cache", true);
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);
    let usage_request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: refresh_on_401_enabled(no_refresh_auth),
        suppress_auth_refresh_output: false,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    match fetch_usage_with_reset_credits(&usage_request) {
        Ok(usage) => {
            if should_writeback_usage(target_file)
                && let Err(err) = writeback::write_weekly(target_file, &usage.json)
            {
                return json_result_error(
                    target_file,
                    "network",
                    "writeback-failed",
                    err.to_string(),
                    None,
                );
            }
            if is_auth_file(target_file)
                && let Ok(sync_rc) = auth::sync::run_with_json(false)
                && sync_rc != 0
            {
                return json_result_error(
                    target_file,
                    "network",
                    "sync-failed",
                    "codex-rate-limits: failed to sync auth after usage fetch".to_string(),
                    None,
                );
            }
            match summary_and_windows_from_usage(&usage.json) {
                Some((summary, windows)) => {
                    let fetched_at_epoch = Utc::now().timestamp();
                    if fetched_at_epoch > 0 {
                        let usage_data = render::parse_usage(&usage.json)
                            .expect("summary requires parsed usage");
                        let values = render::render_values(&usage_data);
                        let weekly = render::weekly_values(&values);
                        let _ = cache::write_prompt_segment_cache(
                            target_file,
                            fetched_at_epoch,
                            &weekly,
                        );
                    }
                    RateLimitJsonResult {
                        provider: "codex".to_string(),
                        name: secret_display_name(target_file),
                        target_file: target_file_name(target_file),
                        status: "ok".to_string(),
                        ok: true,
                        source: "network".to_string(),
                        reason_code: None,
                        summary: Some(summary),
                        windows: Some(windows),
                        reset_credits: reset_credits_from_usage(&usage.json),
                        limit_resets: None,
                        raw_usage: Some(project_safe_usage_json(&usage.json)),
                        error: None,
                    }
                }
                None if render::rate_limit_has_no_windows(&usage.json) => {
                    // Benign: the backend reports no active window. Prefer the
                    // last-known cached values (stale allowed), else report the
                    // empty window as a success rather than an error.
                    if matches!(
                        cache_fallback,
                        CacheFallbackPolicy::NoWindow | CacheFallbackPolicy::AnyFailure
                    ) {
                        let mut fallback =
                            collect_json_from_cache(target_file, "cache-fallback", false);
                        if fallback.ok {
                            fallback.reset_credits = reset_credits_from_usage(&usage.json);
                            return fallback;
                        }
                    }
                    json_result_no_window(target_file, reset_credits_from_usage(&usage.json))
                }
                None => json_result_error(
                    target_file,
                    "network",
                    "invalid-usage-payload",
                    "codex-rate-limits: invalid usage payload".to_string(),
                    Some(serde_json::json!({
                        "raw_usage": project_safe_usage_json(&usage.json),
                    })),
                ),
            }
        }
        Err(err) => {
            if cache_fallback == CacheFallbackPolicy::AnyFailure {
                let fallback = collect_json_from_cache(target_file, "cache-fallback", false);
                if fallback.ok {
                    return fallback;
                }
            }
            let reason = err.reason();
            let code = if reason == ProviderUsageReason::AuthRequired {
                "missing-access-token"
            } else {
                "request-failed"
            };
            json_result_error_with_reason(
                target_file,
                "network",
                code,
                err.to_string(),
                None,
                Some(reason),
            )
        }
    }
}

fn collect_json_from_cache(
    target_file: &Path,
    source: &str,
    enforce_ttl: bool,
) -> RateLimitJsonResult {
    let cache_entry = if enforce_ttl {
        cache::read_cache_entry_for_cached_mode(target_file)
    } else {
        cache::read_cache_entry_allow_stale(target_file).map(|read| read.entry)
    };

    match cache_entry {
        Ok(entry) => RateLimitJsonResult::from_cache(
            codex_identity(target_file),
            source,
            &entry,
            render::format_epoch_local_datetime_with_offset,
        ),
        Err(err) => json_result_error(
            target_file,
            source,
            "cache-read-failed",
            err.to_string(),
            None,
        ),
    }
}

fn json_result_error(
    target_file: &Path,
    source: &str,
    code: &str,
    message: String,
    details: Option<Value>,
) -> RateLimitJsonResult {
    json_result_error_with_reason(target_file, source, code, message, details, None)
}

fn json_result_error_with_reason(
    target_file: &Path,
    source: &str,
    code: &str,
    message: String,
    details: Option<Value>,
    reason_code: Option<ProviderUsageReason>,
) -> RateLimitJsonResult {
    RateLimitJsonResult::error(
        codex_identity(target_file),
        source,
        code,
        message,
        details,
        reason_code,
    )
}

/// Benign "no active rate-limit window" result for a `rate_limit: null` payload
/// with no cache to fall back to. Reported as a success, not an error.
fn json_result_no_window(
    target_file: &Path,
    reset_credits: Option<ResetCredits>,
) -> RateLimitJsonResult {
    RateLimitJsonResult::no_window(codex_identity(target_file), reset_credits)
}

pub(crate) fn secret_display_name(target_file: &Path) -> String {
    cache::secret_name_for_target(target_file).unwrap_or_else(|| {
        target_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .trim_end_matches(".json")
            .to_string()
    })
}

pub(crate) fn target_file_name(target_file: &Path) -> String {
    target_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

pub(crate) fn reset_credits_available_count(usage_json: &Value) -> Option<i64> {
    usage_json
        .get("rate_limit_reset_credits")?
        .get("available_count")?
        .as_i64()
        .filter(|value| *value >= 0)
}

fn reset_credits_from_usage(usage_json: &Value) -> Option<ResetCredits> {
    reset_credits_available_count(usage_json)
        .map(|available_count| ResetCredits { available_count })
}

fn summary_and_windows_from_usage(
    usage_json: &Value,
) -> Option<(RateLimitSummary, Vec<RateLimitWindow>)> {
    let usage_data = render::parse_usage(usage_json)?;
    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    let summary = summary_from_weekly_values(&weekly);
    let windows = windows_from_usage_values(&usage_data, &values);
    (!windows.is_empty()).then_some((summary, windows))
}

fn summary_from_weekly_values(weekly: &render::WeeklyValues) -> RateLimitSummary {
    shared_schema::summary_from_weekly_values(
        weekly,
        render::format_epoch_local_datetime_with_offset,
    )
}

fn windows_from_usage_values(
    usage_data: &render::UsageData,
    values: &render::RenderValues,
) -> Vec<RateLimitWindow> {
    [
        (&usage_data.primary, &values.primary),
        (&usage_data.secondary, &values.secondary),
    ]
    .into_iter()
    .filter_map(|(window, rendered)| {
        let window = window.as_ref()?;
        let rendered = rendered.as_ref()?;
        Some(RateLimitWindow {
            label: rendered.label.clone(),
            window_minutes: None,
            used_percent: shared_schema::percent_i64(window.used_percent),
            remaining_percent: rendered.remaining,
            reset_at_epoch: Some(rendered.reset_epoch).filter(|epoch| *epoch > 0),
        })
    })
    .collect()
}

fn project_safe_usage_json(value: &Value) -> Value {
    let Some(source) = value.as_object() else {
        return Value::Object(serde_json::Map::new());
    };
    let mut projected = serde_json::Map::new();
    if let Some(plan_type) = source
        .get("plan_type")
        .and_then(Value::as_str)
        .filter(|value| is_safe_plan_type(value))
    {
        projected.insert(
            "plan_type".to_string(),
            Value::String(plan_type.to_string()),
        );
    }
    for key in ["rate_limit", "code_review_rate_limit"] {
        if let Some(rate_limit) = source.get(key)
            && let Some(safe) = project_safe_rate_limit(rate_limit)
        {
            projected.insert(key.to_string(), safe);
        }
    }
    Value::Object(projected)
}

fn is_safe_plan_type(value: &str) -> bool {
    matches!(
        value,
        "free" | "go" | "plus" | "pro" | "team" | "business" | "enterprise" | "edu"
    )
}

fn project_safe_rate_limit(value: &Value) -> Option<Value> {
    if value.is_null() {
        return Some(Value::Null);
    }
    let source = value.as_object()?;
    let mut projected = serde_json::Map::new();
    for key in ["allowed", "limit_reached"] {
        if let Some(value) = source.get(key).and_then(Value::as_bool) {
            projected.insert(key.to_string(), Value::Bool(value));
        }
    }
    for key in ["primary_window", "secondary_window"] {
        if let Some(window) = source.get(key)
            && let Some(safe) = project_safe_window(window)
        {
            projected.insert(key.to_string(), safe);
        }
    }
    Some(Value::Object(projected))
}

fn project_safe_window(value: &Value) -> Option<Value> {
    if value.is_null() {
        return Some(Value::Null);
    }
    let source = value.as_object()?;
    let mut projected = serde_json::Map::new();
    for key in ["limit_window_seconds", "used_percent", "reset_at"] {
        if let Some(value) = source.get(key)
            && value.is_number()
        {
            projected.insert(key.to_string(), value.clone());
        }
    }
    Some(Value::Object(projected))
}

#[cfg(test)]
fn collect_secret_files_for_async_text() -> std::result::Result<Vec<PathBuf>, String> {
    let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
    driver::collect_async_text_targets(&CODEX_SPEC, &secret_dir)
}

fn async_fetch_one_line(
    target_file: &Path,
    cached_mode: bool,
    no_refresh_auth: bool,
    secret_name: &str,
) -> AsyncFetchResult {
    if cached_mode {
        return fetch_one_line_cached(target_file);
    }

    let mut attempt = 1;
    let max_attempts = 2;
    let mut network_err: Option<String> = None;

    let mut result = fetch_one_line_network(target_file, no_refresh_auth);
    if !result.err.is_empty() {
        network_err = Some(result.err.clone());
    }

    while attempt < max_attempts && result.rc == 3 {
        thread::sleep(Duration::from_millis(250));
        let next = fetch_one_line_network(target_file, no_refresh_auth);
        if !next.err.is_empty() {
            network_err = Some(next.err.clone());
        }
        result = next;
        attempt += 1;
        if result.rc != 3 {
            break;
        }
    }

    let mut errors: Vec<String> = Vec::new();
    if let Some(err) = network_err {
        errors.push(err);
    }

    // A null window is benign — the backend simply has no usage recorded in the
    // current window. Degrade to the last-known cached values (marked stale)
    // rather than surfacing it as a failure.
    let no_window_live = result.rc == RC_NO_RATE_LIMIT_WINDOW;
    let live_reset_credits = result.reset_credits_available;

    let missing_line = result
        .line
        .as_ref()
        .map(|line| line.trim().is_empty())
        .unwrap_or(true);

    if result.rc != 0 || missing_line {
        let cached = fetch_one_line_cached_allow_stale(target_file);
        if !cached.err.is_empty() {
            errors.push(cached.err.clone());
        }
        if cached.rc == 0
            && cached
                .line
                .as_ref()
                .map(|line| !line.trim().is_empty())
                .unwrap_or(false)
        {
            // Only annotate the fallback for genuine failures; a null window is
            // an expected, quiet condition.
            if result.rc != 0 && !no_window_live {
                let _ = secret_name;
                errors.push(format!(
                    "codex-rate-limits-async: falling back to cache (rc={})",
                    result.rc
                ));
            }
            result = AsyncFetchResult {
                line: cached.line,
                stale: cached.stale,
                reset_credits_available: live_reset_credits,
                ..Default::default()
            };
        } else if no_window_live {
            // No cache to borrow from, but a null window is still benign.
            result = AsyncFetchResult {
                rc: RC_NO_RATE_LIMIT_WINDOW,
                no_window: true,
                reset_credits_available: live_reset_credits,
                ..Default::default()
            };
        }
    }

    let line = result.line.map(normalize_one_line);
    let err = errors.join("\n");
    AsyncFetchResult {
        line,
        rc: result.rc,
        err,
        stale: result.stale,
        no_window: result.no_window,
        reset_credits_available: result.reset_credits_available,
    }
}

fn fetch_one_line_network(target_file: &Path, no_refresh_auth: bool) -> AsyncFetchResult {
    if !target_file.is_file() {
        return AsyncFetchResult {
            line: None,
            rc: 1,
            err: format!("codex-rate-limits: {} not found", target_file.display()),
            ..Default::default()
        };
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: refresh_on_401_enabled(no_refresh_auth),
        suppress_auth_refresh_output: false,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage_with_reset_credits(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("missing access_token") {
                return AsyncFetchResult {
                    line: None,
                    rc: 2,
                    err: format!(
                        "codex-rate-limits: missing access_token in {}",
                        target_file.display()
                    ),
                    ..Default::default()
                };
            }
            return AsyncFetchResult {
                line: None,
                rc: 3,
                err: msg,
                ..Default::default()
            };
        }
    };

    if should_writeback_usage(target_file)
        && let Err(err) = writeback::write_weekly(target_file, &usage.json)
    {
        return AsyncFetchResult {
            line: None,
            rc: 4,
            err: err.to_string(),
            ..Default::default()
        };
    }

    if is_auth_file(target_file) {
        match sync_auth_silent() {
            Ok((sync_rc, sync_err)) => {
                if sync_rc != 0 {
                    return AsyncFetchResult {
                        line: None,
                        rc: 5,
                        err: sync_err.unwrap_or_default(),
                        ..Default::default()
                    };
                }
            }
            Err(_) => {
                return AsyncFetchResult {
                    line: None,
                    rc: 1,
                    ..Default::default()
                };
            }
        }
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None => {
            if render::rate_limit_has_no_windows(&usage.json) {
                return AsyncFetchResult {
                    rc: RC_NO_RATE_LIMIT_WINDOW,
                    reset_credits_available: reset_credits_available_count(&usage.json),
                    ..Default::default()
                };
            }
            return AsyncFetchResult {
                line: None,
                rc: 3,
                err: "codex-rate-limits: invalid usage payload".to_string(),
                ..Default::default()
            };
        }
    };

    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    if weekly.weekly.is_none() && weekly.non_weekly.is_none() {
        return AsyncFetchResult {
            rc: RC_NO_RATE_LIMIT_WINDOW,
            no_window: true,
            reset_credits_available: reset_credits_available_count(&usage.json),
            ..Default::default()
        };
    }

    let fetched_at_epoch = Utc::now().timestamp();
    if fetched_at_epoch > 0 {
        let _ = cache::write_prompt_segment_cache(target_file, fetched_at_epoch, &weekly);
    }

    AsyncFetchResult {
        line: format_one_line_output(
            weekly
                .non_weekly
                .as_ref()
                .map(|window| window.label.as_str()),
            weekly.non_weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.reset_epoch),
        ),
        rc: 0,
        reset_credits_available: reset_credits_available_count(&usage.json),
        ..Default::default()
    }
}

fn fetch_one_line_cached(target_file: &Path) -> AsyncFetchResult {
    match cache::read_cache_entry_for_cached_mode(target_file) {
        Ok(entry) => AsyncFetchResult {
            line: format_one_line_output(
                entry.non_weekly_label.as_deref(),
                entry.non_weekly_remaining,
                entry.weekly_remaining,
                entry.weekly_reset_epoch,
            ),
            rc: 0,
            ..Default::default()
        },
        Err(err) => AsyncFetchResult {
            line: None,
            rc: 1,
            err: err.to_string(),
            ..Default::default()
        },
    }
}

/// Reads the cached one-line value without enforcing the freshness TTL, marking
/// the result `stale` when it is past the TTL. Used as the diag-path fallback
/// so a transient null/failed live fetch degrades to the last-known values.
fn fetch_one_line_cached_allow_stale(target_file: &Path) -> AsyncFetchResult {
    match cache::read_cache_entry_allow_stale(target_file) {
        Ok(read) => AsyncFetchResult {
            line: format_one_line_output(
                read.entry.non_weekly_label.as_deref(),
                read.entry.non_weekly_remaining,
                read.entry.weekly_remaining,
                read.entry.weekly_reset_epoch,
            ),
            rc: 0,
            stale: read.stale,
            ..Default::default()
        },
        Err(err) => AsyncFetchResult {
            line: None,
            rc: 1,
            err: err.to_string(),
            ..Default::default()
        },
    }
}

fn format_one_line_output(
    non_weekly_label: Option<&str>,
    non_weekly_remaining: Option<i64>,
    weekly_remaining: Option<i64>,
    weekly_reset_epoch: Option<i64>,
) -> Option<String> {
    shared_values::format_one_line_output(
        non_weekly_label,
        non_weekly_remaining,
        weekly_remaining,
        weekly_reset_epoch,
        render::format_epoch_local_datetime,
    )
}

fn sync_auth_silent() -> Result<(i32, Option<String>)> {
    let auth_file = match crate::paths::resolve_auth_file() {
        Some(path) => path,
        None => return Ok((0, None)),
    };

    let sync_result = match sync_auth_to_matching_secrets(
        &CODEX_PROVIDER_PROFILE,
        &auth_file,
        fs::SECRET_FILE_MODE,
        TimestampPolicy::Strict,
    ) {
        Ok(result) => result,
        Err(SyncSecretsError::HashAuthFile { path, .. })
        | Err(SyncSecretsError::HashSecretFile { path, .. }) => {
            return Ok((1, Some(format!("codex: failed to hash {}", path.display()))));
        }
        Err(err) => return Err(err.into()),
    };
    if !sync_result.auth_file_present || !sync_result.auth_identity_present {
        return Ok((0, None));
    }

    Ok((0, None))
}

fn maybe_sync_all_mode_auth_silent(debug_mode: bool) {
    match sync_auth_silent() {
        Ok((0, _)) => {}
        Ok((_, sync_err)) => {
            if debug_mode
                && let Some(message) = sync_err
                && !message.trim().is_empty()
            {
                eprintln!("{message}");
            }
        }
        Err(err) => {
            if debug_mode {
                eprintln!("codex-rate-limits: failed to sync auth and secrets: {err}");
            }
        }
    }
}

pub(crate) fn current_secret_basename(secret_files: &[PathBuf]) -> Option<String> {
    let auth_file = crate::paths::resolve_auth_file()?;
    if !auth_file.is_file() {
        return None;
    }

    let auth_key = auth::identity_key_from_auth_file(&auth_file).ok().flatten();
    let auth_hash = fs::sha256_file(&auth_file).ok();

    if let Some(auth_hash) = auth_hash.as_deref() {
        for secret_file in secret_files {
            if let Ok(secret_hash) = fs::sha256_file(secret_file)
                && secret_hash == auth_hash
                && let Some(name) = secret_file.file_name().and_then(|name| name.to_str())
            {
                return Some(name.trim_end_matches(".json").to_string());
            }
        }
    }

    if let Some(auth_key) = auth_key.as_deref() {
        for secret_file in secret_files {
            if let Ok(Some(candidate_key)) = auth::identity_key_from_auth_file(secret_file)
                && candidate_key == auth_key
                && let Some(name) = secret_file.file_name().and_then(|name| name.to_str())
            {
                return Some(name.trim_end_matches(".json").to_string());
            }
        }
    }

    None
}

fn run_single_mode(
    args: &RateLimitsOptions,
    cached_mode: bool,
    one_line: bool,
    output_json: bool,
) -> Result<i32> {
    let target_file = match resolve_target(args.secret.as_deref()) {
        Ok(path) => path,
        Err(code) => return Ok(code),
    };

    if !target_file.is_file() {
        if output_json {
            diag_output::emit_error(
                DIAG_SCHEMA_VERSION,
                DIAG_COMMAND,
                "target-not-found",
                format!("codex-rate-limits: {} not found", target_file.display()),
                Some(serde_json::json!({
                    "target_file": target_file.display().to_string(),
                })),
            )?;
        } else {
            eprintln!("codex-rate-limits: {} not found", target_file.display());
        }
        return Ok(1);
    }

    if cached_mode {
        match cache::read_cache_entry_for_cached_mode(&target_file) {
            Ok(entry) => {
                if let Some(line) = format_one_line_output(
                    entry.non_weekly_label.as_deref(),
                    entry.non_weekly_remaining,
                    entry.weekly_remaining,
                    entry.weekly_reset_epoch,
                ) {
                    println!("{line}");
                }
                return Ok(0);
            }
            Err(err) => {
                eprintln!("{err}");
                return Ok(1);
            }
        }
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.clone(),
        refresh_on_401: refresh_on_401_enabled(args.no_refresh_auth),
        suppress_auth_refresh_output: false,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage_with_reset_credits(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            let reason = err.reason();
            let msg = err.to_string();
            if reason == ProviderUsageReason::AuthRequired {
                if output_json {
                    diag_output::emit_error(
                        DIAG_SCHEMA_VERSION,
                        DIAG_COMMAND,
                        "missing-access-token",
                        format!(
                            "codex-rate-limits: missing access_token in {}",
                            target_file.display()
                        ),
                        Some(serde_json::json!({
                            "target_file": target_file.display().to_string(),
                            "reason_code": reason.as_str(),
                        })),
                    )?;
                } else {
                    eprintln!(
                        "codex-rate-limits: missing access_token in {}",
                        target_file.display()
                    );
                }
                return Ok(2);
            }
            if output_json {
                diag_output::emit_error(
                    DIAG_SCHEMA_VERSION,
                    DIAG_COMMAND,
                    "request-failed",
                    msg,
                    Some(serde_json::json!({
                        "target_file": target_file.display().to_string(),
                        "reason_code": reason.as_str(),
                    })),
                )?;
            } else {
                eprintln!("{msg}");
            }
            return Ok(3);
        }
    };

    if should_writeback_usage(&target_file)
        && let Err(err) = writeback::write_weekly(&target_file, &usage.json)
    {
        if output_json {
            diag_output::emit_error(
                DIAG_SCHEMA_VERSION,
                DIAG_COMMAND,
                "writeback-failed",
                err.to_string(),
                Some(serde_json::json!({
                    "target_file": target_file.display().to_string(),
                })),
            )?;
        } else {
            eprintln!("{err}");
        }
        return Ok(4);
    }

    if is_auth_file(&target_file) {
        let sync_rc = auth::sync::run_with_json(false)?;
        if sync_rc != 0 {
            if output_json {
                diag_output::emit_error(
                    DIAG_SCHEMA_VERSION,
                    DIAG_COMMAND,
                    "sync-failed",
                    "codex-rate-limits: failed to sync auth file",
                    Some(serde_json::json!({
                        "target_file": target_file.display().to_string(),
                    })),
                )?;
            }
            return Ok(5);
        }
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None => {
            if render::rate_limit_has_no_windows(&usage.json) {
                return emit_single_no_window(
                    &target_file,
                    output_json,
                    one_line,
                    reset_credits_from_usage(&usage.json),
                );
            }
            if output_json {
                diag_output::emit_error(
                    DIAG_SCHEMA_VERSION,
                    DIAG_COMMAND,
                    "invalid-usage-payload",
                    "codex-rate-limits: invalid usage payload",
                    Some(serde_json::json!({
                        "target_file": target_file.display().to_string(),
                        "raw_usage": project_safe_usage_json(&usage.json),
                    })),
                )?;
            } else {
                eprintln!("codex-rate-limits: invalid usage payload");
            }
            return Ok(3);
        }
    };

    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    if weekly.weekly.is_none() && weekly.non_weekly.is_none() {
        return emit_single_no_window(
            &target_file,
            output_json,
            one_line,
            reset_credits_from_usage(&usage.json),
        );
    }

    let fetched_at_epoch = Utc::now().timestamp();
    if fetched_at_epoch > 0 {
        let _ = cache::write_prompt_segment_cache(&target_file, fetched_at_epoch, &weekly);
    }

    if output_json {
        let windows = windows_from_usage_values(&usage_data, &values);
        let result = RateLimitJsonResult {
            provider: "codex".to_string(),
            name: secret_display_name(&target_file),
            target_file: target_file_name(&target_file),
            status: "ok".to_string(),
            ok: true,
            source: "network".to_string(),
            reason_code: None,
            summary: Some(summary_from_weekly_values(&weekly)),
            windows: Some(windows),
            reset_credits: reset_credits_from_usage(&usage.json),
            limit_resets: None,
            raw_usage: Some(project_safe_usage_json(&usage.json)),
            error: None,
        };
        emit_single_envelope(true, result)?;
        return Ok(0);
    }

    if one_line {
        if let Some(line) = format_one_line_output(
            weekly
                .non_weekly
                .as_ref()
                .map(|window| window.label.as_str()),
            weekly.non_weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.reset_epoch),
        ) {
            println!("{line}");
        }
        return Ok(0);
    }

    println!("Rate limits remaining");
    for window in [&values.primary, &values.secondary].into_iter().flatten() {
        let reset = render::format_epoch_local_datetime(window.reset_epoch)
            .unwrap_or_else(|| "?".to_string());
        println!("{} {}% • {}", window.label, window.remaining, reset);
    }
    if let Some(count) = reset_credits_available_count(&usage.json) {
        println!("Earned resets available: {count}");
    }

    Ok(0)
}

/// Renders a benign `rate_limit: null` response in single mode: serve the
/// last-known cached values (marked stale) when available, otherwise report
/// "no active rate-limit window" as a success rather than a malformed payload.
fn emit_single_no_window(
    target_file: &Path,
    output_json: bool,
    one_line: bool,
    reset_credits: Option<ResetCredits>,
) -> Result<i32> {
    if let Ok(read) = cache::read_cache_entry_allow_stale(target_file) {
        if output_json {
            let mut result = collect_json_from_cache(target_file, "cache-fallback", false);
            result.reset_credits = reset_credits.clone();
            let ok = result.ok;
            emit_single_envelope(ok, result)?;
            return Ok(0);
        }

        let entry = read.entry;
        let stale_suffix = if read.stale { " (stale)" } else { "" };
        if one_line {
            if let Some(line) = format_one_line_output(
                entry.non_weekly_label.as_deref(),
                entry.non_weekly_remaining,
                entry.weekly_remaining,
                entry.weekly_reset_epoch,
            ) {
                println!("{line}{stale_suffix}");
            }
            return Ok(0);
        }

        println!("Rate limits remaining{stale_suffix}");
        if let (Some(label), Some(remaining)) = (
            entry.non_weekly_label.as_deref(),
            entry.non_weekly_remaining,
        ) {
            let reset = entry
                .non_weekly_reset_epoch
                .and_then(render::format_epoch_local_datetime)
                .unwrap_or_else(|| "?".to_string());
            println!("{label} {remaining}% • {reset}");
        }
        if let (Some(remaining), Some(reset_epoch)) =
            (entry.weekly_remaining, entry.weekly_reset_epoch)
        {
            let reset =
                render::format_epoch_local_datetime(reset_epoch).unwrap_or_else(|| "?".to_string());
            println!("Weekly {remaining}% • {reset}");
        }
        if let Some(reset_credits) = reset_credits {
            println!("Earned resets available: {}", reset_credits.available_count);
        }
        return Ok(0);
    }

    if output_json {
        let result = json_result_no_window(target_file, reset_credits);
        emit_single_envelope(true, result)?;
        return Ok(0);
    }

    println!("No active rate-limit window");
    if let Some(reset_credits) = reset_credits {
        println!("Earned resets available: {}", reset_credits.available_count);
    }
    Ok(0)
}

#[derive(Default)]
struct SingleOneLineResult {
    line: Option<String>,
    no_window: bool,
    reset_credits_available: Option<i64>,
}

fn single_one_line(
    target_file: &Path,
    cached_mode: bool,
    no_refresh_auth: bool,
    debug_mode: bool,
) -> Result<SingleOneLineResult> {
    if !target_file.is_file() {
        if debug_mode {
            eprintln!("codex-rate-limits: target file not found");
        }
        return Ok(SingleOneLineResult::default());
    }

    if cached_mode {
        return match cache::read_cache_entry_for_cached_mode(target_file) {
            Ok(entry) => Ok(SingleOneLineResult {
                line: format_one_line_output(
                    entry.non_weekly_label.as_deref(),
                    entry.non_weekly_remaining,
                    entry.weekly_remaining,
                    entry.weekly_reset_epoch,
                ),
                no_window: false,
                reset_credits_available: None,
            }),
            Err(err) => {
                if debug_mode {
                    eprintln!("{err}");
                }
                Ok(SingleOneLineResult::default())
            }
        };
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);

    let usage_request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: refresh_on_401_enabled(no_refresh_auth),
        suppress_auth_refresh_output: false,
        base_url,
        connect_timeout_seconds: connect_timeout,
        max_time_seconds: max_time,
    };

    let usage = match fetch_usage_with_reset_credits(&usage_request) {
        Ok(value) => value,
        Err(err) => {
            if debug_mode {
                eprintln!("{err}");
            }
            return Ok(SingleOneLineResult::default());
        }
    };

    if should_writeback_usage(target_file) {
        let _ = writeback::write_weekly(target_file, &usage.json);
    }
    if is_auth_file(target_file) {
        let _ = auth::sync::run();
    }

    let usage_data = match render::parse_usage(&usage.json) {
        Some(value) => value,
        None if render::rate_limit_has_no_windows(&usage.json) => {
            return Ok(single_one_line_no_window(
                target_file,
                reset_credits_available_count(&usage.json),
            ));
        }
        None => return Ok(SingleOneLineResult::default()),
    };
    let values = render::render_values(&usage_data);
    let weekly = render::weekly_values(&values);
    if weekly.weekly.is_none() && weekly.non_weekly.is_none() {
        return Ok(single_one_line_no_window(
            target_file,
            reset_credits_available_count(&usage.json),
        ));
    }
    let fetched_at_epoch = Utc::now().timestamp();
    if fetched_at_epoch > 0 {
        let _ = cache::write_prompt_segment_cache(target_file, fetched_at_epoch, &weekly);
    }
    Ok(SingleOneLineResult {
        line: format_one_line_output(
            weekly
                .non_weekly
                .as_ref()
                .map(|window| window.label.as_str()),
            weekly.non_weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.remaining),
            weekly.weekly.as_ref().map(|window| window.reset_epoch),
        ),
        no_window: false,
        reset_credits_available: reset_credits_available_count(&usage.json),
    })
}

fn single_one_line_no_window(
    target_file: &Path,
    reset_credits_available: Option<i64>,
) -> SingleOneLineResult {
    let line = cache::read_cache_entry_allow_stale(target_file)
        .ok()
        .and_then(|read| {
            format_one_line_output(
                read.entry.non_weekly_label.as_deref(),
                read.entry.non_weekly_remaining,
                read.entry.weekly_remaining,
                read.entry.weekly_reset_epoch,
            )
        });
    SingleOneLineResult {
        line,
        no_window: true,
        reset_credits_available,
    }
}

pub(crate) fn resolve_target(secret: Option<&str>) -> std::result::Result<PathBuf, i32> {
    if let Some(secret_name) = secret {
        if secret_name.is_empty() || secret_name.contains('/') || secret_name.contains("..") {
            eprintln!("codex-rate-limits: invalid secret file name");
            return Err(64);
        }
        let secret_dir = crate::paths::resolve_secret_dir().unwrap_or_default();
        return Ok(secret_dir.join(secret_name));
    }

    if let Some(auth_file) = existing_active_auth_file() {
        return Ok(auth_file);
    }

    if let Some(auth_file) = official_codex_auth_file() {
        return Ok(auth_file);
    }

    if let Some(auth_file) = crate::paths::resolve_auth_file() {
        return Ok(auth_file);
    }

    Err(1)
}

fn is_auth_file(target_file: &Path) -> bool {
    if let Some(auth_file) = crate::paths::resolve_auth_file() {
        return auth_file == target_file;
    }
    false
}

fn env_timeout(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::{
        async_fetch_one_line, cache, collect_json_from_cache, collect_secret_files,
        collect_secret_files_for_async_text, current_secret_basename, env_timeout,
        fetch_one_line_cached, is_auth_file, normalize_one_line, parse_one_line_output,
        project_safe_usage_json, render, resolve_target, secret_display_name, single_one_line,
        sync_auth_silent, target_file_name,
    };
    use chrono::Utc;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    const HEADER: &str = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0";
    const PAYLOAD_ALPHA: &str = "eyJzdWIiOiJ1c2VyXzEyMyIsImVtYWlsIjoiYWxwaGFAZXhhbXBsZS5jb20iLCJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF91c2VyX2lkIjoidXNlcl8xMjMiLCJlbWFpbCI6ImFscGhhQGV4YW1wbGUuY29tIn19";
    const PAYLOAD_BETA: &str = "eyJzdWIiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSIsImh0dHBzOi8vYXBpLm9wZW5haS5jb20vYXV0aCI6eyJjaGF0Z3B0X3VzZXJfaWQiOiJ1c2VyXzQ1NiIsImVtYWlsIjoiYmV0YUBleGFtcGxlLmNvbSJ9fQ";

    fn token(payload: &str) -> String {
        format!("{HEADER}.{payload}.sig")
    }

    fn auth_json(
        payload: &str,
        account_id: &str,
        refresh_token: &str,
        last_refresh: &str,
    ) -> String {
        format!(
            r#"{{"tokens":{{"access_token":"{}","id_token":"{}","refresh_token":"{}","account_id":"{}"}},"last_refresh":"{}"}}"#,
            token(payload),
            token(payload),
            refresh_token,
            account_id,
            last_refresh
        )
    }

    fn fresh_fetched_at() -> i64 {
        Utc::now().timestamp()
    }

    fn complete_weekly_values(
        non_weekly_label: &str,
        non_weekly_remaining: i64,
        weekly_remaining: i64,
        weekly_reset_epoch: i64,
        non_weekly_reset_epoch: Option<i64>,
    ) -> render::WeeklyValues {
        render::WeeklyValues {
            weekly: Some(render::WindowValues {
                label: "Weekly".to_string(),
                remaining: weekly_remaining,
                reset_epoch: weekly_reset_epoch,
            }),
            non_weekly: Some(render::WindowValues {
                label: non_weekly_label.to_string(),
                remaining: non_weekly_remaining,
                reset_epoch: non_weekly_reset_epoch.unwrap_or_default(),
            }),
        }
    }

    #[test]
    fn project_safe_usage_json_keeps_only_known_usage_fields() {
        let input = json!({
            "plan_type": "pro",
            "rate_limit": {
                "allowed": true,
                "primary_window": {
                    "limit_window_seconds": 604800,
                    "used_percent": 12,
                    "reset_at": 1700600000,
                    "email": "private@example.com"
                }
            },
            "tokens": {
                "access_token": "a",
                "refresh_token": "b",
                "nested": {
                    "id_token": "c",
                    "Authorization": "Bearer x",
                    "ok": 1
                }
            },
            "items": [
                {"authorization": "Bearer y", "value": 2}
            ],
            "safe": true
        });

        let projected = project_safe_usage_json(&input);
        assert_eq!(projected["plan_type"], "pro");
        assert_eq!(projected["rate_limit"]["allowed"], true);
        assert_eq!(
            projected["rate_limit"]["primary_window"]["limit_window_seconds"],
            604800
        );
        assert!(projected.get("tokens").is_none());
        assert!(projected.get("items").is_none());
        assert!(projected.get("safe").is_none());
        assert!(
            projected["rate_limit"]["primary_window"]
                .get("email")
                .is_none()
        );
    }

    #[test]
    fn collect_secret_files_reports_missing_secret_dir() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let missing = dir.path().join("missing");
        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            missing.to_str().expect("missing path"),
        );

        let err = collect_secret_files().expect_err("expected missing dir error");
        assert_eq!(err.0, 1);
        assert!(err.1.contains("CODEX_SECRET_DIR not found"));
    }

    #[test]
    fn collect_secret_files_returns_sorted_json_files_only() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secrets = dir.path().join("secrets");
        fs::create_dir_all(&secrets).expect("secrets dir");
        fs::write(secrets.join("beta.json"), "{}").expect("write beta");
        fs::write(secrets.join("alpha.json"), "{}").expect("write alpha");
        fs::write(secrets.join("note.txt"), "ignore").expect("write note");
        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secrets.to_str().expect("secrets path"),
        );

        let files = collect_secret_files().expect("secret files");
        assert_eq!(files.len(), 2);
        assert_eq!(
            files[0].file_name().and_then(|name| name.to_str()),
            Some("alpha.json")
        );
        assert_eq!(
            files[1].file_name().and_then(|name| name.to_str()),
            Some("beta.json")
        );
    }

    #[test]
    fn collect_secret_files_for_async_text_allows_empty_secret_dir() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        fs::create_dir_all(&secret_dir).expect("secret dir");
        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );

        let files = collect_secret_files_for_async_text().expect("async text secret files");
        assert!(files.is_empty());
    }

    #[test]
    fn rate_limits_helper_env_timeout_supports_default_and_parse() {
        let lock = GlobalStateLock::new();
        let key = "CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS";

        let _removed = EnvGuard::remove(&lock, key);
        assert_eq!(env_timeout(key, 7), 7);

        let _set = EnvGuard::set(&lock, key, "11");
        assert_eq!(env_timeout(key, 7), 11);

        let _invalid = EnvGuard::set(&lock, key, "oops");
        assert_eq!(env_timeout(key, 7), 7);
    }

    #[test]
    fn rate_limits_helper_resolve_target_and_is_auth_file() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        fs::create_dir_all(&secret_dir).expect("secret dir");
        let auth_file = dir.path().join("auth.json");
        fs::write(&auth_file, "{}").expect("auth");

        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _auth = EnvGuard::set(&lock, "CODEX_AUTH_FILE", auth_file.to_str().expect("auth"));

        assert_eq!(
            resolve_target(Some("alpha.json")).expect("target"),
            secret_dir.join("alpha.json")
        );
        assert_eq!(resolve_target(Some("../bad")).expect_err("usage"), 64);
        assert_eq!(resolve_target(None).expect("auth default"), auth_file);
        assert!(is_auth_file(&auth_file));
        assert!(!is_auth_file(&secret_dir.join("alpha.json")));
    }

    #[test]
    fn rate_limits_helper_resolve_target_without_auth_returns_default_active_path() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let home = dir.path().join("home");
        let codex_home = dir.path().join("codex-home");
        let _auth = EnvGuard::remove(&lock, "CODEX_AUTH_FILE");
        let _codex_home = EnvGuard::set(&lock, "CODEX_HOME", codex_home.to_str().expect("home"));
        let _home = EnvGuard::set(&lock, "HOME", home.to_str().expect("home"));

        assert_eq!(
            resolve_target(None).expect("default active auth"),
            home.join(".agents").join("auth.json")
        );
    }

    #[test]
    fn rate_limits_helper_collect_json_from_cache_covers_hit_and_miss() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_root = dir.path().join("cache-root");
        fs::create_dir_all(&secret_dir).expect("secrets");
        fs::create_dir_all(&cache_root).expect("cache");

        let alpha = secret_dir.join("alpha.json");
        fs::write(&alpha, "{}").expect("alpha");

        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _cache = EnvGuard::set(&lock, "ZSH_CACHE_DIR", cache_root.to_str().expect("cache"));
        let values = complete_weekly_values("3h", 92, 88, 1_700_003_600, Some(1_700_001_200));
        cache::write_prompt_segment_cache(&alpha, fresh_fetched_at(), &values)
            .expect("write cache");

        let hit = collect_json_from_cache(&alpha, "cache", true);
        assert!(hit.ok);
        assert_eq!(hit.status, "ok");
        let summary = hit.summary.expect("summary");
        assert_eq!(summary.non_weekly_label.as_deref(), Some("3h"));
        assert_eq!(summary.non_weekly_remaining, Some(92));
        assert_eq!(summary.weekly_remaining, Some(88));

        let missing_target = secret_dir.join("missing.json");
        let miss = collect_json_from_cache(&missing_target, "cache", true);
        assert!(!miss.ok);
        let error = miss.error.expect("error");
        assert_eq!(error.code, "cache-read-failed");
        assert!(error.message.contains("cache not found"));
    }

    #[test]
    fn rate_limits_helper_fetch_one_line_cached_covers_success_and_error() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_root = dir.path().join("cache-root");
        fs::create_dir_all(&secret_dir).expect("secrets");
        fs::create_dir_all(&cache_root).expect("cache");

        let alpha = secret_dir.join("alpha.json");
        fs::write(&alpha, "{}").expect("alpha");

        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _cache = EnvGuard::set(&lock, "ZSH_CACHE_DIR", cache_root.to_str().expect("cache"));
        let values = complete_weekly_values("3h", 70, 55, 1_700_003_600, Some(1_700_001_200));
        cache::write_prompt_segment_cache(&alpha, fresh_fetched_at(), &values)
            .expect("write cache");

        let cached = fetch_one_line_cached(&alpha);
        assert_eq!(cached.rc, 0);
        assert!(cached.err.is_empty());
        assert!(cached.line.expect("line").contains("3h:70%"));

        let miss = fetch_one_line_cached(&secret_dir.join("beta.json"));
        assert_eq!(miss.rc, 1);
        assert!(miss.line.is_none());
        assert!(miss.err.contains("cache not found"));
    }

    #[test]
    fn rate_limits_helper_async_fetch_one_line_uses_cache_fallback() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_root = dir.path().join("cache-root");
        fs::create_dir_all(&secret_dir).expect("secrets");
        fs::create_dir_all(&cache_root).expect("cache");

        let missing = secret_dir.join("ghost.json");
        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _cache = EnvGuard::set(&lock, "ZSH_CACHE_DIR", cache_root.to_str().expect("cache"));
        let values = complete_weekly_values("3h", 68, 42, 1_700_003_600, Some(1_700_001_200));
        cache::write_prompt_segment_cache(&missing, fresh_fetched_at(), &values)
            .expect("write cache");

        let result = async_fetch_one_line(&missing, false, true, "ghost");
        assert_eq!(result.rc, 0);
        let line = result.line.expect("line");
        assert!(line.contains("3h:68%"));
        assert!(result.err.contains("falling back to cache"));
    }

    #[test]
    fn rate_limits_helper_single_one_line_cached_mode_handles_hit_and_miss() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_root = dir.path().join("cache-root");
        fs::create_dir_all(&secret_dir).expect("secrets");
        fs::create_dir_all(&cache_root).expect("cache");

        let alpha = secret_dir.join("alpha.json");
        let beta = secret_dir.join("beta.json");
        fs::write(&alpha, "{}").expect("alpha");
        fs::write(&beta, "{}").expect("beta");

        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _cache = EnvGuard::set(&lock, "ZSH_CACHE_DIR", cache_root.to_str().expect("cache"));
        let values = complete_weekly_values("3h", 61, 39, 1_700_003_600, Some(1_700_001_200));
        cache::write_prompt_segment_cache(&alpha, fresh_fetched_at(), &values)
            .expect("write cache");

        let hit = single_one_line(&alpha, true, true, false).expect("single");
        assert!(hit.line.expect("line").contains("3h:61%"));

        let miss = single_one_line(&beta, true, true, true).expect("single");
        assert!(miss.line.is_none());

        let missing =
            single_one_line(&secret_dir.join("missing.json"), true, true, true).expect("single");
        assert!(missing.line.is_none());
    }

    #[test]
    fn rate_limits_helper_sync_auth_silent_updates_matching_secret_and_timestamps() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        let cache_dir = dir.path().join("cache");
        fs::create_dir_all(&secret_dir).expect("secrets");
        fs::create_dir_all(&cache_dir).expect("cache");

        let auth_file = dir.path().join("auth.json");
        let alpha = secret_dir.join("alpha.json");
        let beta = secret_dir.join("beta.json");
        fs::write(
            &auth_file,
            auth_json(
                PAYLOAD_ALPHA,
                "acct_001",
                "refresh_new",
                "2025-01-20T12:34:56Z",
            ),
        )
        .expect("auth");
        fs::write(
            &alpha,
            auth_json(
                PAYLOAD_ALPHA,
                "acct_001",
                "refresh_old",
                "2025-01-19T12:34:56Z",
            ),
        )
        .expect("alpha");
        fs::write(
            &beta,
            auth_json(
                PAYLOAD_BETA,
                "acct_002",
                "refresh_beta",
                "2025-01-18T12:34:56Z",
            ),
        )
        .expect("beta");
        fs::write(secret_dir.join("invalid.json"), "{invalid").expect("invalid");
        fs::write(secret_dir.join("note.txt"), "ignore").expect("note");

        let _auth = EnvGuard::set(&lock, "CODEX_AUTH_FILE", auth_file.to_str().expect("auth"));
        let _secret = EnvGuard::set(
            &lock,
            "CODEX_SECRET_DIR",
            secret_dir.to_str().expect("secret"),
        );
        let _cache = EnvGuard::set(
            &lock,
            "CODEX_SECRET_CACHE_DIR",
            cache_dir.to_str().expect("cache"),
        );

        let (rc, err) = sync_auth_silent().expect("sync");
        assert_eq!(rc, 0);
        assert!(err.is_none());
        assert_eq!(
            fs::read(&alpha).expect("alpha"),
            fs::read(&auth_file).expect("auth")
        );
        assert_ne!(
            fs::read(&beta).expect("beta"),
            fs::read(&auth_file).expect("auth")
        );
        assert!(cache_dir.join("alpha.json.timestamp").is_file());
        assert!(cache_dir.join("auth.json.timestamp").is_file());
    }

    #[test]
    fn rate_limits_helper_parsers_and_name_helpers_cover_fallbacks() {
        let parsed =
            parse_one_line_output("alpha 3h:90% W:80% 2025-01-20 12:00:00+00:00").expect("parsed");
        assert_eq!(parsed.window_label.as_deref(), Some("3h"));
        assert_eq!(parsed.non_weekly_remaining, Some(90));
        assert_eq!(parsed.weekly_remaining, Some(80));
        assert_eq!(parsed.weekly_reset_iso, "2025-01-20 12:00:00+00:00");
        assert!(parse_one_line_output("bad").is_none());

        assert_eq!(normalize_one_line("a\tb\nc\r".to_string()), "a b c ");
        assert_eq!(target_file_name(Path::new("alpha.json")), "alpha.json");
        assert_eq!(target_file_name(Path::new("")), "");
        assert_eq!(secret_display_name(Path::new("alpha.json")), "alpha");
    }

    #[test]
    fn rate_limits_helper_current_secret_basename_tracks_auth_switch() {
        let lock = GlobalStateLock::new();
        let dir = tempfile::TempDir::new().expect("tempdir");
        let secret_dir = dir.path().join("secrets");
        fs::create_dir_all(&secret_dir).expect("secrets");

        let auth_file = dir.path().join("auth.json");
        let alpha = secret_dir.join("alpha.json");
        let beta = secret_dir.join("beta.json");

        let alpha_json = auth_json(
            PAYLOAD_ALPHA,
            "acct_001",
            "refresh_alpha",
            "2025-01-20T12:34:56Z",
        );
        let beta_json = auth_json(
            PAYLOAD_BETA,
            "acct_002",
            "refresh_beta",
            "2025-01-21T12:34:56Z",
        );
        fs::write(&alpha, &alpha_json).expect("alpha");
        fs::write(&beta, &beta_json).expect("beta");
        fs::write(&auth_file, &alpha_json).expect("auth alpha");

        let _auth = EnvGuard::set(&lock, "CODEX_AUTH_FILE", auth_file.to_str().expect("auth"));

        let secret_files = vec![alpha.clone(), beta.clone()];
        assert_eq!(
            current_secret_basename(&secret_files).as_deref(),
            Some("alpha")
        );

        fs::write(&auth_file, &beta_json).expect("auth beta");
        assert_eq!(
            current_secret_basename(&secret_files).as_deref(),
            Some("beta")
        );
    }
}

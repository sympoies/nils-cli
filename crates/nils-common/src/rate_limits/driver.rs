//! Multi-account orchestration of `diag rate-limits`.
//!
//! [`run`] owns flag validation, target selection, `--all`, `--async`,
//! `--watch`, `--jobs`, the JSON collection envelopes, and the accounts table.
//! A provider supplies its usage client, cache, and single-target mode through
//! [`RateLimitsProvider`].

use anyhow::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use super::schema::{self, RateLimitResult, TargetIdentity};
use super::table::{self, Row, RowState, TableContext};
use super::values::parse_one_line_output;
use crate::diag_output;
use crate::env as shared_env;

const WATCH_INTERVAL_SECONDS: u64 = 60;
const ANSI_CLEAR_SCREEN_AND_HOME: &str = "\x1b[2J\x1b[H";
const DEFAULT_ASYNC_JOBS: usize = 5;

/// Distinct one-line rc for "live fetch ok, but the provider reports no active
/// window". Kept apart from failure codes so it is neither retried nor counted
/// as an error.
pub const RC_NO_RATE_LIMIT_WINDOW: i32 = 6;

/// Static names and environment keys of one provider's command.
pub struct ProviderSpec {
    /// `provider` field of every result.
    pub provider: &'static str,
    pub schema_version: &'static str,
    pub command: &'static str,
    /// Prefix of every message, such as `codex-rate-limits`.
    pub tool: &'static str,
    /// Heading of the accounts table.
    pub table_title: &'static str,
    /// Environment variable that names the secret directory, used in messages.
    pub secret_dir_env: &'static str,
    /// Usage synopsis printed when `--all` gets a positional target.
    pub usage: &'static str,
    /// Environment flag that turns a bare text invocation into `--all`.
    pub default_all_env: Option<&'static str>,
    /// Test hook bounding `--watch` rounds.
    pub watch_max_rounds_env: &'static str,
    /// Override of the 60 second `--watch` interval.
    pub watch_interval_env: &'static str,
}

impl ProviderSpec {
    fn async_tool(&self) -> String {
        format!("{}-async", self.tool)
    }
}

/// Parsed command-line options, identical for every provider.
#[derive(Clone, Debug, Default)]
pub struct RunOptions {
    pub clear_cache: bool,
    pub debug: bool,
    pub cached: bool,
    pub no_refresh_auth: bool,
    pub json: bool,
    pub one_line: bool,
    pub all: bool,
    pub async_mode: bool,
    pub watch: bool,
    pub jobs: Option<String>,
    pub secret: Option<String>,
}

/// When a JSON collection falls back to cached values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheFallbackPolicy {
    /// Only when the provider reports no active window.
    NoWindow,
    /// On any live failure.
    AnyFailure,
}

/// One target's one-line observation for the accounts table.
#[derive(Debug, Default)]
pub struct OneLineFetch {
    pub line: Option<String>,
    pub rc: i32,
    pub err: String,
    pub reset_credits_available: Option<i64>,
    /// The line was served from cache past its freshness TTL.
    pub stale: bool,
    /// The live fetch succeeded but reported no active window, and no cache
    /// was available. Benign.
    pub no_window: bool,
}

/// Reset epochs shown in a filled table row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResetEpochs {
    pub non_weekly: Option<i64>,
    pub weekly: Option<i64>,
}

/// Failure to list targets: exit code, message, and JSON details.
pub type TargetDiscoveryError = (i32, String, Option<Value>);

/// Progress reporting for multi-target collection.
pub trait ProgressSink {
    fn set_message(&self, message: String);
    fn inc(&self, delta: u64);
    fn finish_and_clear(self: Box<Self>);
}

/// A provider's usage client, cache, and target model.
pub trait RateLimitsProvider: Sync {
    fn spec(&self) -> &ProviderSpec;
    /// Current wall-clock epoch seconds.
    fn now_epoch(&self) -> i64;
    /// Formats `epoch` in local time with a strftime `format`.
    fn format_local(&self, epoch: i64, format: &str) -> Option<String>;
    /// A progress bar for `total` targets, or `None` to show none.
    fn progress(&self, _total: usize, _prefix: &str) -> Option<Box<dyn ProgressSink>> {
        None
    }
    /// Directory holding one `<name>.json` per target.
    fn secret_dir(&self) -> PathBuf;
    /// Targets of the JSON collection modes.
    fn json_targets(&self) -> std::result::Result<Vec<PathBuf>, TargetDiscoveryError> {
        collect_json_targets_from_dir(self.spec(), &self.secret_dir(), true)
    }
    /// Targets used by the async JSON collection mode. Providers may include
    /// an active credential that is not represented in their profile store.
    fn async_json_targets(&self) -> std::result::Result<Vec<PathBuf>, TargetDiscoveryError> {
        self.json_targets()
    }
    fn identity(&self, target: &Path) -> TargetIdentity;
    /// Clears the provider cache for `-c`.
    fn clear_cache(&self) -> std::result::Result<(), String>;
    /// Runs before a live multi-target collection.
    fn prepare_collection(&self, _debug: bool) {}
    /// The single-target mode.
    fn run_single(&self, args: &RunOptions, cached: bool, one_line: bool) -> Result<i32>;
    /// One target's JSON result.
    fn json_result(
        &self,
        target: &Path,
        cached: bool,
        fallback: CacheFallbackPolicy,
    ) -> RateLimitResult;
    /// One target's table observation in `--async` mode.
    fn async_one_line(&self, target: &Path, cached: bool) -> OneLineFetch;
    /// One target's table observation in sequential `--all` mode.
    fn sequential_one_line(&self, target: &Path, cached: bool, debug: bool) -> OneLineFetch;
    /// Reset epochs of a filled row. `fill_from_stale_cache` also consults a
    /// stale cache for epochs the live source did not supply.
    fn row_reset_epochs(
        &self,
        target: &Path,
        cached: bool,
        fill_from_stale_cache: bool,
    ) -> ResetEpochs;
    /// Row name of the active login among `targets`.
    fn current_name(&self, targets: &[PathBuf]) -> Option<String>;
}

pub fn run<P: RateLimitsProvider>(provider: &P, args: &RunOptions) -> Result<i32> {
    let spec = provider.spec();
    let tool = spec.tool;
    let cached_mode = args.cached;
    let mut one_line = args.one_line;
    let mut all_mode = args.all;
    let output_json = args.json;

    let mut debug_mode = args.debug;
    if !debug_mode
        && let Ok(raw) = std::env::var("ZSH_DEBUG")
        && raw.parse::<i64>().unwrap_or(0) >= 2
    {
        debug_mode = true;
    }

    if args.async_mode {
        if !args.cached {
            provider.prepare_collection(debug_mode);
        }
        if args.json {
            return run_async_json_mode(provider, args);
        }
        return run_async_table_mode(provider, args, debug_mode, args.watch);
    }

    if cached_mode {
        one_line = true;
        if output_json {
            diag_output::emit_error(
                spec.schema_version,
                spec.command,
                "invalid-flag-combination",
                format!("{tool}: --json is not supported with --cached"),
                Some(serde_json::json!({
                    "flags": ["--json", "--cached"],
                })),
            )?;
            return Ok(64);
        }
        if args.clear_cache {
            eprintln!("{tool}: -c is not compatible with --cached");
            return Ok(64);
        }
    }

    if output_json && one_line {
        diag_output::emit_error(
            spec.schema_version,
            spec.command,
            "invalid-flag-combination",
            format!("{tool}: --one-line is not compatible with --json"),
            Some(serde_json::json!({
                "flags": ["--one-line", "--json"],
            })),
        )?;
        return Ok(64);
    }

    if args.clear_cache
        && let Err(err) = provider.clear_cache()
    {
        if output_json {
            diag_output::emit_error(
                spec.schema_version,
                spec.command,
                "cache-clear-failed",
                err,
                None,
            )?;
        } else {
            eprintln!("{err}");
        }
        return Ok(1);
    }

    if !all_mode
        && !output_json
        && !cached_mode
        && args.secret.is_none()
        && spec.default_all_env.is_some_and(shared_env::env_truthy)
    {
        all_mode = true;
    }

    if all_mode {
        if !cached_mode {
            provider.prepare_collection(debug_mode);
        }
        if args.secret.is_some() {
            eprintln!("{tool}: usage: {}", spec.usage);
            return Ok(64);
        }
        if output_json {
            return run_all_json_mode(provider, cached_mode);
        }
        return run_all_table_mode(provider, cached_mode, debug_mode);
    }

    provider.run_single(args, cached_mode, one_line)
}

fn run_async_json_mode<P: RateLimitsProvider>(provider: &P, args: &RunOptions) -> Result<i32> {
    let spec = provider.spec();
    let tool = spec.tool;
    if args.one_line {
        diag_output::emit_error(
            spec.schema_version,
            spec.command,
            "invalid-flag-combination",
            format!("{tool}: --async does not support --one-line"),
            Some(serde_json::json!({
                "flag": "--one-line",
                "mode": "async",
            })),
        )?;
        return Ok(64);
    }
    if let Some(secret) = args.secret.as_deref() {
        diag_output::emit_error(
            spec.schema_version,
            spec.command,
            "invalid-positional-arg",
            format!("{tool}: --async does not accept positional args: {secret}"),
            Some(serde_json::json!({
                "secret": secret,
                "mode": "async",
            })),
        )?;
        return Ok(64);
    }
    if args.clear_cache && args.cached {
        diag_output::emit_error(
            spec.schema_version,
            spec.command,
            "invalid-flag-combination",
            format!("{tool}: --async: -c is not compatible with --cached"),
            Some(serde_json::json!({
                "flags": ["--async", "--cached", "-c"],
            })),
        )?;
        return Ok(64);
    }
    if args.clear_cache
        && let Err(err) = provider.clear_cache()
    {
        diag_output::emit_error(
            spec.schema_version,
            spec.command,
            "cache-clear-failed",
            err,
            None,
        )?;
        return Ok(1);
    }

    let targets = match provider.async_json_targets() {
        Ok(value) => value,
        Err((code, message, details)) => {
            diag_output::emit_error(
                spec.schema_version,
                spec.command,
                "secret-discovery-failed",
                message,
                details,
            )?;
            return Ok(code);
        }
    };

    let jobs = resolve_async_jobs(args.jobs.as_deref());
    let cached_mode = args.cached;
    let mut results_by_target = collect_async_items(&targets, jobs, None, |path, _| {
        provider.json_result(&path, cached_mode, CacheFallbackPolicy::AnyFailure)
    });
    let mut results = Vec::new();
    let mut rc = 0;
    for target in &targets {
        let result = results_by_target
            .remove(&target_file_name(target))
            .unwrap_or_else(|| {
                RateLimitResult::error(
                    provider.identity(target),
                    "network",
                    "request-failed",
                    format!(
                        "{tool}: async worker did not return a result for {}",
                        target.display()
                    ),
                    None,
                    None,
                )
            });
        if !args.cached && !result.ok {
            rc = 1;
        }
        results.push(result);
    }
    results.sort_by(|a, b| a.name.cmp(&b.name));
    schema::emit_collection_envelope(spec, "async", rc == 0, results)?;
    Ok(rc)
}

fn run_all_json_mode<P: RateLimitsProvider>(provider: &P, cached_mode: bool) -> Result<i32> {
    let spec = provider.spec();
    let targets = match provider.json_targets() {
        Ok(value) => value,
        Err((code, message, details)) => {
            diag_output::emit_error(
                spec.schema_version,
                spec.command,
                "secret-discovery-failed",
                message,
                details,
            )?;
            return Ok(code);
        }
    };

    let mut results = Vec::new();
    let mut rc = 0;
    for target in &targets {
        let result = provider.json_result(target, cached_mode, CacheFallbackPolicy::NoWindow);
        if !cached_mode && !result.ok {
            rc = 1;
        }
        results.push(result);
    }
    results.sort_by(|a, b| a.name.cmp(&b.name));
    schema::emit_collection_envelope(spec, "all", rc == 0, results)?;
    Ok(rc)
}

/// Lists `<dir>/*.json` for the JSON modes. `strict` also rejects an empty
/// directory.
pub fn collect_json_targets_from_dir(
    spec: &ProviderSpec,
    secret_dir: &Path,
    strict: bool,
) -> std::result::Result<Vec<PathBuf>, TargetDiscoveryError> {
    let tool = spec.tool;
    let env = spec.secret_dir_env;
    let details = || {
        Some(serde_json::json!({
            "secret_dir": secret_dir.display().to_string(),
        }))
    };
    if !secret_dir.is_dir() {
        return Err((
            1,
            format!("{tool}: {env} not found: {}", secret_dir.display()),
            details(),
        ));
    }

    let mut targets: Vec<PathBuf> = std::fs::read_dir(secret_dir)
        .map_err(|err| (1, format!("{tool}: failed to read {env}: {err}"), details()))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_json_file_name(path))
        .collect();

    if strict && targets.is_empty() {
        return Err(no_targets_error(spec, secret_dir));
    }

    targets.sort();
    Ok(targets)
}

pub fn no_targets_error(spec: &ProviderSpec, secret_dir: &Path) -> TargetDiscoveryError {
    (
        1,
        format!(
            "{}: no secrets found in {}",
            spec.tool,
            secret_dir.display()
        ),
        Some(serde_json::json!({
            "secret_dir": secret_dir.display().to_string(),
        })),
    )
}

/// Lists `<dir>/*.json` for `--async` text output; an empty directory is fine.
pub fn collect_async_text_targets(
    spec: &ProviderSpec,
    secret_dir: &Path,
) -> std::result::Result<Vec<PathBuf>, String> {
    let async_tool = spec.async_tool();
    let env = spec.secret_dir_env;
    if !secret_dir.is_dir() {
        return Err(format!(
            "{async_tool}: {env} not found: {}",
            secret_dir.display()
        ));
    }

    let mut targets: Vec<PathBuf> = std::fs::read_dir(secret_dir)
        .map_err(|err| format!("{async_tool}: failed to read {env}: {err}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_json_file_name(path))
        .collect();

    targets.sort();
    Ok(targets)
}

fn is_json_file_name(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()) == Some("json")
}

fn target_file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

fn run_async_table_mode<P: RateLimitsProvider>(
    provider: &P,
    args: &RunOptions,
    debug_mode: bool,
    watch_mode: bool,
) -> Result<i32> {
    let spec = provider.spec();
    let tool = spec.tool;
    let async_tool = spec.async_tool();
    if args.json {
        eprintln!("{tool}: --async does not support --json");
        return Ok(64);
    }
    if args.one_line {
        eprintln!("{tool}: --async does not support --one-line");
        return Ok(64);
    }
    if args.secret.is_some() {
        eprintln!("{tool}: --async does not accept positional args");
        eprintln!(
            "{tool}: hint: async always queries all secrets under {}",
            spec.secret_dir_env
        );
        return Ok(64);
    }
    if args.clear_cache && args.cached {
        eprintln!("{tool}: --async: -c is not compatible with --cached");
        return Ok(64);
    }

    let jobs = resolve_async_jobs(args.jobs.as_deref());

    if args.clear_cache
        && let Err(err) = provider.clear_cache()
    {
        eprintln!("{err}");
        return Ok(1);
    }

    let targets = match collect_async_text_targets(spec, &provider.secret_dir()) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            return Ok(1);
        }
    };

    if !watch_mode {
        if targets.is_empty() {
            eprintln!(
                "{async_tool}: no secrets found in {}",
                provider.secret_dir().display()
            );
            return Ok(1);
        }

        let current_name = provider.current_name(&targets);
        let round = collect_async_round(provider, &targets, args.cached, jobs);
        print_table(
            provider,
            round.rows,
            &round.window_labels,
            current_name,
            None,
        );
        emit_async_debug(spec, debug_mode, &targets, &round.stderr_map);
        return Ok(round.rc);
    }

    let mut overall_rc = 0;
    let mut rendered_rounds = 0u64;
    let max_rounds = env_positive_u64(spec.watch_max_rounds_env);
    let watch_interval_seconds =
        env_positive_u64(spec.watch_interval_env).unwrap_or(WATCH_INTERVAL_SECONDS);
    let is_terminal_stdout = std::io::stdout().is_terminal();

    loop {
        let targets = match collect_async_text_targets(spec, &provider.secret_dir()) {
            Ok(value) => value,
            Err(err) => {
                overall_rc = 1;
                if is_terminal_stdout {
                    print!("{ANSI_CLEAR_SCREEN_AND_HOME}");
                }
                eprintln!("{err}");
                let _ = std::io::stdout().flush();

                rendered_rounds += 1;
                if let Some(limit) = max_rounds
                    && rendered_rounds >= limit
                {
                    break;
                }

                thread::sleep(Duration::from_secs(watch_interval_seconds));
                continue;
            }
        };
        let current_name = provider.current_name(&targets);
        let round = collect_async_round(provider, &targets, args.cached, jobs);
        if round.rc != 0 {
            overall_rc = 1;
        }

        if is_terminal_stdout {
            print!("{ANSI_CLEAR_SCREEN_AND_HOME}");
        }

        let now_epoch = provider.now_epoch();
        let update_time = provider
            .format_local(now_epoch, "%Y-%m-%d %H:%M:%S %:z")
            .unwrap_or_else(|| now_epoch.to_string());
        print_table(
            provider,
            round.rows,
            &round.window_labels,
            current_name,
            Some(update_time.as_str()),
        );
        emit_async_debug(spec, debug_mode, &targets, &round.stderr_map);
        let _ = std::io::stdout().flush();

        rendered_rounds += 1;
        if let Some(limit) = max_rounds
            && rendered_rounds >= limit
        {
            break;
        }

        thread::sleep(Duration::from_secs(watch_interval_seconds));
    }

    Ok(overall_rc)
}

fn print_table<P: RateLimitsProvider>(
    provider: &P,
    rows: Vec<Row>,
    window_labels: &BTreeSet<String>,
    current_name: Option<String>,
    update_time: Option<&str>,
) {
    let format_local = |epoch: i64, format: &str| provider.format_local(epoch, format);
    let rendered = table::render_all_accounts_table(
        rows,
        &TableContext {
            title: provider.spec().table_title,
            window_labels,
            current_name: current_name.as_deref(),
            update_time,
            now_epoch: provider.now_epoch(),
            format_local: &format_local,
        },
    );
    print!("{rendered}");
}

struct AsyncRound {
    rc: i32,
    rows: Vec<Row>,
    window_labels: BTreeSet<String>,
    stderr_map: BTreeMap<String, String>,
}

pub fn resolve_async_jobs(jobs: Option<&str>) -> usize {
    jobs.and_then(|raw| raw.parse::<i64>().ok())
        .filter(|value| *value > 0)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_ASYNC_JOBS)
}

fn env_positive_u64(key: &str) -> Option<u64> {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|value| *value > 0)
}

/// Runs `worker` over `targets` on up to `jobs` threads, keyed by file name.
pub fn collect_async_items<T, F>(
    targets: &[PathBuf],
    jobs: usize,
    progress: Option<Box<dyn ProgressSink>>,
    worker: F,
) -> BTreeMap<String, T>
where
    T: Send,
    F: Fn(PathBuf, String) -> T + Sync,
{
    let total = targets.len();
    let mut items = BTreeMap::new();
    if total == 0 {
        return items;
    }

    let worker_count = jobs.clamp(1, total);
    let worker = &worker;
    thread::scope(|scope| {
        let (tx, rx) = mpsc::channel::<(String, T)>();
        let mut handles = Vec::new();
        let mut index = 0usize;
        let spawn = |path: PathBuf, tx: mpsc::Sender<(String, T)>| {
            scope.spawn(move || {
                let name = target_file_name(&path);
                let value = worker(path, name.clone());
                let _ = tx.send((name, value));
            })
        };

        while index < total && handles.len() < worker_count {
            handles.push(spawn(targets[index].clone(), tx.clone()));
            index += 1;
        }

        while items.len() < total {
            let Ok((name, value)) = rx.recv() else {
                break;
            };
            if let Some(progress) = &progress {
                progress.set_message(name.clone());
                progress.inc(1);
            }
            items.insert(name, value);

            if index < total {
                handles.push(spawn(targets[index].clone(), tx.clone()));
                index += 1;
            }
        }

        drop(tx);
        for handle in handles {
            let _ = handle.join();
        }
    });

    if let Some(progress) = progress {
        progress.finish_and_clear();
    }

    items
}

fn collect_async_round<P: RateLimitsProvider>(
    provider: &P,
    targets: &[PathBuf],
    cached_mode: bool,
    jobs: usize,
) -> AsyncRound {
    let prefix = format!("{} ", provider.spec().tool);
    let progress = if targets.len() > 1 {
        provider.progress(targets.len(), &prefix)
    } else {
        None
    };
    let mut events = collect_async_items(targets, jobs, progress, |path, _| {
        provider.async_one_line(&path, cached_mode)
    });

    let mut rc = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut window_labels = BTreeSet::new();
    let mut stderr_map = BTreeMap::new();

    for target in targets {
        let target_name = target_file_name(target);

        let mut row = Row::empty(target_name.trim_end_matches(".json").to_string());
        let mut benign_no_window = false;
        if let Some(event) = events.remove(&target_name) {
            row.reset_credits_available = event.reset_credits_available;
            if !event.err.is_empty() {
                stderr_map.insert(target_name.clone(), event.err.clone());
            }
            // A null window (no_window) is benign and must not fail the round;
            // a stale cache fallback (rc == 0) likewise succeeded.
            if !cached_mode && event.rc != 0 && !event.no_window {
                rc = 1;
            }
            benign_no_window = event.no_window;

            if let Some(line) = &event.line
                && let Some(parsed) = parse_one_line_output(line)
            {
                fill_row(&mut row, parsed);
                let epochs = provider.row_reset_epochs(target, cached_mode, true);
                row.non_weekly_reset_epoch = epochs.non_weekly;
                row.weekly_reset_epoch = epochs.weekly;
                row.state = if event.stale {
                    RowState::Stale
                } else {
                    RowState::Filled
                };
                if !row.window_label.is_empty() {
                    window_labels.insert(row.window_label.clone());
                }
                rows.push(row);
                continue;
            }
        }

        if benign_no_window {
            row.state = RowState::NoWindow;
        } else if !cached_mode {
            rc = 1;
        }
        rows.push(row);
    }

    AsyncRound {
        rc,
        rows,
        window_labels,
        stderr_map,
    }
}

fn fill_row(row: &mut Row, parsed: super::values::ParsedOneLine) {
    row.window_label = parsed.window_label.unwrap_or_default();
    row.non_weekly_remaining = parsed.non_weekly_remaining.unwrap_or(-1);
    row.weekly_remaining = parsed.weekly_remaining.unwrap_or(-1);
    row.weekly_reset_iso = parsed.weekly_reset_iso;
}

fn emit_async_debug(
    spec: &ProviderSpec,
    debug_mode: bool,
    targets: &[PathBuf],
    stderr_map: &BTreeMap<String, String>,
) {
    if !debug_mode {
        return;
    }

    let mut printed = false;
    for target in targets {
        if let Some(err) = stderr_map.get(&target_file_name(target)) {
            if err.is_empty() {
                continue;
            }
            if !printed {
                printed = true;
                eprintln!();
                eprintln!("{}: per-account stderr (captured):", spec.async_tool());
            }
            eprintln!("---- account stderr ----");
            eprintln!("{err}");
        }
    }
}

fn run_all_table_mode<P: RateLimitsProvider>(
    provider: &P,
    cached_mode: bool,
    debug_mode: bool,
) -> Result<i32> {
    let spec = provider.spec();
    let tool = spec.tool;
    let secret_dir = provider.secret_dir();
    if !secret_dir.is_dir() {
        eprintln!(
            "{tool}: {} not found: {}",
            spec.secret_dir_env,
            secret_dir.display()
        );
        return Ok(1);
    }

    let mut targets: Vec<PathBuf> = std::fs::read_dir(&secret_dir)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_json_file_name(path))
        .collect();

    if targets.is_empty() {
        eprintln!("{tool}: no secrets found in {}", secret_dir.display());
        return Ok(1);
    }

    targets.sort();

    let current_name = provider.current_name(&targets);

    let total = targets.len();
    let progress = if total > 1 {
        provider.progress(total, &format!("{tool} "))
    } else {
        None
    };

    let mut rc = 0;
    let mut rows: Vec<Row> = Vec::new();
    let mut window_labels = BTreeSet::new();

    for target in &targets {
        let target_name = target_file_name(target);
        if let Some(progress) = &progress {
            progress.set_message(target_name.clone());
        }

        let mut row = Row::empty(target_name.trim_end_matches(".json").to_string());
        let one_line = provider.sequential_one_line(target, cached_mode, debug_mode);
        row.reset_credits_available = one_line.reset_credits_available;
        let output = one_line.line.unwrap_or_default();

        if output.is_empty() {
            if !cached_mode && !one_line.no_window {
                rc = 1;
            }
            if one_line.no_window {
                row.state = RowState::NoWindow;
            }
            rows.push(row);
            continue;
        }

        if let Some(parsed) = parse_one_line_output(&output) {
            fill_row(&mut row, parsed);
            let epochs = provider.row_reset_epochs(target, cached_mode, false);
            row.non_weekly_reset_epoch = epochs.non_weekly;
            row.weekly_reset_epoch = epochs.weekly;

            if !row.window_label.is_empty() {
                window_labels.insert(row.window_label.clone());
            }
            rows.push(row);
        } else {
            if !cached_mode {
                rc = 1;
            }
            rows.push(row);
        }

        if let Some(progress) = &progress {
            progress.inc(1);
        }
    }

    if let Some(progress) = progress {
        progress.finish_and_clear();
    }

    print_table(provider, rows, &window_labels, current_name, None);

    Ok(rc)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn async_jobs_default_to_five_and_reject_non_positive_values() {
        assert_eq!(resolve_async_jobs(None), 5);
        assert_eq!(resolve_async_jobs(Some("0")), 5);
        assert_eq!(resolve_async_jobs(Some("x")), 5);
        assert_eq!(resolve_async_jobs(Some("3")), 3);
    }

    #[test]
    fn async_pool_collects_every_target_by_file_name() {
        let targets: Vec<PathBuf> = (0..7)
            .map(|index| PathBuf::from(format!("/secrets/t{index}.json")))
            .collect();
        let items = collect_async_items(&targets, 3, None, |path, name| {
            format!("{}:{name}", path.display())
        });
        assert_eq!(items.len(), 7);
        assert_eq!(items["t4.json"], "/secrets/t4.json:t4.json");
    }
}

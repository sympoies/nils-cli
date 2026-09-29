//! Capacity-aware Codex account selection.
//!
//! The module is split into a pure core and a thin I/O shell so other
//! surfaces (for example a usage endpoint) can reuse the same capacity
//! assessment without re-implementing it:
//!
//! - [`CandidateCapacity::from_snapshot`] classifies one rate-limit snapshot
//!   against the documented [`MIN_REMAINING_PERCENT`] threshold.
//! - [`select`] applies a [`Strategy`] to a set of assessed candidates. It is
//!   pure and deterministic: candidates are ordered by nickname (byte order)
//!   regardless of input order.
//! - [`discover_candidates`], [`assess_candidates`], and
//!   [`assess_for_strategy`] read configured profiles and their capacity,
//!   preferring the shared rate-limit cache (`CODEX_RATE_LIMITS_CACHE_TTL`)
//!   and fetching only cache misses.
//! - [`run`] is the `codex-cli account select` command.
//!
//! Output carries nicknames, percentages, and epochs only. Tokens, account
//! IDs, emails, and filesystem paths never reach a serialized type here.

use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::thread;

use crate::diag_output;
use crate::rate_limits::cache;
use crate::rate_limits::client::{UsageRequest, fetch_usage};
use crate::rate_limits::render;

pub const SCHEMA_VERSION: &str = "codex-cli.account.select.v1";
pub const COMMAND: &str = "account select";

/// A window has capacity when its remaining percentage is at least this value.
/// A candidate is `available` only when every reported window meets it.
pub const MIN_REMAINING_PERCENT: i64 = 1;
/// Upper bound on assessed candidates; extra profiles (in nickname order) are
/// ignored so output and fetch fan-out stay bounded.
pub const MAX_CANDIDATES: usize = 64;
/// Nicknames follow the account-broker grammar: `[A-Za-z0-9._-]{1,64}`.
pub const MAX_NICKNAME_BYTES: usize = 64;

const WEEKLY_LABEL: &str = "weekly";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Strategy {
    /// The profile matching the active auth file, regardless of capacity.
    CurrentDefault,
    /// The first profile with capacity after the origin in nickname order.
    NextWithCapacity,
    /// The default profile when it has capacity, else the next one that does.
    DefaultWithCapacity,
}

impl Strategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CurrentDefault => "current-default",
            Self::NextWithCapacity => "next-with-capacity",
            Self::DefaultWithCapacity => "default-with-capacity",
        }
    }

    /// Whether the strategy needs live capacity (and may fetch cache misses).
    pub const fn needs_capacity(self) -> bool {
        !matches!(self, Self::CurrentDefault)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capacity {
    Available,
    Exhausted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapacitySource {
    /// A fresh entry from the shared rate-limit cache.
    Cache,
    /// A usage fetch made by this assessment (and written back to the cache).
    Network,
    /// No fresh snapshot was available.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapacityWindow {
    pub label: String,
    pub remaining_percent: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at_epoch: Option<i64>,
}

/// One rate-limit snapshot, in the shape shared by the cache and the fetch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RateLimitSnapshot {
    pub fetched_at_epoch: Option<i64>,
    pub non_weekly_label: Option<String>,
    pub non_weekly_remaining: Option<i64>,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: Option<i64>,
    pub weekly_reset_epoch: Option<i64>,
}

impl RateLimitSnapshot {
    fn from_cache_entry(entry: &cache::CacheEntry) -> Self {
        Self {
            fetched_at_epoch: entry.fetched_at_epoch,
            non_weekly_label: entry.non_weekly_label.clone(),
            non_weekly_remaining: entry.non_weekly_remaining,
            non_weekly_reset_epoch: entry.non_weekly_reset_epoch,
            weekly_remaining: entry.weekly_remaining,
            weekly_reset_epoch: entry.weekly_reset_epoch,
        }
    }

    fn from_weekly_values(fetched_at_epoch: i64, values: &render::WeeklyValues) -> Self {
        let positive = |epoch: i64| Some(epoch).filter(|value| *value > 0);
        Self {
            fetched_at_epoch: Some(fetched_at_epoch),
            non_weekly_label: values.non_weekly.as_ref().map(|w| w.label.clone()),
            non_weekly_remaining: values.non_weekly.as_ref().map(|w| w.remaining),
            non_weekly_reset_epoch: values
                .non_weekly
                .as_ref()
                .and_then(|w| positive(w.reset_epoch)),
            weekly_remaining: values.weekly.as_ref().map(|w| w.remaining),
            weekly_reset_epoch: values.weekly.as_ref().and_then(|w| positive(w.reset_epoch)),
        }
    }

    fn windows(&self) -> Vec<CapacityWindow> {
        let mut windows = Vec::with_capacity(2);
        if let Some(remaining) = self.non_weekly_remaining {
            windows.push(CapacityWindow {
                label: bounded_label(self.non_weekly_label.as_deref()),
                remaining_percent: remaining.clamp(0, 100),
                reset_at_epoch: self.non_weekly_reset_epoch,
            });
        }
        if let Some(remaining) = self.weekly_remaining {
            windows.push(CapacityWindow {
                label: WEEKLY_LABEL.to_string(),
                remaining_percent: remaining.clamp(0, 100),
                reset_at_epoch: self.weekly_reset_epoch,
            });
        }
        windows
    }
}

fn bounded_label(label: Option<&str>) -> String {
    match label {
        Some(value)
            if !value.is_empty()
                && value.len() <= 16
                && value.chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            value.to_string()
        }
        _ => "primary".to_string(),
    }
}

/// Bounded, secret-free capacity summary for one candidate profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CandidateCapacity {
    pub name: String,
    pub default: bool,
    pub excluded: bool,
    pub capacity: Capacity,
    pub source: CapacitySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_remaining_percent: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fetched_at_epoch: Option<i64>,
    pub windows: Vec<CapacityWindow>,
}

impl CandidateCapacity {
    /// Classifies a snapshot. `None`, or a snapshot without any window, is
    /// `unknown` with source `none`.
    pub fn from_snapshot(
        name: &str,
        snapshot: Option<&RateLimitSnapshot>,
        source: CapacitySource,
    ) -> Self {
        let windows = snapshot.map(RateLimitSnapshot::windows).unwrap_or_default();
        let min_remaining_percent = windows.iter().map(|w| w.remaining_percent).min();
        let (capacity, source, fetched_at_epoch) = match min_remaining_percent {
            None => (Capacity::Unknown, CapacitySource::None, None),
            Some(min) if min >= MIN_REMAINING_PERCENT => (
                Capacity::Available,
                source,
                snapshot.and_then(|s| s.fetched_at_epoch),
            ),
            Some(_) => (
                Capacity::Exhausted,
                source,
                snapshot.and_then(|s| s.fetched_at_epoch),
            ),
        };
        Self {
            name: name.to_string(),
            default: false,
            excluded: false,
            capacity,
            source,
            min_remaining_percent,
            fetched_at_epoch,
            windows,
        }
    }

    fn eligible(&self) -> bool {
        !self.excluded && self.capacity == Capacity::Available
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectError {
    /// No profile matches the active auth file, or it is excluded.
    DefaultUnavailable,
    /// `default-with-capacity` could not confirm the default's capacity.
    DefaultCapacityUnknown,
    /// No eligible candidate has confirmed capacity.
    NoCapacity,
    /// `--after` names no configured candidate.
    UnknownOrigin,
}

impl SelectError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::DefaultUnavailable => "default-account-unavailable",
            Self::DefaultCapacityUnknown => "default-capacity-unknown",
            Self::NoCapacity => "no-account-with-capacity",
            Self::UnknownOrigin => "unknown-origin",
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::DefaultUnavailable => "No selectable Codex profile matches the active auth file.",
            Self::DefaultCapacityUnknown => {
                "The default Codex profile's capacity could not be confirmed."
            }
            Self::NoCapacity => "No selectable Codex profile has confirmed capacity.",
            Self::UnknownOrigin => "--after does not name a configured Codex profile.",
        }
    }

    pub const fn exit_code(self) -> i32 {
        match self {
            Self::UnknownOrigin => 64,
            _ => 1,
        }
    }
}

/// The rotation origin [`select`] uses: for `next-with-capacity`, `after`
/// when given, else the default profile; `None` for every other strategy.
/// Callers that report the origin use this so it cannot drift from selection.
pub fn effective_origin<'a>(
    strategy: Strategy,
    candidates: &'a [CandidateCapacity],
    after: Option<&'a str>,
) -> Option<&'a str> {
    match strategy {
        Strategy::NextWithCapacity => after.or_else(|| {
            candidates
                .iter()
                .find(|candidate| candidate.default)
                .map(|candidate| candidate.name.as_str())
        }),
        _ => None,
    }
}

/// Applies `strategy` to assessed candidates and returns the selected nickname.
///
/// Candidates form a ring sorted by nickname (byte order), so the result never
/// depends on input order. `next-with-capacity` walks the ring starting after
/// the [`effective_origin`] and wraps, never returning the origin itself; with
/// no origin it starts at the first nickname.
/// `default-with-capacity` keeps an available default, refuses an unknown one,
/// and otherwise walks the ring after the default. Excluded and non-`available`
/// candidates are never returned by a capacity strategy.
pub fn select(
    strategy: Strategy,
    candidates: &[CandidateCapacity],
    origin: Option<&str>,
) -> std::result::Result<String, SelectError> {
    let mut ring: Vec<&CandidateCapacity> = candidates.iter().collect();
    ring.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    let default = ring.iter().copied().find(|candidate| candidate.default);

    match strategy {
        Strategy::CurrentDefault => default
            .filter(|candidate| !candidate.excluded)
            .map(|candidate| candidate.name.clone())
            .ok_or(SelectError::DefaultUnavailable),
        Strategy::NextWithCapacity => {
            let start = match effective_origin(strategy, candidates, origin) {
                Some(name) => Some(
                    ring.iter()
                        .position(|candidate| candidate.name == name)
                        .ok_or(SelectError::UnknownOrigin)?,
                ),
                None => None,
            };
            first_eligible_after(&ring, start)
        }
        Strategy::DefaultWithCapacity => {
            let default = default.ok_or(SelectError::DefaultUnavailable)?;
            if !default.excluded {
                match default.capacity {
                    Capacity::Available => return Ok(default.name.clone()),
                    Capacity::Unknown => return Err(SelectError::DefaultCapacityUnknown),
                    Capacity::Exhausted => {}
                }
            }
            let start = ring.iter().position(|c| c.name == default.name);
            first_eligible_after(&ring, start)
        }
    }
}

fn first_eligible_after(
    ring: &[&CandidateCapacity],
    start: Option<usize>,
) -> std::result::Result<String, SelectError> {
    let len = ring.len();
    let order: Vec<usize> = match start {
        Some(index) => (1..len).map(|offset| (index + offset) % len).collect(),
        None => (0..len).collect(),
    };
    order
        .into_iter()
        .map(|index| ring[index])
        .find(|candidate| candidate.eligible())
        .map(|candidate| candidate.name.clone())
        .ok_or(SelectError::NoCapacity)
}

/// Whether `value` is a valid profile nickname.
pub fn valid_nickname(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NICKNAME_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && value != "."
        && value != ".."
}

/// One configured profile. The path stays in-process and is never serialized.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub name: String,
    pub path: PathBuf,
    /// Whether this profile owns its shared rate-limit cache entry. False when
    /// its cache key collides with another candidate's (for example `Alpha`
    /// and `alpha`); such a profile never reads or writes the shared cache.
    pub shared_cache: bool,
}

#[derive(Clone, Debug)]
pub struct CandidateSet {
    /// Up to [`MAX_CANDIDATES`] profiles in nickname order.
    pub candidates: Vec<Candidate>,
    /// Nickname of the profile matching the active auth file.
    pub default: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryError {
    NoProfiles,
}

/// Lists `CODEX_SECRET_DIR/*.json` profiles with valid nicknames.
pub fn discover_candidates() -> std::result::Result<CandidateSet, DiscoveryError> {
    let secret_dir = crate::paths::resolve_secret_dir().ok_or(DiscoveryError::NoProfiles)?;
    let entries = std::fs::read_dir(&secret_dir).map_err(|_| DiscoveryError::NoProfiles)?;
    let mut candidates: Vec<Candidate> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter_map(|path| {
            let name = path
                .file_name()?
                .to_str()?
                .strip_suffix(".json")?
                .to_string();
            valid_nickname(&name).then_some(Candidate {
                name,
                path,
                shared_cache: true,
            })
        })
        .collect();
    if candidates.is_empty() {
        return Err(DiscoveryError::NoProfiles);
    }
    candidates.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
    candidates.truncate(MAX_CANDIDATES);
    mark_cache_collisions(&mut candidates);
    let paths: Vec<PathBuf> = candidates.iter().map(|c| c.path.clone()).collect();
    let default = crate::rate_limits::current_secret_basename(&paths)
        .filter(|name| candidates.iter().any(|c| &c.name == name));
    Ok(CandidateSet {
        candidates,
        default,
    })
}

fn mark_cache_collisions(candidates: &mut [Candidate]) {
    let keys: Vec<Option<PathBuf>> = candidates
        .iter()
        .map(|candidate| cache::cache_file_for_target(&candidate.path).ok())
        .collect();
    for (index, candidate) in candidates.iter_mut().enumerate() {
        candidate.shared_cache = match &keys[index] {
            Some(key) => {
                keys.iter()
                    .filter(|other| other.as_ref() == Some(key))
                    .count()
                    == 1
            }
            None => false,
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssessMode {
    /// Read fresh cache entries only; never touch the network.
    CacheOnly,
    /// Read fresh cache entries, then fetch misses and write them back.
    CacheThenNetwork,
}

/// Assesses every candidate. Excluded candidates are always cache-only.
pub fn assess_candidates(
    set: &CandidateSet,
    excluded: &BTreeSet<String>,
    mode: AssessMode,
) -> Vec<CandidateCapacity> {
    let mut snapshots: Vec<(Option<RateLimitSnapshot>, CapacitySource)> = set
        .candidates
        .iter()
        .map(|candidate| {
            match candidate
                .shared_cache
                .then(|| fresh_cache_snapshot(&candidate.path))
                .flatten()
            {
                Some(snapshot) => (Some(snapshot), CapacitySource::Cache),
                None => (None, CapacitySource::None),
            }
        })
        .collect();

    if mode == AssessMode::CacheThenNetwork {
        let misses: Vec<usize> = snapshots
            .iter()
            .enumerate()
            .filter(|(index, (snapshot, _))| {
                snapshot.is_none() && !excluded.contains(&set.candidates[*index].name)
            })
            .map(|(index, _)| index)
            .collect();
        for (index, snapshot) in fetch_snapshots(set, &misses) {
            if let Some(snapshot) = snapshot {
                snapshots[index] = (Some(snapshot), CapacitySource::Network);
            }
        }
    }

    set.candidates
        .iter()
        .zip(snapshots)
        .map(|(candidate, (snapshot, source))| {
            let mut assessed =
                CandidateCapacity::from_snapshot(&candidate.name, snapshot.as_ref(), source);
            assessed.default = set.default.as_deref() == Some(candidate.name.as_str());
            assessed.excluded = excluded.contains(&candidate.name);
            assessed
        })
        .collect()
}

/// Assesses candidates the way `strategy` needs: `current-default` is
/// cache-only; `default-with-capacity` returns the cache-only assessment when
/// it already shows a non-excluded, `available` default, and otherwise fetches
/// misses; `next-with-capacity` always fetches misses.
pub fn assess_for_strategy(
    set: &CandidateSet,
    excluded: &BTreeSet<String>,
    strategy: Strategy,
) -> Vec<CandidateCapacity> {
    match strategy {
        Strategy::CurrentDefault => assess_candidates(set, excluded, AssessMode::CacheOnly),
        Strategy::DefaultWithCapacity => {
            let cached = assess_candidates(set, excluded, AssessMode::CacheOnly);
            let settled = cached.iter().any(|candidate| {
                candidate.default
                    && !candidate.excluded
                    && candidate.capacity == Capacity::Available
            });
            if settled {
                cached
            } else {
                assess_candidates(set, excluded, AssessMode::CacheThenNetwork)
            }
        }
        Strategy::NextWithCapacity => {
            assess_candidates(set, excluded, AssessMode::CacheThenNetwork)
        }
    }
}

fn fresh_cache_snapshot(target_file: &Path) -> Option<RateLimitSnapshot> {
    let read = cache::read_cache_entry_allow_stale(target_file).ok()?;
    if read.stale {
        return None;
    }
    Some(RateLimitSnapshot::from_cache_entry(&read.entry))
}

fn fetch_snapshots(
    set: &CandidateSet,
    indexes: &[usize],
) -> Vec<(usize, Option<RateLimitSnapshot>)> {
    if indexes.is_empty() {
        return Vec::new();
    }
    // One wave: every miss (at most MAX_CANDIDATES) is fetched concurrently, so
    // the fetch phase takes about one CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS.
    thread::scope(|scope| {
        let handles: Vec<_> = indexes
            .iter()
            .map(|&index| {
                let candidate = &set.candidates[index];
                scope.spawn(move || {
                    (
                        index,
                        fetch_snapshot(&candidate.path, candidate.shared_cache),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect()
    })
}

/// Fetches usage once (no auth refresh) and, when the profile owns its cache
/// entry, writes the shared cache.
fn fetch_snapshot(target_file: &Path, write_cache: bool) -> Option<RateLimitSnapshot> {
    let request = UsageRequest {
        target_file: target_file.to_path_buf(),
        refresh_on_401: false,
        suppress_auth_refresh_output: true,
        base_url: std::env::var("CODEX_CHATGPT_BASE_URL")
            .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string()),
        connect_timeout_seconds: env_u64("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2),
        max_time_seconds: env_u64("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8),
    };
    let usage = fetch_usage(&request).ok()?;
    let data = render::parse_usage(&usage.json)?;
    let values = render::weekly_values(&render::render_values(&data));
    if values.weekly.is_none() && values.non_weekly.is_none() {
        return None;
    }
    let fetched_at_epoch = Utc::now().timestamp();
    if write_cache {
        let _ = cache::write_prompt_segment_cache(target_file, fetched_at_epoch, &values);
    }
    Some(RateLimitSnapshot::from_weekly_values(
        fetched_at_epoch,
        &values,
    ))
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

#[derive(Clone, Debug)]
pub struct SelectOptions {
    pub strategy: Strategy,
    pub after: Option<String>,
    pub exclude: Vec<String>,
    pub output_json: bool,
}

#[derive(Serialize)]
struct Thresholds {
    min_remaining_percent: i64,
}

#[derive(Serialize)]
struct SelectResult<'a> {
    strategy: Strategy,
    selected: String,
    default_account: Option<&'a str>,
    origin: Option<&'a str>,
    thresholds: Thresholds,
    cache_ttl_seconds: u64,
    candidates: &'a [CandidateCapacity],
}

pub fn run(options: &SelectOptions) -> Result<i32> {
    if options.after.is_some() && options.strategy != Strategy::NextWithCapacity {
        return emit_usage_error(
            options.output_json,
            "invalid-flag-combination",
            "--after is only valid with --strategy next-with-capacity.",
        );
    }
    let nicknames = options.after.iter().chain(options.exclude.iter());
    if nicknames.into_iter().any(|name| !valid_nickname(name)) {
        return emit_usage_error(
            options.output_json,
            "invalid-nickname",
            "Profile nicknames must match [A-Za-z0-9._-]{1,64}.",
        );
    }

    let set = match discover_candidates() {
        Ok(set) => set,
        Err(DiscoveryError::NoProfiles) => {
            return emit_error(
                options.output_json,
                "no-account-profiles",
                "No Codex account profiles are configured.",
                1,
                None,
            );
        }
    };
    if let Some(after) = options.after.as_deref()
        && !set.candidates.iter().any(|c| c.name == after)
    {
        let error = SelectError::UnknownOrigin;
        return emit_error(
            options.output_json,
            error.code(),
            error.message(),
            error.exit_code(),
            None,
        );
    }

    let excluded: BTreeSet<String> = options.exclude.iter().cloned().collect();
    let candidates = assess_for_strategy(&set, &excluded, options.strategy);
    let origin = effective_origin(options.strategy, &candidates, options.after.as_deref());

    match select(options.strategy, &candidates, options.after.as_deref()) {
        Ok(selected) => {
            if options.output_json {
                diag_output::emit_success_result(
                    SCHEMA_VERSION,
                    COMMAND,
                    SelectResult {
                        strategy: options.strategy,
                        selected,
                        default_account: set.default.as_deref(),
                        origin,
                        thresholds: Thresholds {
                            min_remaining_percent: MIN_REMAINING_PERCENT,
                        },
                        cache_ttl_seconds: cache::cache_ttl_seconds(),
                        candidates: &candidates,
                    },
                )?;
            } else {
                println!("{selected}");
            }
            Ok(0)
        }
        Err(error) => emit_error(
            options.output_json,
            error.code(),
            error.message(),
            error.exit_code(),
            Some(json!({
                "strategy": options.strategy,
                "default_account": set.default,
                "origin": origin,
                "candidates": candidates,
            })),
        ),
    }
}

fn emit_usage_error(output_json: bool, code: &str, message: &str) -> Result<i32> {
    emit_error(output_json, code, message, 64, None)
}

fn emit_error(
    output_json: bool,
    code: &str,
    message: &str,
    exit_code: i32,
    details: Option<serde_json::Value>,
) -> Result<i32> {
    if output_json {
        diag_output::emit_error(SCHEMA_VERSION, COMMAND, code, message, details)?;
    } else {
        eprintln!("codex-cli account select: {message}");
    }
    Ok(exit_code)
}

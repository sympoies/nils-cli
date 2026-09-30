//! Window values, the per-target cache entry format, and the one-line summary.

use crate::usage_cache_policy;

#[derive(Clone, Debug)]
pub struct WindowValues {
    pub label: String,
    pub remaining: i64,
    pub reset_epoch: i64,
}

/// A target's weekly window and its first non-weekly window.
pub struct WeeklyValues {
    pub weekly: Option<WindowValues>,
    pub non_weekly: Option<WindowValues>,
}

impl WeeklyValues {
    pub fn is_empty(&self) -> bool {
        self.weekly.is_none() && self.non_weekly.is_none()
    }
}

/// Compact "time left" until `target_epoch`: `" 3d  4h"` or `" 2h  5m"`.
pub fn format_until_epoch_compact(target_epoch: i64, now_epoch: i64) -> Option<String> {
    if target_epoch <= 0 || now_epoch <= 0 {
        return None;
    }
    let remaining = target_epoch - now_epoch;
    if remaining <= 0 {
        return Some(format!("{:>2}h {:>2}m", 0, 0));
    }

    if remaining >= 86_400 {
        let days = remaining / 86_400;
        let hours = (remaining % 86_400) / 3_600;
        return Some(format!("{:>2}d {:>2}h", days, hours));
    }

    let hours = remaining / 3_600;
    let minutes = (remaining % 3_600) / 60;
    Some(format!("{:>2}h {:>2}m", hours, minutes))
}

/// One cached rate-limit snapshot, stored as `key=value` lines.
#[derive(Debug)]
pub struct CacheEntry {
    pub fetched_at_epoch: Option<i64>,
    pub non_weekly_label: Option<String>,
    pub non_weekly_remaining: Option<i64>,
    pub non_weekly_reset_epoch: Option<i64>,
    pub weekly_remaining: Option<i64>,
    pub weekly_reset_epoch: Option<i64>,
}

impl CacheEntry {
    /// Whether the entry holds at least one window and no half-written window.
    pub fn is_complete(&self) -> bool {
        !(self.non_weekly_label.as_deref().is_some_and(str::is_empty)
            || self.non_weekly_label.is_some() != self.non_weekly_remaining.is_some()
            || self.weekly_remaining.is_some() != self.weekly_reset_epoch.is_some()
            || (self.non_weekly_remaining.is_none() && self.weekly_remaining.is_none()))
    }

    /// Whether the entry is older than `ttl_seconds` (or has no fetch time).
    pub fn is_stale(&self, now_epoch: i64, ttl_seconds: u64) -> bool {
        let fetched_at = match self.fetched_at_epoch {
            Some(value) if value > 0 => value,
            _ => return true,
        };
        if now_epoch <= 0 {
            return false;
        }
        let ttl_i64 = i64::try_from(ttl_seconds).unwrap_or(i64::MAX);
        now_epoch.saturating_sub(fetched_at) > ttl_i64
    }
}

pub fn parse_cache_entry(content: &str) -> CacheEntry {
    let mut entry = CacheEntry {
        fetched_at_epoch: None,
        non_weekly_label: None,
        non_weekly_remaining: None,
        non_weekly_reset_epoch: None,
        weekly_remaining: None,
        weekly_reset_epoch: None,
    };
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("fetched_at=") {
            entry.fetched_at_epoch = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("non_weekly_label=") {
            entry.non_weekly_label = Some(value.to_string());
        } else if let Some(value) = line.strip_prefix("non_weekly_remaining=") {
            entry.non_weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("non_weekly_reset_epoch=") {
            entry.non_weekly_reset_epoch = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_remaining=") {
            entry.weekly_remaining = value.parse::<i64>().ok();
        } else if let Some(value) = line.strip_prefix("weekly_reset_epoch=") {
            entry.weekly_reset_epoch = value.parse::<i64>().ok();
        }
    }
    entry
}

/// Serializes a snapshot; `None` when there is no window to store.
pub fn render_cache_entry(fetched_at_epoch: i64, values: &WeeklyValues) -> Option<String> {
    if values.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    lines.push(format!("fetched_at={fetched_at_epoch}"));
    if let Some(non_weekly) = &values.non_weekly {
        lines.push(format!("non_weekly_label={}", non_weekly.label));
        lines.push(format!("non_weekly_remaining={}", non_weekly.remaining));
        if non_weekly.reset_epoch > 0 {
            lines.push(format!("non_weekly_reset_epoch={}", non_weekly.reset_epoch));
        }
    }
    if let Some(weekly) = &values.weekly {
        lines.push(format!("weekly_remaining={}", weekly.remaining));
        lines.push(format!("weekly_reset_epoch={}", weekly.reset_epoch));
    }
    Some(lines.join("\n"))
}

/// Default freshness TTL of a cached entry, in seconds.
pub const DEFAULT_CACHE_TTL_SECONDS: u64 = 180;

/// The cache freshness TTL from `<PREFIX>_RATE_LIMITS_CACHE_TTL` (a duration
/// such as `90`, `5m`, or `1h`), else [`DEFAULT_CACHE_TTL_SECONDS`].
pub fn cache_ttl_seconds(env_prefix: &str) -> u64 {
    std::env::var(format!("{env_prefix}_RATE_LIMITS_CACHE_TTL"))
        .ok()
        .and_then(|raw| crate::env::parse_duration_seconds(&raw))
        .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
}

/// Whether `<PREFIX>_RATE_LIMITS_CACHE_ALLOW_STALE` lets `--cached` show an
/// entry past its TTL.
pub fn cache_allow_stale(env_prefix: &str) -> bool {
    crate::env::env_truthy_or(
        &format!("{env_prefix}_RATE_LIMITS_CACHE_ALLOW_STALE"),
        false,
    )
}

/// Whether a cache fetched at `fetched_at_epoch` is still within the fixed
/// display ceiling at `now_epoch`.
pub fn fetched_at_within_display_age(fetched_at_epoch: Option<i64>, now_epoch: i64) -> bool {
    let age_seconds = fetched_at_epoch
        .filter(|value| *value > 0 && now_epoch > 0)
        .map(|value| now_epoch.saturating_sub(value));
    usage_cache_policy::classify_display_age_seconds(age_seconds).is_display_eligible()
}

/// `"<label>:<n>% W:<n>% <reset>"`, omitting absent parts.
pub fn format_one_line_output(
    non_weekly_label: Option<&str>,
    non_weekly_remaining: Option<i64>,
    weekly_remaining: Option<i64>,
    weekly_reset_epoch: Option<i64>,
    format_reset: impl Fn(i64) -> Option<String>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let (Some(label), Some(remaining)) = (non_weekly_label, non_weekly_remaining) {
        parts.push(format!("{label}:{remaining}%"));
    }
    if let Some(remaining) = weekly_remaining {
        parts.push(format!("W:{remaining}%"));
        if let Some(reset_epoch) = weekly_reset_epoch {
            parts.push(format_reset(reset_epoch).unwrap_or_else(|| "?".to_string()));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// The one-line summary of a cache entry.
pub fn one_line_from_cache(
    entry: &CacheEntry,
    format_reset: impl Fn(i64) -> Option<String>,
) -> Option<String> {
    format_one_line_output(
        entry.non_weekly_label.as_deref(),
        entry.non_weekly_remaining,
        entry.weekly_remaining,
        entry.weekly_reset_epoch,
        format_reset,
    )
}

/// The one-line summary of live window values.
pub fn one_line_from_values(
    values: &WeeklyValues,
    format_reset: impl Fn(i64) -> Option<String>,
) -> Option<String> {
    format_one_line_output(
        values
            .non_weekly
            .as_ref()
            .map(|window| window.label.as_str()),
        values.non_weekly.as_ref().map(|window| window.remaining),
        values.weekly.as_ref().map(|window| window.remaining),
        values.weekly.as_ref().map(|window| window.reset_epoch),
        format_reset,
    )
}

pub fn normalize_one_line(line: String) -> String {
    line.replace(['\n', '\r', '\t'], " ")
}

pub struct ParsedOneLine {
    pub window_label: Option<String>,
    pub non_weekly_remaining: Option<i64>,
    pub weekly_remaining: Option<i64>,
    pub weekly_reset_iso: String,
}

pub fn parse_one_line_output(line: &str) -> Option<ParsedOneLine> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let weekly_index = parts.iter().position(|part| part.starts_with("W:"));
    let weekly_remaining = weekly_index.and_then(|index| {
        parts[index]
            .trim_start_matches("W:")
            .trim_end_matches('%')
            .parse::<i64>()
            .ok()
    });
    let non_weekly = parts.iter().enumerate().find_map(|(index, part)| {
        if Some(index) == weekly_index || !part.ends_with('%') || !part.contains(':') {
            return None;
        }
        let (label, remaining) = part.split_once(':')?;
        let remaining = remaining.trim_end_matches('%').parse::<i64>().ok()?;
        Some((label.trim_matches('"').to_string(), remaining))
    });
    let weekly_reset_iso = weekly_index
        .map(|index| parts[index + 1..].join(" "))
        .unwrap_or_default();
    if weekly_remaining.is_none() && non_weekly.is_none() {
        return None;
    }
    Some(ParsedOneLine {
        window_label: non_weekly.as_ref().map(|(label, _)| label.clone()),
        non_weekly_remaining: non_weekly.map(|(_, remaining)| remaining),
        weekly_remaining,
        weekly_reset_iso,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::{EnvGuard, GlobalStateLock};
    use pretty_assertions::assert_eq;

    #[test]
    fn cache_policy_env_is_read_under_the_provider_prefix() {
        let lock = GlobalStateLock::new();
        let _ttl = EnvGuard::remove(&lock, "EXAMPLE_RATE_LIMITS_CACHE_TTL");
        let _stale = EnvGuard::remove(&lock, "EXAMPLE_RATE_LIMITS_CACHE_ALLOW_STALE");
        assert_eq!(cache_ttl_seconds("EXAMPLE"), DEFAULT_CACHE_TTL_SECONDS);
        assert!(!cache_allow_stale("EXAMPLE"));

        let _ttl = EnvGuard::set(&lock, "EXAMPLE_RATE_LIMITS_CACHE_TTL", "5m");
        let _stale = EnvGuard::set(&lock, "EXAMPLE_RATE_LIMITS_CACHE_ALLOW_STALE", "true");
        assert_eq!(cache_ttl_seconds("EXAMPLE"), 300);
        assert!(cache_allow_stale("EXAMPLE"));

        let _invalid = EnvGuard::set(&lock, "EXAMPLE_RATE_LIMITS_CACHE_TTL", "soon");
        assert_eq!(cache_ttl_seconds("EXAMPLE"), DEFAULT_CACHE_TTL_SECONDS);
    }

    fn values() -> WeeklyValues {
        WeeklyValues {
            weekly: Some(WindowValues {
                label: "Weekly".to_string(),
                remaining: 60,
                reset_epoch: 1_700_500_000,
            }),
            non_weekly: Some(WindowValues {
                label: "5h".to_string(),
                remaining: 75,
                reset_epoch: 0,
            }),
        }
    }

    #[test]
    fn cache_entry_round_trips_and_validates_completeness() {
        let rendered = render_cache_entry(42, &values()).expect("rendered");
        assert_eq!(
            rendered,
            "fetched_at=42\nnon_weekly_label=5h\nnon_weekly_remaining=75\nweekly_remaining=60\nweekly_reset_epoch=1700500000"
        );
        let entry = parse_cache_entry(&rendered);
        assert!(entry.is_complete());
        assert_eq!(entry.non_weekly_reset_epoch, None);
        assert!(!entry.is_stale(100, 180));
        assert!(entry.is_stale(300, 180));
        assert!(!parse_cache_entry("fetched_at=1\nnon_weekly_label=5h").is_complete());
        assert!(
            render_cache_entry(
                1,
                &WeeklyValues {
                    weekly: None,
                    non_weekly: None
                }
            )
            .is_none()
        );
    }

    #[test]
    fn one_line_round_trips_through_the_parser() {
        let line =
            one_line_from_values(&values(), |_| Some("11-20 17:06".to_string())).expect("line");
        assert_eq!(line, "5h:75% W:60% 11-20 17:06");
        let parsed = parse_one_line_output(&line).expect("parsed");
        assert_eq!(parsed.window_label.as_deref(), Some("5h"));
        assert_eq!(parsed.non_weekly_remaining, Some(75));
        assert_eq!(parsed.weekly_remaining, Some(60));
        assert_eq!(parsed.weekly_reset_iso, "11-20 17:06");
        assert!(parse_one_line_output("bad").is_none());
        assert_eq!(normalize_one_line("a\tb\nc\r".to_string()), "a b c ");
    }

    #[test]
    fn compact_until_covers_past_hours_and_days() {
        assert_eq!(format_until_epoch_compact(0, 10), None);
        assert_eq!(
            format_until_epoch_compact(5, 10).as_deref(),
            Some(" 0h  0m")
        );
        assert_eq!(
            format_until_epoch_compact(10 + 3_660, 10).as_deref(),
            Some(" 1h  1m")
        );
        assert_eq!(
            format_until_epoch_compact(10 + 90_000, 10).as_deref(),
            Some(" 1d  1h")
        );
    }

    #[test]
    fn display_age_uses_the_shared_ceiling() {
        assert!(fetched_at_within_display_age(Some(1_000), 1_100));
        assert!(!fetched_at_within_display_age(Some(1_000), 2_000));
        assert!(!fetched_at_within_display_age(None, 2_000));
    }
}

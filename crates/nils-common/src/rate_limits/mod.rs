//! Shared `diag rate-limits` orchestration for provider CLIs.
//!
//! - [`driver`] runs the command: flag validation, target selection, `--all`,
//!   `--async`, `--watch`, `--jobs`, and the JSON collection envelopes. A
//!   provider plugs in through [`RateLimitsProvider`].
//! - [`schema`] owns the result shape every provider emits.
//! - [`table`] renders the all-accounts table.
//! - [`values`] owns window values, the cache entry format, and the one-line
//!   summary.
//!
//! Wall-clock time and local-time formatting come from the provider, so this
//! module stays deterministic.

pub mod driver;
pub mod schema;
pub mod table;
pub mod values;

pub use driver::{
    CacheFallbackPolicy, OneLineFetch, ProgressSink, ProviderSpec, RC_NO_RATE_LIMIT_WINDOW,
    RateLimitsProvider, ResetEpochs, RunOptions, TargetDiscoveryError, run,
};
pub use schema::{
    RateLimitResult, RateLimitSummary, RateLimitWindow, ResetCredits, TargetIdentity,
};
pub use values::{CacheEntry, WeeklyValues, WindowValues};

#[cfg(test)]
pub(crate) mod test_support {
    /// Formats `epoch` in UTC, standing in for a provider's local-time formatter.
    pub(crate) fn utc_format(epoch: i64, format: &str) -> Option<String> {
        let timestamp = jiff::Timestamp::from_second(epoch).ok()?;
        Some(
            timestamp
                .to_zoned(jiff::tz::TimeZone::UTC)
                .strftime(format)
                .to_string(),
        )
    }
}

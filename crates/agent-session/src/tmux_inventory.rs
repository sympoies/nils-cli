//! Passive tmux inventory. Never use it as a pane identity or mutation guard.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;

use jiff::Timestamp;

use super::{
    format_tmux_window_activity, parse_tmux_window_activity_seconds, tmux_output_reports_absent,
};

#[derive(Debug, Clone)]
struct TmuxSessionSnapshot {
    last_terminal_activity_at: Option<String>,
}

pub(super) struct TmuxSessionSnapshots {
    pub(super) started_at: Timestamp,
    sessions: BTreeMap<String, TmuxSessionSnapshot>,
}

// tmux sanitizes control characters in format output under a C locale. Keep
// the bulk snapshot delimiter printable, parse it from the right so even an
// unusual pre-existing session name containing the delimiter remains intact,
// and continue accepting prior tab-delimited fixture or wrapper output.
const TMUX_SESSION_SNAPSHOT_FORMAT: &str = "#{session_name}|#{window_activity}";
const TMUX_SESSION_SNAPSHOT_SEPARATOR: char = '|';

impl TmuxSessionSnapshots {
    pub(super) fn contains_key(&self, tmux_session: &str) -> bool {
        self.sessions.contains_key(tmux_session)
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub(super) fn last_activity(&self, session: &str) -> Option<&str> {
        self.sessions
            .get(session)?
            .last_terminal_activity_at
            .as_deref()
    }
}

pub(super) fn tmux_session_snapshots(tmux_bin: &Path) -> Option<TmuxSessionSnapshots> {
    let started_at = Timestamp::now();
    parse_snapshot(started_at, snapshot_command(tmux_bin).output().ok()?)
}

pub(super) fn bounded_snapshot(tmux_bin: &Path) -> Option<TmuxSessionSnapshots> {
    let started_at = Timestamp::now();
    let output = super::run_output_with_timeout_and_strict_cap(
        snapshot_command(tmux_bin),
        Duration::from_millis(250),
        64 * 1024,
    )
    .ok()?;
    // The auth observer has always distinguished a failed inventory command
    // (degraded) from successful inventory with no matching session.
    if !output.status.success() {
        return None;
    }
    parse_snapshot(started_at, output)
}

fn snapshot_command(tmux_bin: &Path) -> Command {
    let mut command = Command::new(tmux_bin);
    command
        .env("LC_ALL", "C")
        .args(["list-windows", "-a", "-F", TMUX_SESSION_SNAPSHOT_FORMAT]);
    command
}

fn parse_snapshot(started_at: Timestamp, output: Output) -> Option<TmuxSessionSnapshots> {
    if !output.status.success() {
        if tmux_output_reports_absent(&output) {
            return Some(TmuxSessionSnapshots {
                started_at,
                sessions: BTreeMap::new(),
            });
        }
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut activity_by_session: BTreeMap<String, Option<i64>> = BTreeMap::new();
    for line in raw.lines() {
        let Some((session, activity)) = line
            .rsplit_once(TMUX_SESSION_SNAPSHOT_SEPARATOR)
            .or_else(|| line.split_once('\t'))
        else {
            continue;
        };
        if session.is_empty() {
            continue;
        }
        let Some(epoch_seconds) = parse_tmux_window_activity_seconds(activity) else {
            activity_by_session
                .entry(session.to_string())
                .or_insert(None);
            continue;
        };
        activity_by_session
            .entry(session.to_string())
            .and_modify(|current| {
                *current = Some(current.map_or(epoch_seconds, |value| value.max(epoch_seconds)));
            })
            .or_insert(Some(epoch_seconds));
    }
    Some(TmuxSessionSnapshots {
        started_at,
        sessions: activity_by_session
            .into_iter()
            .map(|(session, maybe_epoch_seconds)| {
                (
                    session,
                    TmuxSessionSnapshot {
                        last_terminal_activity_at: maybe_epoch_seconds
                            .and_then(format_tmux_window_activity),
                    },
                )
            })
            .collect(),
    })
}

// Recheck even a quiet pane periodically, without delaying normal observation
// when activity changes. tmux window_activity has one-second resolution.
pub(super) const QUIET_REPROBE_SECONDS: i64 = 300;

pub(super) fn quiet_since_sample(
    window_activity: Option<&str>,
    sampled_at: &str,
    now: Timestamp,
) -> bool {
    window_activity
        .and_then(|at| at.parse::<Timestamp>().ok())
        .zip(sampled_at.parse::<Timestamp>().ok())
        .is_some_and(|(activity, sampled)| {
            activity.as_second() < sampled.as_second()
                && (0..QUIET_REPROBE_SECONDS).contains(&(now.as_second() - sampled.as_second()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn bounded_inventory_batches_sessions_and_takes_latest_window() {
        let dir = tempfile::TempDir::new().unwrap();
        let tmux = dir.path().join("tmux");
        let calls = dir.path().join("calls");
        fs::write(&tmux, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\nprintf 'first|200\\nsecond|100\\nfirst|300\\n'\n",
            shell_words::quote(&calls.to_string_lossy()),
        )).unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let snapshot = bounded_snapshot(&tmux).unwrap();

        assert!(snapshot.contains_key("first"));
        assert!(snapshot.contains_key("second"));
        assert!(!snapshot.contains_key("other"));
        assert_eq!(
            snapshot.last_activity("first"),
            Some("1970-01-01T00:05:00Z")
        );
        assert_eq!(
            snapshot.last_activity("second"),
            Some("1970-01-01T00:01:40Z")
        );
        assert_eq!(
            fs::read_to_string(&calls).unwrap(),
            format!("list-windows -a -F {TMUX_SESSION_SNAPSHOT_FORMAT}\n"),
        );
    }
}

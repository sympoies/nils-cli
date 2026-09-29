//! `agent-session board` (`session-board-v1`, "CLI").
//!
//! Local mode builds an `agent-session.board-view.v1` object from this
//! machine's state directory: the same records as `GET /board/v1`, the closed
//! rows from the local ledger, and the query filters applied here.
//!
//! Relay mode is a managed session asking the aggregator through the
//! daemon's `GET /sessions/{id}/board/v1` (`relay.rs`); `select_mode` decides
//! between the two, and `data.mode` reports the choice.

use std::cmp::Ordering;

use nils_common::cli_contract::OutputFormat;
use serde_json::{Value, json};

use super::relay::{self, Caller};
use crate::cli::BoardArgs;
use crate::{CliContext, CliError};

pub(crate) const VIEW_SCHEMA: &str = "agent-session.board-view.v1";
const COMMAND: &str = "board";
/// Local mode's source retention is the closed-ledger bound.
const LOCAL_RETENTION: &str = "7d";
const MAX_RECORDS: usize = 1024;
const TITLE_WIDTH: usize = 60;
const REPO_WIDTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StateFilter {
    Live,
    Stopped,
    Closed,
    All,
}

impl StateFilter {
    fn parse(raw: &str) -> Result<Self, CliError> {
        match raw {
            "live" => Ok(Self::Live),
            "stopped" => Ok(Self::Stopped),
            "closed" => Ok(Self::Closed),
            "all" => Ok(Self::All),
            _ => Err(query_invalid(
                "--state must be live, stopped, closed, or all",
            )),
        }
    }

    fn admits(self, state: &str) -> bool {
        match self {
            Self::Live => state == "live",
            Self::Stopped => state == "stopped",
            Self::Closed => state == "closed",
            Self::All => true,
        }
    }
}

struct Query {
    state: StateFilter,
    since_seconds: Option<i64>,
    repo: Option<String>,
    machine: Option<String>,
}

pub(super) fn query_invalid(message: &str) -> CliError {
    CliError::usage("board-query-invalid", message, None)
}

/// `<n><unit>` with unit `m`, `h`, `d`, `w`, or `mo` (31 days); `n` >= 1.
fn parse_duration_seconds(raw: &str) -> Result<i64, CliError> {
    let invalid = || query_invalid("--since must be <n> with unit m, h, d, w, or mo");
    let digits = raw.bytes().take_while(u8::is_ascii_digit).count();
    let (number, unit) = raw.split_at(digits);
    let unit_seconds = match unit {
        "m" => 60,
        "h" => 60 * 60,
        "d" => 24 * 60 * 60,
        "w" => 7 * 24 * 60 * 60,
        "mo" => 31 * 24 * 60 * 60,
        _ => return Err(invalid()),
    };
    let count: i64 = number.parse().map_err(|_| invalid())?;
    if count < 1 {
        return Err(invalid());
    }
    count.checked_mul(unit_seconds).ok_or_else(invalid)
}

fn parse_query(args: &BoardArgs) -> Result<Query, CliError> {
    Ok(Query {
        state: StateFilter::parse(&args.state)?,
        since_seconds: args
            .since
            .as_deref()
            .map(parse_duration_seconds)
            .transpose()?,
        repo: args.repo.clone(),
        machine: args.machine.clone(),
    })
}

/// Relay mode when a trusted managed session reaches the daemon relay route;
/// local mode without a trusted identity or daemon endpoint, or when the
/// daemon answers `board-disabled` or `board-relay-disabled`. Any other relay
/// failure is returned, never replaced by a local view.
fn select_mode(
    context: &CliContext,
    args: &BoardArgs,
    query: &Query,
    caller: Option<&Caller>,
) -> Result<Value, CliError> {
    if let Some(caller) = caller {
        let mut filters = vec![("state", args.state.as_str())];
        for (key, value) in [
            ("since", &args.since),
            ("repo", &args.repo),
            ("machine", &args.machine),
        ] {
            if let Some(value) = value {
                filters.push((key, value.as_str()));
            }
        }
        if let Some(board) = relay::fetch(context, caller, &filters)? {
            return Ok(json!({ "mode": "relay", "board": board }));
        }
    }
    local_view(context, query).map(|board| json!({ "mode": "local", "board": board }))
}

pub(crate) fn run(context: &CliContext, args: BoardArgs) -> i32 {
    let format = args.format;
    let result = parse_query(&args).and_then(|query| {
        let caller = relay::caller(context);
        select_mode(context, &args, &query, caller.as_ref()).map(|data| (data, caller))
    });
    match result {
        Ok((data, caller)) => match format {
            OutputFormat::Json => {
                crate::render_single_success(COMMAND, format, &data, |_| String::new())
            }
            OutputFormat::Text => {
                print!("{}", render_text(&data, caller.as_ref()));
                0
            }
        },
        Err(err) => crate::render_error(COMMAND, format, err),
    }
}

fn timestamp_seconds(value: &Value) -> Option<i64> {
    value
        .as_str()?
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_second())
}

/// The key a record is ordered and windowed by: `updated_at`, or `closed_at`
/// for a closed row.
fn activity_key(record: &Value) -> &Value {
    if record["state"] == "closed" {
        &record["closed_at"]
    } else {
        &record["updated_at"]
    }
}

fn state_rank(record: &Value) -> u8 {
    match record["state"].as_str() {
        Some("live") => 0,
        Some("stopped") => 1,
        _ => 2,
    }
}

fn compare_records(a: &Value, b: &Value) -> Ordering {
    state_rank(a)
        .cmp(&state_rank(b))
        .then_with(|| {
            let (a_key, b_key) = (activity_key(a), activity_key(b));
            match (timestamp_seconds(a_key), timestamp_seconds(b_key)) {
                (Some(a_time), Some(b_time)) => b_time.cmp(&a_time),
                _ => b_key.as_str().cmp(&a_key.as_str()),
            }
        })
        .then_with(|| a["machine"].as_str().cmp(&b["machine"].as_str()))
        .then_with(|| a["session_id"].as_str().cmp(&b["session_id"].as_str()))
}

fn local_view(context: &CliContext, query: &Query) -> Result<Value, CliError> {
    let machine = super::machine_identity(None, context);
    let generated = jiff::Timestamp::now();
    let retention_seconds = super::ledger::MAX_AGE_SECONDS;
    let since_capped = query
        .since_seconds
        .is_some_and(|since| since > retention_seconds);
    let window = query
        .since_seconds
        .unwrap_or(retention_seconds)
        .min(retention_seconds);
    let effective_since = generated.as_second() - window;

    // A CLI process cannot see daemon federation, so messaging is never
    // claimed here.
    let snapshot = super::snapshot(context, &crate::resolve_tmux_bin(None), &machine, false)?;
    let closed = super::closed(context, None, &machine)?;
    let open_records = snapshot["records"].as_array().cloned().unwrap_or_default();
    let closed_records = closed["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| entry["record"].clone());

    let mut records: Vec<Value> = open_records
        .into_iter()
        .chain(closed_records)
        .filter(|record| {
            query
                .state
                .admits(record["state"].as_str().unwrap_or_default())
        })
        .filter(|record| {
            record["state"] == "live"
                || timestamp_seconds(activity_key(record))
                    .is_none_or(|seconds| seconds >= effective_since)
        })
        .filter(|record| {
            query
                .repo
                .as_deref()
                .is_none_or(|repo| record["repo_name"].as_str() == Some(repo))
        })
        .filter(|record| {
            query
                .machine
                .as_deref()
                .is_none_or(|name| record["machine"].as_str() == Some(name))
        })
        .collect();
    records.sort_by(compare_records);
    let truncated = records.len() > MAX_RECORDS;
    records.truncate(MAX_RECORDS);

    let generated_at = generated.to_string();
    let effective_since = jiff::Timestamp::from_second(effective_since)
        .map(|timestamp| timestamp.to_string())
        .unwrap_or_else(|_| generated_at.clone());
    Ok(json!({
        "schema_version": VIEW_SCHEMA,
        "record_schema": super::RECORD_SCHEMA,
        "generated_at": generated_at,
        "retention": LOCAL_RETENTION,
        "effective_since": effective_since,
        "since_capped": since_capped,
        "machines": [{
            "machine": machine,
            "available": true,
            "last_seen_at": generated_at,
        }],
        "records": records,
        "truncated": truncated,
    }))
}

fn truncate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(width.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

/// A single-line, width-bounded field; `-` when absent.
fn field(value: &Value, width: usize) -> String {
    match value.as_str() {
        Some(text) if !text.trim().is_empty() => {
            let single_line: String = text
                .chars()
                .map(|character| {
                    if character.is_control() {
                        ' '
                    } else {
                        character
                    }
                })
                .collect();
            truncate(single_line.trim(), width)
        }
        _ => "-".to_string(),
    }
}

fn age(value: &Value, now: i64) -> String {
    let Some(then) = timestamp_seconds(value) else {
        return "-".to_string();
    };
    let seconds = now.saturating_sub(then).max(0);
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// Whether a record is the caller's own session: same id and incarnation.
fn is_caller(record: &Value, caller: Option<&Caller>) -> bool {
    caller.is_some_and(|caller| {
        record["session_id"].as_str() == Some(caller.session_id.as_str())
            && record["session_incarnation"].as_str() == Some(caller.incarnation.as_str())
    })
}

/// Mode, then one line per unavailable machine, then one line per record.
/// The caller's own session is marked, and a peer that can currently receive
/// a remote message carries its `message send` target. `summary` is never
/// printed.
fn render_text(data: &Value, caller: Option<&Caller>) -> String {
    let board = &data["board"];
    let now = timestamp_seconds(&board["generated_at"])
        .unwrap_or_else(|| jiff::Timestamp::now().as_second());
    let mut out = format!("mode: {}\n", data["mode"].as_str().unwrap_or("local"));
    for machine in board["machines"].as_array().into_iter().flatten() {
        if machine["available"] == false {
            out.push_str(&format!(
                "unavailable  {}  last seen {}\n",
                field(&machine["machine"], REPO_WIDTH),
                field(&machine["last_seen_at"], TITLE_WIDTH),
            ));
        }
    }
    for record in board["records"].as_array().into_iter().flatten() {
        let turn = &record["turn_state"];
        let progress = match &turn["current_turn"]["last_progress_at"] {
            Value::String(_) => &turn["current_turn"]["last_progress_at"],
            _ => &turn["phase_changed_at"],
        };
        out.push_str(&format!(
            "{}  {}  {}  {}  {}  {}  {}",
            field(&record["state"], REPO_WIDTH),
            field(&record["machine"], REPO_WIDTH),
            field(&record["session_id"], TITLE_WIDTH),
            field(&record["repo_name"], REPO_WIDTH),
            field(&turn["phase"], REPO_WIDTH),
            age(progress, now),
            field(&record["title"], TITLE_WIDTH),
        ));
        if is_caller(record, caller) {
            out.push_str("  (this session)");
        } else if record["messaging_supported"] == true && record["session_incarnation"].is_string()
        {
            out.push_str(&format!(
                "  send: --to-machine {} --to {} (incarnation {})",
                field(&record["machine"], REPO_WIDTH),
                field(&record["session_id"], TITLE_WIDTH),
                field(&record["session_incarnation"], TITLE_WIDTH),
            ));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn durations_accept_exactly_the_documented_units() {
        for (raw, seconds) in [
            ("30m", 30 * 60),
            ("2h", 2 * 3_600),
            ("3d", 3 * 86_400),
            ("2w", 14 * 86_400),
            ("1mo", 31 * 86_400),
        ] {
            assert_eq!(parse_duration_seconds(raw).expect(raw), seconds, "{raw}");
        }
        for raw in [
            "",
            "d",
            "0d",
            "-1d",
            "3x",
            "3 d",
            "3D",
            "1.5h",
            "99999999999999999999w",
        ] {
            let error = parse_duration_seconds(raw).expect_err(raw).into_inner();
            assert_eq!(error.code, "board-query-invalid", "{raw}");
        }
    }

    #[test]
    fn records_order_by_state_then_newest_then_identity() {
        let record = |state: &str, machine: &str, id: &str, at: &str| {
            let key = if state == "closed" {
                "closed_at"
            } else {
                "updated_at"
            };
            json!({"state": state, "machine": machine, "session_id": id, key: at})
        };
        let mut records = [
            record("closed", "a", "c1", "2030-01-01T00:05:00Z"),
            record("stopped", "a", "s1", "2030-01-01T00:01:00Z"),
            record("live", "b", "l2", "2030-01-01T00:03:00Z"),
            record("live", "a", "l1", "2030-01-01T00:03:00Z"),
            record("stopped", "a", "s2", "2030-01-01T00:02:00.5Z"),
        ];
        records.sort_by(compare_records);
        let ids: Vec<&str> = records
            .iter()
            .map(|record| record["session_id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec!["l1", "l2", "s2", "s1", "c1"]);
    }

    #[test]
    fn text_uses_progress_age_and_bounds_every_field() {
        let data = json!({
            "mode": "relay",
            "board": {
                "generated_at": "2030-01-01T01:00:00Z",
                "machines": [
                    {"machine": "host-a", "available": true, "last_seen_at": "2030-01-01T01:00:00Z"},
                    {"machine": "host-b", "available": false, "last_seen_at": null}
                ],
                "records": [{
                    "state": "live",
                    "machine": "host-a",
                    "session_id": "s1",
                    "repo_name": "repo",
                    "title": format!("{}\nsecond line", "t".repeat(80)),
                    "summary": "never printed",
                    "turn_state": {
                        "phase": "working",
                        "phase_changed_at": "2030-01-01T00:00:00Z",
                        "current_turn": {"last_progress_at": "2030-01-01T00:55:00Z"}
                    }
                }, {
                    "state": "stopped",
                    "machine": "host-a",
                    "session_id": "s2",
                    "repo_name": null,
                    "title": null,
                    "turn_state": {"phase": "waiting", "phase_changed_at": "2029-12-31T01:00:00Z", "current_turn": null}
                }]
            }
        });
        let text = render_text(&data, None);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "mode: relay");
        assert_eq!(lines[1], "unavailable  host-b  last seen -");
        assert_eq!(
            lines[2],
            format!("live  host-a  s1  repo  working  5m  {}…", "t".repeat(59))
        );
        assert_eq!(lines[3], "stopped  host-a  s2  -  waiting  1d  -");
        assert_eq!(lines.len(), 4);
        assert!(!text.contains("never printed"));
    }
}

//! Session board v1 (`docs/specs/session-board-v1.md`).
//!
//! The board is a separate, opt-in projection of the local session records. It
//! carries only the allowlisted record fields, a home-relative `cwd`, and the
//! serving machine's identity. `serve.rs` only registers the routes and calls
//! into this module.
//!
//! This module also owns the machine label shared by `serve`, the always-on
//! `list --format json` `machine` field, and board CLI local mode, so all
//! three resolve the same identity.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{CliContext, CliError, SessionRecord, SessionView};

mod ledger;
mod programs;
mod relay;
mod view;

pub(crate) use ledger::{CloseReason, cursor_invalid};
pub(crate) use programs::ProgramCache;
pub(crate) use relay::relay_route;
pub(crate) use view::run;

pub(crate) const BOARD_SCHEMA: &str = "agent-session.board.v1";
pub(crate) const RECORD_SCHEMA: &str = "agent-session.board-record.v1";
/// Additive capabilities of the v1 envelopes. A reader feature-detects from
/// this list, never from the presence of a record field.
pub(crate) const EXTENSIONS: [&str; 2] = ["lineage.v1", "work.v1"];
/// What the daemon snapshot names: the extensions plus the programs route.
const DAEMON_EXTENSIONS: [&str; 3] = ["lineage.v1", "work.v1", "programs.v1"];
/// `AGENT_SESSION_BOARD=1` enables the daemon board routes, like `--board`.
pub(crate) const BOARD_ENV: &str = "AGENT_SESSION_BOARD";

/// Whether `serve` answers the board routes.
pub(crate) fn serve_enabled(flag: bool) -> bool {
    flag || crate::env_truthy(BOARD_ENV)
}

/// Failure for every board route while the board is disabled (HTTP 404). It
/// is decided before authentication and any state read.
pub(crate) const DISABLED_CODE: &str = "board-disabled";
pub(crate) const DISABLED_MESSAGE: &str = "the session board is disabled on this daemon";

/// The machine label this process reports: an explicit serve `--machine`,
/// then `AGENT_SESSION_MACHINE`, then the `--host` / `AGENT_SESSION_HOST`
/// identity, then the short hostname. `serve` and `list` share it so a list
/// record names the same machine as the serve envelope.
pub(crate) fn machine_identity(explicit: Option<String>, context: &CliContext) -> String {
    explicit
        .or_else(|| crate::non_empty_env("AGENT_SESSION_MACHINE"))
        .or_else(|| context.host.clone())
        .or_else(crate::short_hostname)
        .unwrap_or_else(|| "unknown".to_string())
}

/// A `list --format json` record: the existing session view plus `machine`.
#[derive(Serialize)]
pub(crate) struct ListRecord<'a> {
    #[serde(flatten)]
    view: &'a SessionView,
    machine: &'a str,
}

pub(crate) fn list_records<'a>(views: &'a [SessionView], machine: &'a str) -> Vec<ListRecord<'a>> {
    views
        .iter()
        .map(|view| ListRecord { view, machine })
        .collect()
}

/// `GET /board/v1` data: this machine's `live` and `stopped` records, sorted by
/// session id. A record that cannot be read or projected is omitted and
/// counted in `skipped_count`, never guessed.
pub(crate) fn snapshot(
    context: &CliContext,
    tmux_bin: &Path,
    machine: &str,
    federation_configured: bool,
) -> Result<Value, CliError> {
    snapshot_with(
        context,
        tmux_bin,
        machine,
        federation_configured,
        &mut || {},
    )
}

/// `snapshot` with a hook between reading the ledger head and enumerating
/// records, so a test can remove a record in exactly that window.
fn snapshot_with(
    context: &CliContext,
    tmux_bin: &Path,
    machine: &str,
    federation_configured: bool,
    after_ledger_head: &mut dyn FnMut(),
) -> Result<Value, CliError> {
    // Read before enumeration: a record removed after this point is in the
    // closed ledger after this cursor.
    let ledger_cursor = ledger::head_cursor(context)?;
    after_ledger_head();
    let home = home_dir();
    let mut records = Vec::new();
    let mut skipped_count = 0_u64;
    crate::visit_session_views(context, Some(tmux_bin), false, &mut |view| {
        let projected = view.ok().and_then(|view| {
            let view = serde_json::to_value(&view).ok()?;
            project_record(&view, machine, home.as_deref(), federation_configured)
        });
        match projected {
            Some(record) => records.push(record),
            None => skipped_count += 1,
        }
        Ok(())
    })?;
    records.sort_by(|a, b| {
        a["session_id"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["session_id"].as_str().unwrap_or_default())
    });
    Ok(json!({
        "schema_version": BOARD_SCHEMA,
        "record_schema": RECORD_SCHEMA,
        "machine": machine,
        "generated_at": jiff::Timestamp::now().to_string(),
        "extensions": DAEMON_EXTENSIONS,
        "ledger_cursor": ledger_cursor,
        "records": records,
        "skipped_count": skipped_count,
    }))
}

/// `GET /board/closed/v1` data: retained closed records after `since`.
pub(crate) fn closed(
    context: &CliContext,
    since: Option<&str>,
    machine: &str,
) -> Result<Value, CliError> {
    ledger::read_since(context, since, machine)
}

/// The closed record for a session about to be removed, built while its
/// state is still readable. `None` when it cannot be projected; the removal
/// then proceeds and the aggregator later classifies the row as vanished.
pub(crate) fn closed_record(
    context: &CliContext,
    record: &SessionRecord,
    reason: CloseReason,
) -> Option<Value> {
    let view = crate::session_view_from_parts(
        context,
        record,
        "stopped".to_string(),
        None,
        &crate::resolve_tmux_bin(None),
        false,
        crate::coordination::CoordinationSummary::default(),
    );
    let view = serde_json::to_value(&view).ok()?;
    let mut closed = project_record(&view, "", home_dir().as_deref(), false)?;
    let object = closed.as_object_mut()?;
    // Ledger entries never store `machine`: the serving daemon stamps it.
    object.remove("machine");
    object.insert("state".into(), json!("closed"));
    object.insert("runtime_status".into(), Value::Null);
    object.insert("messaging_supported".into(), json!(false));
    object.insert("close_reason".into(), json!(reason.as_str()));
    Some(closed)
}

/// Append a close after the removal committed. A failure never reaches the
/// caller; it is recorded content-free in the observation spool.
pub(crate) fn record_close(context: &CliContext, closed: Option<Value>) {
    let appended = closed.is_some_and(|closed| ledger::append(context, closed).is_ok());
    if appended {
        return;
    }
    use nils_common::observation::{self, Component, Event, Severity};
    if let Ok(event) = Event::new(
        Component::AgentSession,
        "board-ledger",
        "board-ledger-append-failed",
        Severity::Warn,
        env!("CARGO_PKG_VERSION"),
        jiff::Timestamp::now().as_second(),
    ) {
        let _ = observation::append(&context.state_dir, &event);
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
}

/// `cwd` relative to the daemon user's home as `~` or `~/...`; `None` for a
/// path outside home, a non-normalized path, or an unusable home.
pub(crate) fn home_relative_cwd(cwd: &str, home: Option<&Path>) -> Option<String> {
    let home = home?;
    // A home of `/` would make every absolute path "home-relative".
    home.parent()?;
    let cwd = Path::new(cwd);
    let normalized = |path: &Path| {
        path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
    };
    if !normalized(cwd) || !normalized(home) {
        return None;
    }
    let relative = cwd.strip_prefix(home).ok()?;
    let relative = relative.to_str()?;
    Some(if relative.is_empty() {
        "~".to_string()
    } else {
        format!("~/{relative}")
    })
}

/// Project one serialized `SessionView` into a board record. Only the fields
/// in the spec's field-source table are read; everything else is dropped.
/// Returns `None` when a required source field is missing or mistyped.
pub(crate) fn project_record(
    view: &Value,
    machine: &str,
    home: Option<&Path>,
    federation_configured: bool,
) -> Option<Value> {
    let view = view.as_object()?;
    let string = |key: &str| view.get(key).and_then(Value::as_str);
    let nullable = |key: &str| {
        string(key)
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null)
    };
    let session_id = string("id")?;
    let provider = string("agent")?;
    let status = string("status")?;
    let created_at = string("created_at")?;
    let updated_at = string("updated_at")?;
    let state = if status == "running" {
        "live"
    } else {
        "stopped"
    };
    let incarnation = string("session_incarnation");
    let messaging_supported = federation_configured
        && state == "live"
        && incarnation.is_some()
        && string("coordination_mode") != Some("off")
        && view.get("coordination_available").and_then(Value::as_bool) == Some(true);
    let title_state = match view.get("title_state") {
        Some(Value::Object(title_state)) => json!({
            "activity": title_state.get("activity").filter(|value| value.is_string()).cloned().unwrap_or(Value::Null),
        }),
        _ => Value::Null,
    };
    let turn_state = view
        .get("turn_state")
        .and_then(Value::as_object)
        .map(project_turn_state)
        .unwrap_or(Value::Null);
    let mut projected = json!({
        "machine": machine,
        "session_id": session_id,
        "session_incarnation": nullable("session_incarnation"),
        "messaging_supported": messaging_supported,
        "repo_name": nullable("repo_name"),
        "cwd": string("cwd").and_then(|cwd| home_relative_cwd(cwd, home)),
        "provider": provider,
        "agent_profile": nullable("agent_profile"),
        "model": nullable("model"),
        "reasoning_effort": nullable("reasoning_effort"),
        "title": nullable("title"),
        "title_state": title_state,
        "turn_state": turn_state,
        "state": state,
        "runtime_status": status,
        "created_at": created_at,
        "updated_at": updated_at,
        "closed_at": null,
        "close_reason": null,
        "summary": null,
        "role": nullable("role"),
        "lineage": project_lineage(view),
        "work": project_work(view),
    });
    if let Some(incident) = view.get("auth_incident").and_then(|value| {
        serde_json::from_value::<crate::auth_incident::AuthIncident>(value.clone()).ok()
    }) {
        projected["auth_incident"] = json!(incident);
    }
    if let Some(health) = view
        .get("auth_detection_health")
        .and_then(Value::as_str)
        .filter(|value| {
            matches!(
                *value,
                "structured"
                    | "degraded_terminal_fallback"
                    | "degraded_source_unavailable"
                    | "degraded_store_unavailable"
                    | "degraded_queue_capacity"
            )
        })
    {
        projected["auth_detection_health"] = json!(health);
    }
    Some(projected)
}

/// A session reference as `{machine, session_id, session_created_at}`; the
/// incarnation is never part of a board reference. `null` when any member is
/// missing or mistyped.
fn project_session_ref(value: Option<&Value>) -> Value {
    let Some(reference) = value.and_then(Value::as_object) else {
        return Value::Null;
    };
    let member = |key: &str| {
        reference
            .get(key)
            .filter(|value| value.is_string())
            .cloned()
    };
    match (
        member("machine"),
        member("session_id"),
        member("session_created_at"),
    ) {
        (Some(machine), Some(session_id), Some(session_created_at)) => json!({
            "machine": machine,
            "session_id": session_id,
            "session_created_at": session_created_at,
        }),
        _ => Value::Null,
    }
}

/// `lineage` of a board record: the stored lineage plus `effective_parent`
/// (the adopting steward when there is one, otherwise `parent`). `null` for a
/// session without lineage.
fn project_lineage(view: &Map<String, Value>) -> Value {
    let Some(lineage) = view.get("lineage").and_then(Value::as_object) else {
        return Value::Null;
    };
    let root = project_session_ref(lineage.get("root"));
    let (Some(depth), Some(starter), false) = (
        lineage.get("depth").filter(|depth| depth.is_u64()),
        lineage.get("starter").and_then(Value::as_object),
        root.is_null(),
    ) else {
        return Value::Null;
    };
    let text = |key: &str| {
        starter
            .get(key)
            .filter(|value| value.is_string())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let parent = project_session_ref(lineage.get("parent"));
    let adopted = project_session_ref(
        view.get("lineage_adoption")
            .and_then(|adoption| adoption.get("adopted_by")),
    );
    let effective_parent = if adopted.is_null() {
        parent.clone()
    } else {
        adopted
    };
    json!({
        "parent": parent,
        "effective_parent": effective_parent,
        "root": root,
        "depth": depth,
        "starter": {"kind": text("kind"), "via": text("via")},
        "budget": lineage.get("budget").cloned().unwrap_or(Value::Null),
    })
}

/// `work` of a board record: program, issues and whether they were inherited.
/// `null` for a session without work references.
fn project_work(view: &Map<String, Value>) -> Value {
    let Some(work) = view.get("work").and_then(Value::as_object) else {
        return Value::Null;
    };
    let program = work
        .get("program")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(Value::Null);
    let issues = work
        .get("issues")
        .filter(|value| value.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));
    // A cleared `work set` leaves an empty object: that is no work.
    if program.is_null() && issues.as_array().is_some_and(Vec::is_empty) {
        return Value::Null;
    }
    json!({
        "program": program,
        "issues": issues,
        "inherited": work.get("inherited").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// Allowlisted `turn_state` subset; unknown upstream fields are dropped.
fn project_turn_state(turn_state: &Map<String, Value>) -> Value {
    let pick = |object: &Map<String, Value>, key: &str| {
        object
            .get(key)
            .filter(|value| value.is_string())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let object = |object: &Map<String, Value>, key: &str| {
        object.get(key).and_then(Value::as_object).cloned()
    };
    let current_turn = object(turn_state, "current_turn")
        .map(|current| {
            json!({
                "last_progress_at": pick(&current, "last_progress_at"),
                "attention": object(&current, "attention")
                    .map(|attention| json!({
                        "kind": pick(&attention, "kind"),
                        "requested_at": pick(&attention, "requested_at"),
                    }))
                    .unwrap_or(Value::Null),
            })
        })
        .unwrap_or(Value::Null);
    let last_turn = object(turn_state, "last_turn")
        .map(|last| {
            let mut projected = json!({ "outcome": pick(&last, "outcome") });
            if let Some(kind @ ("authentication" | "provider_capacity")) =
                last.get("provider_failure_kind").and_then(Value::as_str)
            {
                projected["provider_failure_kind"] = json!(kind);
            }
            projected
        })
        .unwrap_or(Value::Null);
    let source = object(turn_state, "source")
        .map(|source| json!({ "confidence": pick(&source, "confidence") }))
        .unwrap_or(Value::Null);
    json!({
        "phase": pick(turn_state, "phase"),
        "phase_changed_at": pick(turn_state, "phase_changed_at"),
        "current_turn": current_turn,
        "last_turn": last_turn,
        "source": source,
    })
}

/// `(session_id, close_reason)` for every retained closed entry, in order.
#[cfg(test)]
pub(crate) fn closed_reasons_for_test(context: &CliContext) -> Vec<(String, String)> {
    closed(context, None, "test").expect("closed ledger")["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| {
            let field = |key: &str| {
                entry["record"][key]
                    .as_str()
                    .unwrap_or_default()
                    .to_string()
            };
            (field("session_id"), field("close_reason"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const RECORD_KEYS: [&str; 23] = [
        "agent_profile",
        "close_reason",
        "closed_at",
        "created_at",
        "cwd",
        "lineage",
        "machine",
        "messaging_supported",
        "model",
        "provider",
        "reasoning_effort",
        "repo_name",
        "role",
        "runtime_status",
        "session_id",
        "session_incarnation",
        "state",
        "summary",
        "title",
        "title_state",
        "turn_state",
        "updated_at",
        "work",
    ];

    /// A serialized `SessionView` with every excluded field populated.
    fn full_view() -> Value {
        let mut view = json!({
            "id": "20260928-073057-claude",
            "agent": "claude",
            "capabilities": ["managed-account-handoff"],
            "agent_profile": "work",
            "mode": "interactive",
            "coordination_mode": "advisory",
            "title": "Specify the session board",
            "title_state": {
                "topic": "board",
                "topic_source": "auto",
                "references": ["#1"],
                "activity": "Drafting the closed-ledger section"
            },
            "title_state_supported": true,
            "title_revision": 3,
            "retitle_attempt": {"state": "idle"},
            "session_incarnation": "runtime-launch-id",
            "cwd": "/home/user/Project/nils-cli",
            "tmux_session": "agent-secret",
            "status": "running",
            "resumable": true,
            "resume_blocked_reason": "nope",
            "repo_name": "nils-cli",
            "provider_resume": {"provider": "claude", "session_id": "provider-secret"},
            "attach_command": "tmux attach -t agent-secret",
            "ssh_attach_command": "ssh -t host 'tmux attach'",
            "prompt_file": "/home/user/.state/prompt.txt",
            "log_file": "/home/user/.state/log.txt",
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:04:00Z",
            "last_terminal_activity_at": "2030-01-01T00:04:00Z",
            "runtime_started_at": "2030-01-01T00:00:00Z",
            "last_prompt": {"text": "secret prompt"},
            "last_prompt_state": "current",
            "last_prompt_continuity": "token",
            "startup": {"state": "ready"},
            "auto_resume": {"enabled": false},
            "codex_account": {"state": "unsupported"},
            "work_context_state": "claimed",
            "claim_id": "claim",
            "claim_expires_at": "2030-01-01T01:00:00Z",
            "unread_message_count": 4,
            "coordination_conflict_severity": "warning",
            "coordination_available": true,
            "orchestration": {"group": "g"}
        });
        view["turn_state"] = json!({
            "schema_version": "agent-session.turn-state.v1",
            "phase": "working",
            "phase_changed_at": "2030-01-01T00:00:00Z",
            "revision": 7,
            "source": {"kind": "provider", "provider": "claude", "confidence": "authoritative", "future": 1},
            "semantic_event": {"kind": "tool"},
            "diagnostic": {"code": "x"},
            "current_turn": {
                "provider_turn_id": "turn-secret",
                "started_at": "2030-01-01T00:00:00Z",
                "last_progress_at": "2030-01-01T00:04:00Z",
                "attention": {"kind": "approval", "requested_at": "2030-01-01T00:03:00Z", "pending_count": 2, "certainty": "exact"},
                "future": true
            },
            "last_turn": {"provider_turn_id": "t0", "completed_at": "2030-01-01T00:00:00Z", "outcome": "completed", "provider_failure_kind": "x"},
            "future_field": {"leak": true}
        });
        view
    }

    fn keys(value: &Value) -> Vec<&str> {
        let mut keys: Vec<&str> = value
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn projects_exactly_the_allowlisted_fields() {
        let record = project_record(&full_view(), "host-a", Some(Path::new("/home/user")), true)
            .expect("projected");
        assert_eq!(keys(&record), RECORD_KEYS.to_vec());
        assert_eq!(
            record,
            json!({
                "machine": "host-a",
                "session_id": "20260928-073057-claude",
                "session_incarnation": "runtime-launch-id",
                "messaging_supported": true,
                "repo_name": "nils-cli",
                "cwd": "~/Project/nils-cli",
                "provider": "claude",
                "agent_profile": "work",
                "model": null,
                "reasoning_effort": null,
                "title": "Specify the session board",
                "title_state": {"activity": "Drafting the closed-ledger section"},
                "turn_state": {
                    "phase": "working",
                    "phase_changed_at": "2030-01-01T00:00:00Z",
                    "current_turn": {
                        "last_progress_at": "2030-01-01T00:04:00Z",
                        "attention": {"kind": "approval", "requested_at": "2030-01-01T00:03:00Z"}
                    },
                    "last_turn": {"outcome": "completed"},
                    "source": {"confidence": "authoritative"}
                },
                "state": "live",
                "runtime_status": "running",
                "created_at": "2030-01-01T00:00:00Z",
                "updated_at": "2030-01-01T00:04:00Z",
                "closed_at": null,
                "close_reason": null,
                "summary": null,
                "role": null,
                "lineage": null,
                "work": null
            })
        );
    }

    fn session_ref(id: &str) -> Value {
        json!({"machine": "host-a", "session_id": id, "session_created_at": "2030-01-01T00:00:00Z"})
    }

    /// `full_view` of a grandchild adopted by a steward, with work.
    fn lineage_view() -> Value {
        let mut view = full_view();
        view["role"] = json!("coordinator");
        view["lineage"] = json!({
            "schema_version": "agent-session.session-lineage.v1",
            "machine": "host-a",
            "parent": {
                "machine": "host-a",
                "session_id": "parent",
                "session_created_at": "2030-01-01T00:00:00Z",
                "session_incarnation": "parent-launch"
            },
            "root": session_ref("root"),
            "depth": 2,
            "starter": {"kind": "session", "via": "console"},
            "budget": null
        });
        view["work"] = json!({
            "program": {"provider": "github", "repository": "o/laoda", "number": 44},
            "issues": [{"provider": "github", "repository": "o/nils-cli", "number": 2032}],
            "inherited": true,
            "revision": 3
        });
        view
    }

    #[test]
    fn lineage_work_and_role_are_projected_without_incarnations() {
        let record = project_record(&lineage_view(), "host-a", None, false).expect("record");
        assert_eq!(keys(&record), RECORD_KEYS.to_vec());
        assert_eq!(record["role"], "coordinator");
        assert_eq!(
            record["lineage"],
            json!({
                "parent": session_ref("parent"),
                "effective_parent": session_ref("parent"),
                "root": session_ref("root"),
                "depth": 2,
                "starter": {"kind": "session", "via": "console"},
                "budget": null
            })
        );
        assert_eq!(
            record["work"],
            json!({
                "program": {"provider": "github", "repository": "o/laoda", "number": 44},
                "issues": [{"provider": "github", "repository": "o/nils-cli", "number": 2032}],
                "inherited": true
            })
        );
    }

    #[test]
    fn work_cleared_by_work_set_is_null() {
        let mut view = lineage_view();
        view["work"] = json!({"program": null, "issues": [], "inherited": false, "revision": 2});
        let record = project_record(&view, "host-a", None, false).expect("record");
        assert_eq!(record["work"], Value::Null);
    }

    #[test]
    fn the_spec_lineage_example_round_trips() {
        let example = spec_example("A session with lineage and work looks like this");
        let mut view = lineage_view();
        view["work"]["program"] = example["work"]["program"].clone();
        view["work"]["issues"] = example["work"]["issues"].clone();
        let record = project_record(&view, "host-a", None, false).expect("record");
        for key in ["role", "lineage", "work"] {
            assert_eq!(record[key], example[key], "{key}");
        }
    }

    #[test]
    fn the_effective_parent_is_the_adopting_steward() {
        let mut view = lineage_view();
        view["lineage_adoption"] = json!({
            "adopted_by": {
                "machine": "host-b",
                "session_id": "steward",
                "session_created_at": "2030-02-02T00:00:00Z",
                "session_incarnation": "steward-launch"
            },
            "revision": 1,
            "updated_at": "2030-02-02T00:00:00Z"
        });
        let record = project_record(&view, "host-a", None, false).expect("record");
        assert_eq!(record["lineage"]["parent"], session_ref("parent"));
        assert_eq!(
            record["lineage"]["effective_parent"],
            json!({"machine": "host-b", "session_id": "steward", "session_created_at": "2030-02-02T00:00:00Z"})
        );
        // A cleared adoption falls back to the parent.
        view["lineage_adoption"]["adopted_by"] = Value::Null;
        let record = project_record(&view, "host-a", None, false).expect("record");
        assert_eq!(record["lineage"]["effective_parent"], session_ref("parent"));
    }

    #[test]
    fn a_root_session_has_null_parents_and_a_malformed_lineage_is_null() {
        let mut view = lineage_view();
        view["lineage"]["parent"] = Value::Null;
        view["lineage"]["depth"] = json!(0);
        let record = project_record(&view, "host-a", None, false).expect("record");
        assert_eq!(record["lineage"]["parent"], Value::Null);
        assert_eq!(record["lineage"]["effective_parent"], Value::Null);
        assert_eq!(record["lineage"]["depth"], 0);

        for mutate in [
            (|view: &mut Value| view["lineage"]["depth"] = json!("2")) as fn(&mut Value),
            |view| view["lineage"]["root"] = Value::Null,
            |view| view["lineage"]["starter"] = json!("session"),
        ] {
            let mut view = lineage_view();
            mutate(&mut view);
            let record = project_record(&view, "host-a", None, false).expect("record");
            assert_eq!(record["lineage"], Value::Null);
        }
    }

    /// The first JSON example after `heading` in the normative spec.
    fn spec_example(heading: &str) -> Value {
        let spec = include_str!("../docs/specs/session-board-v1.md");
        let section = &spec[spec.find(heading).expect("spec heading")..];
        let start = section.find("```json\n").expect("json example") + "```json\n".len();
        let end = start + section[start..].find("```").expect("example end");
        serde_json::from_str(&section[start..end]).expect("spec example json")
    }

    #[test]
    fn the_spec_record_example_round_trips() {
        let mut view = full_view();
        view["turn_state"]["current_turn"]["attention"] = Value::Null;
        view.as_object_mut().expect("view").remove("agent_profile");
        let record =
            project_record(&view, "host-a", Some(Path::new("/home/user")), true).expect("record");
        assert_eq!(record, spec_example("## Record"));
    }

    #[test]
    fn unavailable_values_are_null_not_omitted() {
        let mut view = full_view();
        let object = view.as_object_mut().expect("view");
        for key in [
            "title_state",
            "turn_state",
            "agent_profile",
            "session_incarnation",
            "reasoning_effort",
            "repo_name",
        ] {
            object.remove(key);
        }
        object.insert("title".into(), Value::Null);
        let record =
            project_record(&view, "host-a", Some(Path::new("/home/user")), true).expect("record");
        assert_eq!(keys(&record), RECORD_KEYS.to_vec());
        for key in [
            "title_state",
            "turn_state",
            "agent_profile",
            "session_incarnation",
            "reasoning_effort",
            "repo_name",
            "title",
        ] {
            assert_eq!(record[key], Value::Null, "{key}");
        }
        // No incarnation, no remote messaging.
        assert_eq!(record["messaging_supported"], false);

        // A turn state without current or last turn keeps those keys as null.
        let mut view = full_view();
        view["turn_state"] = json!({
            "phase": "waiting",
            "phase_changed_at": "2030-01-01T00:00:00Z",
            "source": {"confidence": "observed"}
        });
        view["title_state"]["activity"] = Value::Null;
        let record =
            project_record(&view, "host-a", Some(Path::new("/home/user")), true).expect("record");
        assert_eq!(
            record["turn_state"],
            json!({
                "phase": "waiting",
                "phase_changed_at": "2030-01-01T00:00:00Z",
                "current_turn": null,
                "last_turn": null,
                "source": {"confidence": "observed"}
            })
        );
        assert_eq!(record["title_state"], json!({"activity": null}));
    }

    #[test]
    fn every_runtime_status_maps_to_live_or_stopped() {
        for (status, state) in [
            ("running", "live"),
            ("stopped", "stopped"),
            ("missing", "stopped"),
            ("unknown", "stopped"),
        ] {
            let mut view = full_view();
            view["status"] = json!(status);
            let record = project_record(&view, "host-a", None, true).expect("record");
            assert_eq!(record["state"], state, "{status}");
            assert_eq!(record["runtime_status"], status);
            assert_eq!(
                record["messaging_supported"],
                status == "running",
                "{status}"
            );
        }
    }

    #[test]
    fn messaging_requires_federation_and_available_coordination() {
        let supported = |mutate: &dyn Fn(&mut Value), federation: bool| {
            let mut view = full_view();
            mutate(&mut view);
            project_record(&view, "host-a", None, federation).expect("record")["messaging_supported"]
                .clone()
        };
        assert_eq!(supported(&|_| {}, true), true);
        assert_eq!(supported(&|_| {}, false), false);
        assert_eq!(
            supported(&|view| view["coordination_mode"] = json!("off"), true),
            false
        );
        assert_eq!(
            supported(&|view| view["coordination_available"] = json!(false), true),
            false
        );
    }

    #[test]
    fn cwd_is_home_relative_or_null() {
        let home = Some(Path::new("/home/user"));
        for (cwd, expected) in [
            ("/home/user/Project/x", Some("~/Project/x")),
            ("/home/user", Some("~")),
            ("/home/user/", Some("~")),
            ("/home/username/x", None),
            ("/srv/repo", None),
            ("/home/user/../other", None),
            ("relative/path", None),
        ] {
            assert_eq!(
                home_relative_cwd(cwd, home).as_deref(),
                expected,
                "cwd={cwd}"
            );
        }
        assert_eq!(home_relative_cwd("/home/user/x", None), None);
        assert_eq!(home_relative_cwd("/x", Some(Path::new("/"))), None);
    }

    fn stored_session(context: &CliContext, id: &str) -> crate::SessionRecord {
        let record: crate::SessionRecord = serde_json::from_value(json!({
            "schema_version": crate::SESSION_DOCUMENT_VERSION,
            "id": id,
            "agent": "codex",
            "mode": "interactive",
            "title": "Closing",
            "cwd": "/srv/outside-home",
            "tmux_session": format!("agent-{id}"),
            "prompt_file": null,
            "log_file": null,
            "created_at": "2030-01-01T00:00:00Z",
            "updated_at": "2030-01-01T00:04:00Z",
            "runtime": {
                "kind": "tmux",
                "tmux_session": format!("agent-{id}"),
                "generation": 1,
                "started_at": "2030-01-01T00:00:00Z",
                "launch_id": "live-incarnation"
            }
        }))
        .expect("record");
        std::fs::create_dir_all(crate::session_dir(context, id)).expect("session dir");
        crate::write_session_record(context, &record).expect("write record");
        record
    }

    fn remove(context: &CliContext, record: crate::SessionRecord, reason: CloseReason) {
        let session_dir = crate::session_dir(context, &record.id);
        let fence = crate::SessionRegistryFence::from_record(&record);
        crate::finish_session_delete(context, record, session_dir, fence, reason)
            .expect("remove record");
    }

    #[test]
    fn a_closed_record_never_claims_a_runtime_or_messaging() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let record = stored_session(&context, "closing");
        let closed = closed_record(&context, &record, CloseReason::Archived).expect("closed");
        assert_eq!(closed.get("machine"), None);
        assert_eq!(closed["state"], "closed");
        assert_eq!(closed["runtime_status"], Value::Null);
        assert_eq!(closed["messaging_supported"], false);
        assert_eq!(closed["close_reason"], "archived");
        assert_eq!(closed["session_incarnation"], "live-incarnation");
        assert_eq!(closed["cwd"], Value::Null);
        assert_eq!(closed["updated_at"], "2030-01-01T00:04:00Z");
    }

    #[test]
    fn a_closed_record_keeps_its_lineage_work_and_role() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let mut record = stored_session(&context, "closing-child");
        record.role = Some("coordinator".to_string());
        record.lineage = serde_json::from_value(json!({
            "schema_version": "agent-session.session-lineage.v1",
            "machine": "host-a",
            "parent": session_ref("root"),
            "root": session_ref("root"),
            "depth": 1,
            "starter": {"kind": "session", "via": "cli"},
            "budget": null
        }))
        .expect("lineage");
        record.work = serde_json::from_value(json!({
            "program": null,
            "issues": [{"provider": "github", "repository": "o/r", "number": 1}],
            "inherited": false,
            "revision": 1
        }))
        .expect("work");
        let closed = closed_record(&context, &record, CloseReason::Deleted).expect("closed");
        assert_eq!(closed["role"], "coordinator");
        assert_eq!(closed["lineage"]["root"], session_ref("root"));
        assert_eq!(closed["lineage"]["effective_parent"], session_ref("root"));
        assert_eq!(
            closed["work"],
            json!({
                "program": null,
                "issues": [{"provider": "github", "repository": "o/r", "number": 1}],
                "inherited": false
            })
        );
        // The ledger serves it with the rest; an entry closed before these
        // members existed reads back with nulls.
        record_close(&context, Some(closed));
        let mut preexisting = project_record(&full_view(), "", None, false).expect("preexisting");
        preexisting.as_object_mut().expect("object").remove("role");
        preexisting
            .as_object_mut()
            .expect("object")
            .remove("lineage");
        preexisting.as_object_mut().expect("object").remove("work");
        preexisting["state"] = json!("closed");
        preexisting["session_id"] = json!("preexisting");
        ledger::append(&context, preexisting).expect("append preexisting");
        let read = super::closed(&context, None, "host-a").expect("closed");
        assert_eq!(read["extensions"], json!(EXTENSIONS));
        let entries = read["entries"].as_array().expect("entries");
        assert_eq!(entries[0]["record"]["role"], "coordinator");
        for key in ["role", "lineage", "work"] {
            assert_eq!(entries[1]["record"][key], Value::Null, "{key}");
        }
    }

    #[test]
    fn a_record_removed_after_the_ledger_head_is_read_follows_the_snapshot_cursor() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let tmux = tmp.path().join("tmux-absent");
        let kept = stored_session(&context, "kept");
        drop(kept);
        let mut removed = Some(stored_session(&context, "removed"));
        let snapshot = snapshot_with(&context, &tmux, "host-a", false, &mut || {
            if let Some(record) = removed.take() {
                remove(&context, record, CloseReason::Deleted);
            }
        })
        .expect("snapshot");
        let ids: Vec<&str> = snapshot["records"]
            .as_array()
            .expect("records")
            .iter()
            .map(|record| record["session_id"].as_str().expect("id"))
            .collect();
        assert_eq!(ids, vec!["kept"]);
        let cursor = snapshot["ledger_cursor"].as_str().expect("ledger cursor");
        let closed = closed(&context, Some(cursor), "host-a").expect("closed");
        let entries = closed["entries"].as_array().expect("entries");
        assert_eq!(entries.len(), 1, "{closed}");
        assert_eq!(entries[0]["record"]["session_id"], "removed");
        assert_eq!(entries[0]["record"]["machine"], "host-a");
        assert_eq!(entries[0]["record"]["close_reason"], "deleted");
    }

    #[test]
    fn a_view_missing_required_fields_is_not_projected() {
        for key in ["id", "agent", "status", "created_at", "updated_at"] {
            let mut view = full_view();
            view.as_object_mut().expect("view").remove(key);
            assert_eq!(project_record(&view, "host-a", None, false), None, "{key}");
        }
    }
}

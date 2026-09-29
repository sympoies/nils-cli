//! Session board v1 (`docs/specs/session-board-v1.md`).
//!
//! The board is a separate, opt-in projection of the local session records. It
//! carries only the allowlisted record fields, a home-relative `cwd`, and the
//! serving machine's identity. `serve.rs` only registers the routes and calls
//! into this module.

use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::{CliContext, CliError, SessionView};

pub(crate) const BOARD_SCHEMA: &str = "agent-session.board.v1";
pub(crate) const RECORD_SCHEMA: &str = "agent-session.board-record.v1";
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
        "records": records,
        "skipped_count": skipped_count,
    }))
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
    Some(json!({
        "machine": machine,
        "session_id": session_id,
        "session_incarnation": nullable("session_incarnation"),
        "messaging_supported": messaging_supported,
        "repo_name": nullable("repo_name"),
        "cwd": string("cwd").and_then(|cwd| home_relative_cwd(cwd, home)),
        "provider": provider,
        "agent_profile": nullable("agent_profile"),
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
    }))
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
        .map(|last| json!({ "outcome": pick(&last, "outcome") }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const RECORD_KEYS: [&str; 18] = [
        "agent_profile",
        "close_reason",
        "closed_at",
        "created_at",
        "cwd",
        "machine",
        "messaging_supported",
        "provider",
        "repo_name",
        "runtime_status",
        "session_id",
        "session_incarnation",
        "state",
        "summary",
        "title",
        "title_state",
        "turn_state",
        "updated_at",
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
                "summary": null
            })
        );
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

    #[test]
    fn a_view_missing_required_fields_is_not_projected() {
        for key in ["id", "agent", "status", "created_at", "updated_at"] {
            let mut view = full_view();
            view.as_object_mut().expect("view").remove(key);
            assert_eq!(project_record(&view, "host-a", None, false), None, "{key}");
        }
    }
}

//! Owned child sessions (`session-coordination-v1`, "Owned child sessions").
//!
//! The daemon half answers `POST /sessions/{id}/console-start/v1` for a
//! managed session by asking the aggregator's
//! `POST /api/coordination/sessions/v1`, with the federation relay token and
//! the caller's exact identity, to create a session owned by the caller's
//! Console owner. The CLI half is `agent-session start --via-console`: it
//! reaches that route only through the private daemon endpoint and never
//! reads relay secrets.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::remote::{self, Config};
use crate::lineage::{self, LineageSeed, WorkRequest};
use crate::{CliContext, CliError};

pub(crate) const RESULT_SCHEMA: &str = "agent-session.console-start.v1";
pub(crate) const DISABLED_CODE: &str = "console-start-disabled";
pub(crate) const UNAVAILABLE_CODE: &str = "console-start-unavailable";
const INVALID_CODE: &str = "console-start-invalid";
const MAX_BODY_BYTES: u64 = 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 256;
const MAX_CODE_BYTES: usize = 64;
/// A create that carries a prompt or selects an account can take the
/// aggregator and the target daemon more than a minute.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

fn disabled() -> CliError {
    CliError::runtime(
        DISABLED_CODE,
        "federation is not configured on this daemon",
        None,
    )
}

fn unavailable(message: &str) -> CliError {
    CliError::runtime(UNAVAILABLE_CODE, message, None)
}

fn invalid() -> CliError {
    CliError::usage(
        INVALID_CODE,
        "a console start takes only `machine`, `no_parent`, `work`, `role`, and a `session` object",
        None,
    )
}

/// What the caller asks for the child's lineage and work: by default the
/// caller is the parent and its program and issues are inherited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChildLineage {
    pub(crate) no_parent: bool,
    pub(crate) work: WorkRequest,
    pub(crate) role: Option<String>,
}

impl Default for ChildLineage {
    fn default() -> Self {
        Self {
            no_parent: false,
            role: None,
            work: WorkRequest {
                inherit: true,
                ..WorkRequest::default()
            },
        }
    }
}

fn client() -> Result<reqwest::blocking::Client, CliError> {
    reqwest::blocking::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable("the console start client is unavailable"))
}

/// A bounded, single-line, non-empty diagnostic string, or `None`.
fn bounded_line(value: &Value) -> Option<&str> {
    let text = value.as_str()?.trim();
    (!text.is_empty() && text.len() <= MAX_MESSAGE_BYTES && !text.chars().any(char::is_control))
        .then_some(text)
}

/// A stable failure code shape: lowercase ASCII words joined by `-`.
fn is_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= MAX_CODE_BYTES
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !code.starts_with('-')
}

/// The JSON body of a response, read to at most `MAX_BODY_BYTES`.
fn read_json(response: reqwest::blocking::Response) -> Option<Value> {
    let mut bytes = Vec::new();
    response
        .take(MAX_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_BODY_BYTES {
        return None;
    }
    serde_json::from_slice(&bytes).ok()
}

/// A failure with its own code and message when both have a safe shape,
/// classified by the coordination v1 exit codes.
fn forwarded(code: Option<&str>, message: &Value) -> CliError {
    let code = code
        .filter(|code| is_code(code))
        .unwrap_or(UNAVAILABLE_CODE);
    let message = bounded_line(message).unwrap_or("the console start request failed");
    match code {
        INVALID_CODE
        | "invalid-request"
        | "role-invalid"
        | "role-requires-root"
        | "lineage-invalid"
        | "lineage-parent-mismatch"
        | "work-ref-invalid"
        | "launch-env-key-refused"
        | "launch-env-allowlist-invalid"
        | "launch-env-too-many"
        | "launch-env-value-invalid"
        | "launch-env-invalid"
        | "launch-env-key-duplicate"
        | "launch-env-assignment-invalid" => CliError::usage(code, message, None),
        "coordination-unauthorized"
        | "session-incarnation-conflict"
        | "ownership-unknown"
        | "machine-forbidden" => CliError::data(code, message, None),
        _ => CliError::runtime(code, message, None),
    }
}

/// The request body: optional `machine`, `no_parent`, `work`, and `role`, and a
/// required `session` object.
fn checked_request(body: &Value) -> Option<(Option<&str>, ChildLineage, &Map<String, Value>)> {
    let request = body.as_object()?;
    if request.keys().any(|key| {
        !matches!(
            key.as_str(),
            "machine" | "session" | "no_parent" | "work" | "role"
        )
    }) {
        return None;
    }
    let machine = match request.get("machine") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_str().filter(|machine| !machine.is_empty())?),
    };
    let mut child = ChildLineage::default();
    match request.get("no_parent") {
        None | Some(Value::Null) => {}
        Some(value) => child.no_parent = value.as_bool()?,
    }
    match request.get("work") {
        None | Some(Value::Null) => {}
        Some(value) => child.work = WorkRequest::from_request_json(value).ok()?,
    }
    match request.get("role") {
        None | Some(Value::Null) => {}
        Some(value) => child.role = lineage::role_from_request(Some(value.as_str()?)).ok()?,
    }
    Some((machine, child, request.get("session")?.as_object()?))
}

/// The child's `lineage` and resolved `work`, from the caller's own record on
/// `relay_machine`. The aggregator checks the parent against the caller's
/// identity and forwards both to the target daemon.
fn child_session(
    relay_machine: &str,
    caller: &crate::SessionRecord,
    request: &ChildLineage,
    session: &Map<String, Value>,
) -> Result<Map<String, Value>, CliError> {
    let mut session = session.clone();
    session.remove("lineage");
    session.remove("work");
    session.remove("role");
    let (seed, work) = if request.no_parent {
        let mut seed = LineageSeed::root(
            relay_machine,
            lineage::STARTER_OPERATOR,
            lineage::VIA_CONSOLE,
        );
        seed.set_forge_context(crate::forge_identity::context_of(caller)?.map(|context| {
            nils_common::forge_identity::session::LaunchContext {
                role: None,
                ..context
            }
        }));
        (seed, request.work.resolve(None))
    } else {
        let seed = LineageSeed::child_of(
            relay_machine,
            relay_machine,
            caller,
            lineage::STARTER_SESSION,
            lineage::VIA_CONSOLE,
        )?;
        (seed, request.work.resolve(caller.work.as_ref()))
    };
    session.insert("lineage".to_string(), seed.to_create_json());
    if let Some(work) = work {
        session.insert(
            "work".to_string(),
            json!({ "program": work.program, "issues": work.issues, "inherited": work.inherited }),
        );
    }
    if let Some(role) = &request.role {
        session.insert("role".to_string(), json!(role));
    }
    Ok(session)
}

/// `POST /sessions/{id}/console-start/v1`: authenticate the exact current
/// session incarnation, then ask the aggregator to create the child session.
/// The aggregator decides the owner from the caller's grant; this route has no
/// way to name one. No lock is held across the network call.
pub(crate) fn relay_route(
    context: &CliContext,
    federation: Option<&Config>,
    session: &str,
    token: &str,
    body: &Value,
) -> Result<Value, CliError> {
    let (caller, incarnation) = super::authenticate_token(context, session, token)?;
    let Some(config) = federation else {
        return Err(disabled());
    };
    let (machine, lineage, child) = checked_request(body).ok_or_else(invalid)?;
    lineage::require_root_for_role(lineage.role.as_deref(), !lineage.no_parent)?;
    let child = child_session(&config.machine, &caller, &lineage, child)?;
    let mut request = json!({
        "source_session_id": session,
        "source_incarnation": incarnation,
        "session": child,
    });
    if let Some(machine) = machine {
        request["machine"] = json!(machine);
    }
    let aggregator_unavailable = || unavailable("the console aggregator is unavailable");
    let response = client()?
        .post(format!("{}/api/coordination/sessions/v1", config.url))
        .bearer_auth(&config.token)
        .json(&request)
        .send()
        .map_err(|_| aggregator_unavailable())?;
    let status = response.status();
    let body = read_json(response).ok_or_else(aggregator_unavailable)?;
    if status.is_success() {
        let created = body.pointer("/data/session").filter(|_| body["ok"] == true);
        return match created {
            Some(created) if created["id"].is_string() => Ok(json!({
                "schema_version": RESULT_SCHEMA,
                "machine": machine.unwrap_or(&config.machine),
                "session": created,
            })),
            _ => Err(aggregator_unavailable()),
        };
    }
    if status.as_u16() == 401 {
        return Err(unavailable(
            "the console aggregator rejected this daemon's relay credential",
        ));
    }
    let error = &body["error"];
    Err(forwarded(error["code"].as_str(), &error["message"]))
}

/// `agent-session start --via-console`: the child session through the local
/// daemon, as the managed session this CLI runs in.
pub(crate) fn cli_start(
    context: &CliContext,
    capability_file: Option<&Path>,
    machine: Option<&str>,
    session: Value,
    lineage: &ChildLineage,
) -> Result<Value, CliError> {
    let caller = crate::non_empty_env("AGENT_SESSION_ID").ok_or_else(|| {
        CliError::usage(
            "console-start-unmanaged",
            "--via-console runs only inside a managed session (AGENT_SESSION_ID is unset)",
            None,
        )
    })?;
    let token = super::capability_token_from_file(capability_file)?;
    super::authenticate_token(context, &caller, &token)?;
    let unreachable = || unavailable("the local agent-session daemon is unreachable");
    let mut url = remote::daemon_url(context).map_err(|_| unreachable())?;
    url.path_segments_mut()
        .map_err(|_| unreachable())?
        .pop_if_empty()
        .extend(["sessions", caller.as_str(), "console-start", "v1"]);
    let requested_pinned = session["title_mode"] == "pinned";
    let requested_launch_env = crate::launch_env::from_json(session.get("launch_env"))?;
    let mut request = json!({ "session": session });
    if let Some(machine) = machine {
        request["machine"] = json!(machine);
    }
    if lineage.no_parent {
        request["no_parent"] = json!(true);
    }
    if let Some(work) = lineage.work.to_request_json() {
        request["work"] = work;
    }
    if let Some(role) = &lineage.role {
        request["role"] = json!(role);
    }
    let response = client()?
        .post(url)
        .bearer_auth(&token)
        .json(&request)
        .send()
        .map_err(|_| unreachable())?;
    let status = response.status();
    let body = read_json(response)
        .ok_or_else(|| unavailable("the local daemon's console start answer is unreadable"))?;
    if status.is_success() {
        if !requested_launch_env.is_empty()
            && body["session"]["id"].is_string()
            && body["session"]["launch_env"] != json!(requested_launch_env)
        {
            return Err(CliError::runtime(
                "launch-env-unconfirmed",
                "the created session did not confirm its launch env; inspect it before retrying",
                Some(json!({"created_session_id":body["session"]["id"],"safe_to_retry":false})),
            ));
        }
        if requested_pinned
            && body["session"]["id"].is_string()
            && (body["session"]["title_mode"] != "pinned"
                || body["session"]["display_revision"].as_u64().is_none())
        {
            return Err(CliError::runtime(
                "title-mode-unconfirmed",
                "the created session did not confirm pinned title mode; inspect it before retrying",
                Some(json!({"created_session_id":body["session"]["id"],"safe_to_retry":false})),
            ));
        }
        return (body["schema_version"] == RESULT_SCHEMA && body["session"]["id"].is_string())
            .then_some(body)
            .ok_or_else(|| unavailable("the console start answered with an unsupported result"));
    }
    let error = &body["error"];
    Err(forwarded(error["code"].as_str(), &error["message"]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn requests_admit_only_machine_and_a_session_object() {
        let session = json!({"agent": "claude"});
        let with_machine = json!({"machine": "c8", "session": session});
        let (machine, lineage, child) = checked_request(&with_machine).expect("with machine");
        assert_eq!(
            (machine, lineage, child),
            (
                Some("c8"),
                ChildLineage::default(),
                session.as_object().unwrap()
            )
        );
        let default = json!({"session": session});
        let (machine, _, _) = checked_request(&default).expect("default");
        assert_eq!(machine, None);
        let root = json!({
            "session": session,
            "no_parent": true,
            "work": {"issues": [{"provider": "github", "repository": "a/b", "number": 1}], "inherit": false}
        });
        let (_, lineage, _) = checked_request(&root).expect("root");
        assert_eq!(
            lineage,
            ChildLineage {
                no_parent: true,
                role: None,
                work: WorkRequest::from_flags(None, &["a/b#1".to_string()], true).unwrap(),
            }
        );
        for body in [
            json!({"session": session, "principal": "other"}),
            json!({"machine": 7, "session": session}),
            json!({"machine": "", "session": session}),
            json!({"machine": "c8"}),
            json!({"session": "claude"}),
            json!({"session": session, "no_parent": "yes"}),
            json!({"session": session, "role": "invalid role"}),
            json!({"session": session, "role": 7}),
            json!({"session": session, "work": {"program": "a/b#1"}}),
            json!([]),
        ] {
            assert!(checked_request(&body).is_none(), "{body}");
        }
    }

    #[test]
    fn launch_env_forwarded_validation_preserves_usage_errors() {
        for code in [
            "launch-env-key-refused",
            "launch-env-allowlist-invalid",
            "launch-env-too-many",
            "launch-env-value-invalid",
            "launch-env-invalid",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!(error.code, code);
            assert_eq!(error.exit_code, 64, "{code}");
        }
    }

    #[test]
    fn forwarded_failures_keep_only_safe_shapes() {
        for code in [
            "ownership-unknown",
            "machine-forbidden",
            "session-incarnation-conflict",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!(
                (error.code.as_str(), error.message.as_str(), error.exit_code),
                (code, "refused", 65),
                "{code}"
            );
        }
        for code in [
            "role-invalid",
            "role-requires-root",
            "lineage-invalid",
            "lineage-parent-mismatch",
            "work-ref-invalid",
        ] {
            let error = forwarded(Some(code), &json!("refused")).into_inner();
            assert_eq!((error.code.as_str(), error.exit_code), (code, 64), "{code}");
        }
        let error = forwarded(Some("invalid-request"), &json!(null)).into_inner();
        assert_eq!(
            (error.code.as_str(), error.message.as_str(), error.exit_code),
            ("invalid-request", "the console start request failed", 64)
        );
        let error = forwarded(Some("agent-profile-unavailable"), &json!("no profile")).into_inner();
        assert_eq!(
            (error.code.as_str(), error.exit_code),
            ("agent-profile-unavailable", 1)
        );
        for code in [
            None,
            Some(""),
            Some("Bad"),
            Some("-lead"),
            Some("has space"),
        ] {
            let error = forwarded(code, &json!("line\u{1b}[2J")).into_inner();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (UNAVAILABLE_CODE, "the console start request failed"),
                "{code:?}"
            );
        }
    }
}

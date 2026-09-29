//! Board relay (`session-board-v1`, "Relay route" and "Mode selection").
//!
//! The daemon half answers `GET /sessions/{id}/board/v1` for a managed
//! session by forwarding the board query to the aggregator's
//! `GET /api/coordination/board/v1` with the federation relay token, over the
//! same bounded client as peer discovery. The CLI half is relay mode of
//! `agent-session board`: it reaches that route only through the private
//! daemon endpoint, as federated messaging does, and never reads relay
//! secrets.

use std::io::Read;

use serde_json::Value;

use crate::coordination::remote::{self, Config};
use crate::{CliContext, CliError};

/// The query parameters the route forwards unchanged.
const FILTERS: [&str; 4] = ["state", "since", "repo", "machine"];
/// At most 1024 records per view, with headroom for long titles.
const MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 256;
const MAX_CODE_BYTES: usize = 64;

const RELAY_DISABLED_CODE: &str = "board-relay-disabled";
const RELAY_UNAVAILABLE_CODE: &str = "board-relay-unavailable";
const QUERY_INVALID_CODE: &str = "board-query-invalid";

fn relay_disabled() -> CliError {
    CliError::runtime(
        RELAY_DISABLED_CODE,
        "federation is not configured on this daemon",
        None,
    )
}

fn relay_unavailable(message: &str) -> CliError {
    CliError::runtime(RELAY_UNAVAILABLE_CODE, message, None)
}

fn aggregator_unavailable() -> CliError {
    relay_unavailable("the board aggregator is unavailable")
}

fn relay_unauthorized() -> CliError {
    CliError::runtime(
        "board-relay-unauthorized",
        "the board aggregator rejected this daemon's relay credential",
        None,
    )
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

/// Only a supported `agent-session.board-view.v1` object is accepted.
fn checked_view(value: Value) -> Option<Value> {
    (value["schema_version"] == super::view::VIEW_SCHEMA
        && value["machines"].is_array()
        && value["records"].is_array())
    .then_some(value)
}

/// The four filters from the route's raw query. An unknown or repeated
/// parameter is `board-query-invalid`; values are the aggregator's to judge.
fn parse_filters(raw_query: Option<&str>) -> Result<Vec<(String, String)>, CliError> {
    let invalid = || {
        super::view::query_invalid("the board relay accepts only state, since, repo, and machine")
    };
    let mut url = reqwest::Url::parse("http://board.invalid/").map_err(|_| invalid())?;
    url.set_query(raw_query);
    let mut filters: Vec<(String, String)> = Vec::new();
    for (key, value) in url.query_pairs() {
        if !FILTERS.contains(&key.as_ref()) || filters.iter().any(|(seen, _)| *seen == key) {
            return Err(invalid());
        }
        filters.push((key.into_owned(), value.into_owned()));
    }
    Ok(filters)
}

/// `GET /sessions/{id}/board/v1`: authenticate the exact current session
/// incarnation, then forward the query to the aggregator. No lock is held
/// across the network call.
pub(crate) fn relay_route(
    context: &CliContext,
    federation: Option<&Config>,
    session: &str,
    token: &str,
    raw_query: Option<&str>,
) -> Result<Value, CliError> {
    let (_, incarnation) = crate::coordination::authenticate_token(context, session, token)?;
    let Some(config) = federation else {
        return Err(relay_disabled());
    };
    let filters = parse_filters(raw_query)?;
    let mut url = reqwest::Url::parse(&format!("{}/api/coordination/board/v1", config.url))
        .map_err(|_| aggregator_unavailable())?;
    url.query_pairs_mut()
        .extend_pairs(&filters)
        .append_pair("source_session_id", session)
        .append_pair("source_incarnation", &incarnation);
    let response = remote::client()
        .map_err(|_| aggregator_unavailable())?
        .get(url)
        .bearer_auth(&config.token)
        .send()
        .map_err(|_| aggregator_unavailable())?;
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(relay_unauthorized());
    }
    let body = read_json(response).ok_or_else(aggregator_unavailable)?;
    if status.is_success() {
        return checked_view(body).ok_or_else(aggregator_unavailable);
    }
    if status.as_u16() == 400
        && body.pointer("/error/code") == Some(&Value::from(QUERY_INVALID_CODE))
    {
        let message = bounded_line(&body["error"]["message"])
            .unwrap_or("the aggregator rejected the board query");
        return Err(super::view::query_invalid(message));
    }
    Err(aggregator_unavailable())
}

/// The managed session this CLI runs in, once its `AGENT_SESSION_ID` and
/// capability verify against the exact current incarnation.
pub(super) struct Caller {
    pub(super) session_id: String,
    pub(super) incarnation: String,
    token: String,
}

/// The managed session this CLI claims to run in: `AGENT_SESSION_ID` with a
/// capability file. Without one there is no managed identity.
pub(super) fn claimed_session() -> Option<String> {
    crate::non_empty_env(crate::coordination::CAPABILITY_ENV)?;
    crate::non_empty_env("AGENT_SESSION_ID")
}

/// Whether the daemon published its endpoint; without one there is no relay.
pub(super) fn endpoint_present(context: &CliContext) -> bool {
    let endpoint = context.state_dir.join("coordination/daemon-endpoint.json");
    !matches!(
        std::fs::symlink_metadata(endpoint),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound
    )
}

/// Verify the claimed session against its exact current incarnation.
pub(super) fn authenticate(context: &CliContext, session_id: &str) -> Result<Caller, CliError> {
    let token = crate::coordination::capability_token_from_file(None)?;
    let (_, incarnation) = crate::coordination::authenticate_token(context, session_id, &token)?;
    Ok(Caller {
        session_id: session_id.to_string(),
        incarnation,
        token,
    })
}

/// Relay mode: the aggregator view through the local daemon. `Ok(None)`
/// selects local mode, which happens only when the daemon answers
/// `board-disabled` or `board-relay-disabled`. Every other failure, an
/// unreachable daemon included, is returned: a local view would present one
/// machine as the whole deployment.
pub(super) fn fetch(
    context: &CliContext,
    caller: &Caller,
    filters: &[(&str, &str)],
) -> Result<Option<Value>, CliError> {
    let unreachable = || relay_unavailable("the local agent-session daemon is unreachable");
    let mut url = remote::daemon_url(context).map_err(|_| unreachable())?;
    url.path_segments_mut()
        .map_err(|_| unreachable())?
        .pop_if_empty()
        .extend(["sessions", caller.session_id.as_str(), "board", "v1"]);
    url.query_pairs_mut().extend_pairs(filters);
    let response = remote::client()
        .map_err(|_| unreachable())?
        .get(url)
        .bearer_auth(&caller.token)
        .send()
        .map_err(|_| unreachable())?;
    let status = response.status();
    let body = read_json(response)
        .ok_or_else(|| relay_unavailable("the local daemon's board relay answer is unreadable"))?;
    if status.is_success() {
        return checked_view(body)
            .map(Some)
            .ok_or_else(|| relay_unavailable("the board relay answered with an unsupported view"));
    }
    let code = body
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if code == super::DISABLED_CODE || code == RELAY_DISABLED_CODE {
        return Ok(None);
    }
    Err(forwarded(code, &body["error"]["message"]))
}

/// The daemon's failure, forwarded with its own code and message when both
/// have a safe shape, classified by the coordination v1 exit codes.
fn forwarded(code: &str, message: &Value) -> CliError {
    let code = if is_code(code) {
        code
    } else {
        RELAY_UNAVAILABLE_CODE
    };
    let message = bounded_line(message).unwrap_or("the board relay request failed");
    match code {
        QUERY_INVALID_CODE => CliError::usage(code, message, None),
        "coordination-unauthorized" | "session-incarnation-conflict" => {
            CliError::data(code, message, None)
        }
        _ => CliError::runtime(code, message, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    #[test]
    fn filters_admit_each_known_parameter_once() {
        let filters = parse_filters(Some("state=live&since=3d&repo=a%20b&machine=m")).expect("ok");
        assert_eq!(
            filters,
            [
                ("state", "live"),
                ("since", "3d"),
                ("repo", "a b"),
                ("machine", "m")
            ]
            .map(|(key, value)| (key.to_string(), value.to_string()))
        );
        assert_eq!(parse_filters(None).expect("empty"), Vec::new());
        for raw in ["limit=1", "state=live&state=all", "source_incarnation=x"] {
            let error = parse_filters(Some(raw)).expect_err(raw).into_inner();
            assert_eq!(error.code, "board-query-invalid", "{raw}");
        }
    }

    #[test]
    fn forwarded_failures_keep_only_safe_shapes() {
        let error = forwarded("board-query-invalid", &json!("bad since")).into_inner();
        assert_eq!(
            (error.code.as_str(), error.message.as_str(), error.exit_code),
            ("board-query-invalid", "bad since", 64)
        );
        let error = forwarded("coordination-unauthorized", &json!(null)).into_inner();
        assert_eq!(
            (error.code.as_str(), error.message.as_str(), error.exit_code),
            (
                "coordination-unauthorized",
                "the board relay request failed",
                65
            )
        );
        for code in ["", "Bad", "-lead", "has space", &"x".repeat(65)] {
            let error = forwarded(code, &json!("line\u{1b}[2J")).into_inner();
            assert_eq!(
                (error.code.as_str(), error.message.as_str(), error.exit_code),
                (
                    "board-relay-unavailable",
                    "the board relay request failed",
                    1
                ),
                "{code}"
            );
        }
    }
}

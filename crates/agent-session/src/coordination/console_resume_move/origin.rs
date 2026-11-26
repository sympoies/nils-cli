//! Origin discovery, first dispatch, and reconciliation through the relay's fence.
use std::time::Duration;

use reqwest::{StatusCode, blocking::Client};

use crate::CliContext;

use super::{
    super::{console_start, remote},
    refusal::Failure,
    reply::Reply,
    store::{Retained, Store},
    wire::{StartRequest, has_capability},
};

const RELAY_ROUTE: &str = "/api/coordination/resume-move/v1";
pub(super) const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) fn start(
    context: &CliContext,
    config: Option<&remote::Config>,
    request: &StartRequest,
    body: &str,
) -> Reply {
    let move_id = &request.session.move_id;
    let observe = || Store::open(context).and_then(|mut store| store.existing(request));
    match observe() {
        Ok(Some(Retained::Terminal(reply))) => return reply,
        Err(error) => return error.reply(move_id),
        _ => {}
    }
    let relay = config.and_then(|config| {
        let client = console_start::client().ok()?;
        let url = format!("{}{RELAY_ROUTE}", config.url);
        let response = client
            .get(&url)
            .bearer_auth(&config.token)
            .timeout(DISCOVERY_TIMEOUT)
            .send()
            .ok()?;
        has_capability(response, &config.machine).then_some((client, url, config.token.as_str()))
    });
    let Some((client, url, token)) = relay else {
        return match observe() {
            Ok(Some(Retained::Terminal(reply))) => reply,
            Ok(Some(Retained::Unresolved(_))) => Reply::unknown(move_id),
            Ok(None) => Reply::failure(
                Failure::before_start(StatusCode::SERVICE_UNAVAILABLE, "resume-move-unavailable"),
                Some(move_id),
            ),
            Err(error) => error.reply(move_id),
        };
    };

    let reply = match Store::open(context).and_then(|store| store.claim(request, body)) {
        Ok(None) => post(&client, &url, token, body.to_owned(), request),
        Ok(Some(Retained::Unresolved(retained))) => {
            reconcile(&client, &url, token, retained, request)
        }
        Ok(Some(Retained::Terminal(reply))) => return reply,
        Err(error) => return error.reply(move_id),
    };
    Store::open(context)
        .and_then(|store| store.record(request, reply))
        .unwrap_or_else(|_| Reply::unknown(move_id))
}

fn post(client: &Client, url: &str, token: &str, body: String, request: &StartRequest) -> Reply {
    client
        .post(url)
        .bearer_auth(token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .ok()
        .and_then(|response| Reply::from_response(response, request))
        .unwrap_or_else(|| Reply::unknown(&request.session.move_id))
}

fn reconcile(
    client: &Client,
    url: &str,
    token: &str,
    retained: String,
    request: &StartRequest,
) -> Reply {
    // A repeat uses the original body, including operation ID and caller
    // incarnation. The relay's durable fence makes this a status-only read.
    post(client, url, token, retained, request)
}

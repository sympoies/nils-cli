//! One-shot companion inbox submissions through the local daemon and its relay.

use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use reqwest::Url;
use reqwest::blocking::ClientBuilder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{authenticate_any_token, capability_token_from_file, remote};
use crate::cli::MessageSendCompanionArgs;
use crate::{CliContext, CliError, render_error, render_single_success};

pub(crate) const COMMAND: &str = "message-send-companion";
pub(crate) const ROUTE: &str = "/coordination/companion/messages/v1";
pub(crate) const MAX_REQUEST_BYTES: usize = 32 * 1024;
const RELAY_ROUTE: &str = "/api/coordination/prompter/v1/messages";
const MAX_BODY_BYTES: usize = 4_096;
const MAX_REPLY_BYTES: usize = 8 * 1024;
const LOCAL_TIMEOUT: Duration = Duration::from_secs(25);

const INPUT_INVALID: &str = "companion-input-invalid";
const SOURCE_FORBIDDEN: &str = "companion-source-forbidden";
const BODY_TOO_LARGE: &str = "companion-body-too-large";
const INBOX_FULL: &str = "companion-inbox-full";
const RELAY_UNAVAILABLE: &str = "companion-relay-unavailable";
const SUBMIT_AMBIGUOUS: &str = "companion-submit-ambiguous";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submission {
    body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    urgent: Option<bool>,
}

impl Submission {
    fn validate(&self) -> Result<(), Failure> {
        if self.body.len() > MAX_BODY_BYTES {
            return Err(Failure::TooLarge);
        }
        if self.body.trim().is_empty()
            || self.color.as_deref().is_some_and(|color| {
                !color.strip_prefix('#').is_some_and(|rgb| {
                    rgb.len() == 6 && rgb.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            })
        {
            return Err(Failure::Invalid);
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct RelaySubmission<'a> {
    source_session_id: &'a str,
    source_incarnation: &'a str,
    #[serde(flatten)]
    message: Submission,
}

#[derive(Serialize)]
pub(crate) struct Receipt {
    id: String,
}

pub(crate) enum Failure {
    Invalid,
    Forbidden,
    TooLarge,
    Full,
    Unavailable,
    Ambiguous,
}

impl From<Failure> for CliError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::Invalid => CliError::usage(
                INPUT_INVALID,
                "provide a readable UTF-8 body file with non-whitespace text and use #RRGGBB for color",
                None,
            ),
            Failure::Forbidden => CliError::data(
                SOURCE_FORBIDDEN,
                "the current session is not authorized to submit a companion message",
                None,
            ),
            Failure::TooLarge => CliError::data(
                BODY_TOO_LARGE,
                "the companion submission exceeds a byte limit (4,096 UTF-8 bytes for the body)",
                None,
            ),
            Failure::Full => CliError::data(INBOX_FULL, "the companion inbox is full", None),
            Failure::Unavailable => CliError::unavailable(
                RELAY_UNAVAILABLE,
                "the daemon or companion relay is unavailable for submission",
                None,
            ),
            Failure::Ambiguous => CliError::runtime(
                SUBMIT_AMBIGUOUS,
                "submission outcome is unknown. Check the companion inbox before submitting again.",
                None,
            ),
        }
    }
}

pub(super) fn run(context: &CliContext, args: MessageSendCompanionArgs) -> i32 {
    let format = args.format;
    match submit_local(context, args) {
        Ok(receipt) => render_single_success(COMMAND, format, &receipt, |receipt| {
            format!("{}\n", receipt.id)
        }),
        Err(failure) => render_error(COMMAND, format, failure.into()),
    }
}

fn submit_local(context: &CliContext, args: MessageSendCompanionArgs) -> Result<Receipt, Failure> {
    let message = Submission {
        body: read_body(&args.body_file)?,
        color: args.color,
        urgent: args.urgent.then_some(true),
    };
    message.validate()?;
    let capability = capability_token_from_file(None).map_err(|_| Failure::Forbidden)?;
    let base = remote::daemon_url(context).map_err(|_| Failure::Unavailable)?;
    let url = base.join(ROUTE).map_err(|_| Failure::Unavailable)?;
    let client = remote::client_builder().timeout(LOCAL_TIMEOUT).no_proxy();
    let (status, response) = post(client, url, &capability, &message)?;
    match (status, error_code(&response)) {
        (400, Some(INPUT_INVALID)) => Err(Failure::Invalid),
        (422, Some(SOURCE_FORBIDDEN)) => Err(Failure::Forbidden),
        (422, Some(BODY_TOO_LARGE)) => Err(Failure::TooLarge),
        (422, Some(INBOX_FULL)) => Err(Failure::Full),
        (503, Some(RELAY_UNAVAILABLE)) => Err(Failure::Unavailable),
        _ => receipt(status, &response),
    }
}

fn read_body(path: &Path) -> Result<String, Failure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| Failure::Invalid)?;
    if !file.metadata().map_err(|_| Failure::Invalid)?.is_file() {
        return Err(Failure::Invalid);
    }
    let mut bytes = Vec::new();
    file.take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Invalid)?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(Failure::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| Failure::Invalid)
}

pub(crate) fn submit_relay(
    context: &CliContext,
    config: Option<&remote::Config>,
    capability: &str,
    message: Submission,
) -> Result<Receipt, Failure> {
    message.validate()?;
    let (record, incarnation) =
        authenticate_any_token(context, capability).map_err(|error| match error.code() {
            "coordination-unauthorized" | "session-incarnation-conflict" => Failure::Forbidden,
            _ => Failure::Unavailable,
        })?;
    let config = config.ok_or(Failure::Unavailable)?;
    let url =
        Url::parse(&format!("{}{RELAY_ROUTE}", config.url)).map_err(|_| Failure::Unavailable)?;
    let request = RelaySubmission {
        source_session_id: &record.id,
        source_incarnation: &incarnation,
        message,
    };
    let (status, response) = post(remote::client_builder(), url, &config.token, &request)?;
    match (status, error_code(&response)) {
        (400, Some("invalid-request")) => Err(Failure::Invalid),
        (401, Some("unauthorized")) | (503, Some("source-unavailable")) => {
            Err(Failure::Unavailable)
        }
        (403, Some("source-forbidden")) | (409, Some("session-incarnation-conflict")) => {
            Err(Failure::Forbidden)
        }
        (413, Some("body-too-large")) => Err(Failure::TooLarge),
        (429, Some("inbox-full")) => Err(Failure::Full),
        _ => receipt(status, &response),
    }
}

fn post(
    builder: ClientBuilder,
    url: Url,
    bearer: &str,
    payload: &impl Serialize,
) -> Result<(u16, Value), Failure> {
    let client = builder
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| Failure::Unavailable)?;
    let request = client
        .post(url)
        .bearer_auth(bearer)
        .json(payload)
        .build()
        .map_err(|_| Failure::Unavailable)?;
    // From execution onward an error can mean the write succeeded and the receipt was lost.
    let response = client.execute(request).map_err(|_| Failure::Ambiguous)?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    response
        .take((MAX_REPLY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::Ambiguous)?;
    if bytes.len() > MAX_REPLY_BYTES {
        return Err(Failure::Ambiguous);
    }
    let body = serde_json::from_slice(&bytes).map_err(|_| Failure::Ambiguous)?;
    Ok((status, body))
}

fn error_code(response: &Value) -> Option<&str> {
    if response["ok"] == false {
        response.pointer("/error/code")?.as_str()
    } else {
        None
    }
}

fn receipt(status: u16, response: &Value) -> Result<Receipt, Failure> {
    if status != 201 || response["ok"] != true {
        return Err(Failure::Ambiguous);
    }
    let id = response
        .pointer("/data/id")
        .and_then(Value::as_str)
        .filter(|id| {
            id.len() <= 20
                && id.bytes().all(|byte| byte.is_ascii_digit())
                && id.parse::<u64>().is_ok()
        })
        .ok_or(Failure::Ambiguous)?;
    Ok(Receipt { id: id.to_owned() })
}

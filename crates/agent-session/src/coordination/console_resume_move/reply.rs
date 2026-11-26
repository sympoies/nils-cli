//! Validate each reply before forwarding it, retaining it or presenting it to the caller.
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::CliError;
use crate::cli::resume_move::{canonical_uuid, destination::Outcome};

use super::{
    refusal::{Failure, allowlisted},
    wire::{self, START_SCHEMA, StartRequest, Success, present},
};

#[derive(Deserialize)]
#[serde(untagged)]
enum Envelope {
    Created(Success<Created>),
    Refused(Refused),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Created {
    machine: String,
    move_id: String,
    state: Outcome,
    session_id: String,
    session_incarnation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refused {
    schema_version: String,
    ok: bool,
    error: Refusal,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Refusal {
    code: String,
    details: Details,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Details {
    move_id: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    state: Option<Outcome>,
    safe_to_retry: bool,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    session_created: Option<bool>,
    next_action: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    status: u16,
    body: Value,
}

impl Reply {
    pub(crate) fn failure(error: Failure, move_id: Option<&str>) -> Self {
        let (status, body) = error.envelope(START_SCHEMA, move_id);
        Self {
            status: status.as_u16(),
            body,
        }
    }

    pub(super) fn unknown(move_id: &str) -> Self {
        Self::failure(Failure::unknown(None), Some(move_id))
    }

    pub(super) fn from_response(
        response: reqwest::blocking::Response,
        request: &StartRequest,
    ) -> Option<Self> {
        let status = response.status();
        Self::checked(status, wire::read_response(response)?, request)
    }

    pub(super) fn checked_retained(&self, request: &StartRequest) -> Option<Self> {
        let status = StatusCode::from_u16(self.status).ok()?;
        Self::checked(
            status,
            serde_json::from_value(self.body.clone()).ok()?,
            request,
        )
        .filter(Self::terminal)
    }

    fn checked(status: StatusCode, envelope: Envelope, request: &StartRequest) -> Option<Self> {
        let move_id = &request.session.move_id;
        let body = match envelope {
            Envelope::Created(reply)
                if status == StatusCode::OK
                    && reply.schema_version == START_SCHEMA
                    && reply.ok
                    && reply.data.machine == request.machine
                    && reply.data.move_id == *move_id
                    && reply.data.state == Outcome::Created
                    && reply.data.session_id == format!("move-{move_id}")
                    && canonical_uuid(&reply.data.session_incarnation) =>
            {
                json!({"schema_version":START_SCHEMA,"ok":true,"data":reply.data})
            }
            Envelope::Refused(reply)
                if (status.is_client_error() || status.is_server_error())
                    && reply.schema_version == START_SCHEMA
                    && !reply.ok =>
            {
                let (code, action) = allowlisted(&reply.error.code)?;
                let details = reply.error.details;
                let expected_action = match details.session_created {
                    Some(false) if code != "resume-move-outcome-unknown" => action,
                    None => "reconcile_destination",
                    _ => return None,
                };
                if details.move_id != *move_id
                    || details.safe_to_retry
                    || details.next_action != expected_action
                {
                    return None;
                }
                json!({"schema_version":START_SCHEMA,"ok":false,"error":{"code":code,"details":details}})
            }
            _ => return None,
        };
        Some(Self {
            status: status.as_u16(),
            body,
        })
    }

    pub(super) fn is_created(&self) -> bool {
        self.body["ok"] == true
    }

    pub(super) fn terminal(&self) -> bool {
        self.is_created() || self.body["error"]["details"]["session_created"] == false
    }

    pub(crate) fn into_parts(self) -> (StatusCode, Value) {
        (
            StatusCode::from_u16(self.status).expect("reply status was validated"),
            self.body,
        )
    }

    pub(super) fn into_cli(self) -> Result<Value, CliError> {
        let (_, mut body) = self.into_parts();
        if body["ok"] == true {
            return Ok(body["data"].take());
        }
        let error = &mut body["error"];
        let code = error["code"].take();
        let code = code.as_str().expect("reply code was validated");
        let details = error["details"].take();
        let message = match code {
            "resume-move-unavailable" => "Console resume-move capability or routing is unavailable",
            "resume-move-outcome-unknown" => {
                "the move outcome is unresolved; reconcile with the destination"
            }
            "resume-move-state-unavailable" => {
                "the Console resume-move receipt store is unavailable"
            }
            "resume-move-binding-mismatch" => {
                "the request does not match the authenticated caller or retained move"
            }
            "resume-unsupported" => "this Console resume-move request is not supported",
            _ => "the destination refused the Console resume-move request",
        };
        Err(CliError::data(code, message, Some(details)))
    }
}

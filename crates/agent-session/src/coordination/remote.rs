//! Daemon-owned federation. Session credentials never cross the machine boundary.
use std::path::Path;
use std::time::Duration;

use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    authenticate_from_file, authenticate_token, digest_bytes, incarnation, lock_registry,
    now_epoch, timestamp,
};
use crate::{CliContext, CliError};

const JOURNAL_VERSION: &str = "agent-session.federation-journal.v2";
const EXTENDED_JOURNAL_VERSION: &str = "agent-session.federation-journal.v3";
const AUDIT_JOURNAL_VERSION: &str = "agent-session.federation-journal.v4";
const LEGACY_JOURNAL_VERSION: &str = "agent-session.federation-journal.v1";
const JOURNAL_FILE: &str = "federation-journal.json";
const MAX_JOURNAL_BYTES: u64 = 32 * 1024 * 1024;
/// Submit leaves this much room so drain's retry bookkeeping always fits.
const JOURNAL_HEADROOM_BYTES: u64 = 256 * 1024;
const RECEIVE_PRINCIPAL: &str = "remote:receive";
const RECEIVE_INCARNATION: &str = "v1";
const RECEIVE_OPERATION: &str = "remote-message-receive";
const ENVELOPE_VERSION: &str = "agent-session.remote-message.v1";
const ENVELOPE_V2: &str = "agent-session.remote-message.v2";
const SERVICE_ENVELOPE_V2: &str = "agent-session.remote-service-message.v2";
const DELIVERY_VERSION: &str = "agent-session.remote-delivery.v1";
/// Queued (undelivered) envelopes for one destination machine. Each carries its
/// body, so an asleep or offline destination is bounded without blocking others.
const MAX_PENDING_PER_DESTINATION: usize = 512;
/// Queued envelopes across all destinations.
const MAX_PENDING: usize = 2048;
/// Every retained source identity: queued envelopes plus body-free terminal records.
const MAX_RETAINED_IDS: usize = 16384;
/// Retry interval; an unavailable destination backs off its attempted envelopes together.
const RETRY_SECS: i64 = 5;

#[derive(Clone)]
pub(crate) struct Config {
    pub machine: String,
    pub url: String,
    pub token: String,
    pub ingress_token: String,
}
impl Config {
    pub fn from_environment(machine: &str) -> Result<Option<Self>, CliError> {
        let values = [
            "AGENT_SESSION_RELAY_URL",
            "AGENT_SESSION_RELAY_TOKEN",
            "AGENT_SESSION_RELAY_INGRESS_TOKEN",
        ]
        .map(crate::non_empty_env);
        if values.iter().all(Option::is_none) {
            return Ok(None);
        }
        let [Some(url), Some(token), Some(ingress_token)] = values else {
            return Err(unavailable());
        };
        let parsed = reqwest::Url::parse(&url).map_err(|_| unavailable())?;
        if !matches!(parsed.scheme(), "https" | "http")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || token.len() < 32
            || ingress_token.len() < 32
            || token == ingress_token
        {
            return Err(unavailable());
        }
        if parsed.scheme() == "http"
            && !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        {
            return Err(unavailable());
        }
        Ok(Some(Self {
            machine: machine.into(),
            url: url.trim_end_matches('/').into(),
            token,
            ingress_token,
        }))
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Address {
    pub machine: String,
    pub session_id: String,
    pub session_incarnation: String,
}
/// Both variants reject unknown fields, so mixed session/service identities cannot parse.
/// The envelope schema must also match the variant before ingress admission.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum Origin {
    Session(Address),
    Service(super::service::Origin),
}
impl From<Address> for Origin {
    fn from(value: Address) -> Self {
        Self::Session(value)
    }
}
impl Origin {
    fn generation(&self) -> &str {
        match self {
            Self::Session(a) => &a.session_incarnation,
            Self::Service(a) => &a.service_generation,
        }
    }
    fn principal(&self) -> String {
        match self {
            Self::Session(a) => a.session_id.clone(),
            Self::Service(a) => a.stored_id(),
        }
    }
    fn stored_id(&self) -> String {
        match self {
            Self::Session(a) => encode_sender(a),
            Self::Service(a) => a.stored_id(),
        }
    }
    fn wire_version(&self) -> &str {
        match self {
            Self::Session(_) => ENVELOPE_VERSION,
            Self::Service(_) => super::service::WIRE_VERSION,
        }
    }
    pub(super) fn valid(&self) -> bool {
        match self {
            Self::Session(a) => {
                !a.machine.is_empty()
                    && !a.session_id.is_empty()
                    && !a.session_incarnation.is_empty()
            }
            Self::Service(a) => a.valid(),
        }
    }
    pub(super) fn projection(&self) -> Value {
        let mut value = serde_json::to_value(self).expect("origin");
        if matches!(self, Self::Service(_)) {
            value["kind"] = json!("service");
        }
        value
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope<F = Address> {
    pub schema_version: String,
    pub message_id: String,
    pub from: F,
    pub to: Address,
    pub body: String,
    pub body_sha256: String,
    pub created_at_epoch: i64,
    pub expires_at_epoch: i64,
    pub reply_to: Option<String>,
    pub reply_depth: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<crate::cli::MessageCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarding: Option<super::forwarding::Provenance>,
}
impl<F: Into<Origin>> Envelope<F> {
    fn into_origin(self) -> Envelope<Origin> {
        Envelope {
            schema_version: self.schema_version,
            message_id: self.message_id,
            from: self.from.into(),
            to: self.to,
            body: self.body,
            body_sha256: self.body_sha256,
            created_at_epoch: self.created_at_epoch,
            expires_at_epoch: self.expires_at_epoch,
            reply_to: self.reply_to,
            reply_depth: self.reply_depth,
            category: self.category,
            forwarding: self.forwarding,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Outbox {
    pub envelope: Envelope<Origin>,
    pub request_digest: String,
    pub idempotency_key: String,
    pub state: String,
    pub attempts: u64,
    pub next_attempt_epoch: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_at_epoch: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_changed_at_epoch: Option<i64>,
    pub receipt: Option<Value>,
    pub reason: Option<String>,
}
impl Outbox {
    fn retains_failed_service_body(&self, now: i64) -> bool {
        matches!(self.envelope.from, Origin::Service(_))
            && matches!(self.state.as_str(), "rejected" | "delivery-unknown")
            && self.envelope.expires_at_epoch > now
    }
    fn identity(&self) -> Retained {
        Retained {
            message_id: self.envelope.message_id.clone(),
            sender: self.envelope.from.clone(),
            recipient: self.envelope.to.clone(),
            expires_at_epoch: self.envelope.expires_at_epoch,
            created_at_epoch: Some(self.envelope.created_at_epoch),
            last_attempt_at_epoch: self.last_attempt_at_epoch,
            state_changed_at_epoch: self.state_changed_at_epoch,
            request_digest: self.request_digest.clone(),
            idempotency_key: self.idempotency_key.clone(),
            state: self.state.clone(),
            attempts: self.attempts,
            reason: self.reason.clone(),
            category: self.envelope.category,
            forwarding: self.envelope.forwarding.clone(),
            persisted_at_epoch: self
                .receipt
                .as_ref()
                .and_then(|receipt| receipt["persisted_at_epoch"].as_i64()),
        }
    }
}
/// Body-free record of a terminal (never retried) outbox entry. It keeps what
/// idempotent replay, reply replay and delivery status read, so terminal
/// envelopes stop occupying the pending caps. It is always smaller than the
/// entry it replaces: a delivered receipt is rebuilt from `persisted_at_epoch`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Retained {
    message_id: String,
    sender: Origin,
    recipient: Address,
    expires_at_epoch: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_at_epoch: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_attempt_at_epoch: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    state_changed_at_epoch: Option<i64>,
    request_digest: String,
    idempotency_key: String,
    state: String,
    attempts: u64,
    reason: Option<String>,
    persisted_at_epoch: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<crate::cli::MessageCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    forwarding: Option<super::forwarding::Provenance>,
}
impl Retained {
    /// The receipt drain accepted: exactly these fields, checked against this
    /// message ID and recipient.
    fn receipt(&self) -> Option<Value> {
        self.persisted_at_epoch.map(|persisted| {
            json!({"schema_version":DELIVERY_VERSION,"message_id":self.message_id,"state":"delivered","recipient":self.recipient,"persisted_at_epoch":persisted})
        })
    }
}
// Independent source state; existing registry writers never deserialize or rewrite it.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: String,
    remote_outbox: Vec<Outbox>,
    #[serde(default)]
    retained: Vec<Retained>,
}
impl Journal {
    /// Failed service content stays recoverable until its original expiry.
    /// Other terminal entries compact to body-free records immediately; every
    /// identity is retained through expiry plus 24 hours, never evicting live IDs.
    fn compact(&mut self, now: i64) {
        let (queued, terminal) = std::mem::take(&mut self.remote_outbox)
            .into_iter()
            .partition(|item| item.state == "queued" || item.retains_failed_service_body(now));
        self.remote_outbox = queued;
        self.retained.extend(terminal.iter().map(Outbox::identity));
        self.remote_outbox
            .retain(|i| i.envelope.expires_at_epoch.saturating_add(86400) > now);
        self.retained
            .retain(|i| i.expires_at_epoch.saturating_add(86400) > now);
    }
    fn find(&self, mut matches: impl FnMut(&Retained) -> bool) -> Option<Retained> {
        self.remote_outbox
            .iter()
            .map(Outbox::identity)
            .chain(self.retained.iter().cloned())
            .find(|item| matches(item))
    }
    /// Source-side `quota-exceeded` with pending and delivered counts.
    fn quota(
        &self,
        (message, quota, count, limit): (&str, &str, usize, usize),
        host: &str,
        destination: &str,
    ) -> CliError {
        let mut error = super::quota_origin(
            super::quota_exceeded(message, quota, count, limit),
            host,
            "source",
        );
        if let Some(Value::Object(details)) = error.details_mut() {
            let pending_to_destination = self
                .remote_outbox
                .iter()
                .filter(|item| item.envelope.to.machine == destination)
                .count();
            let delivered = self
                .retained
                .iter()
                .filter(|item| item.state == "delivered")
                .count();
            details.insert("destination_machine".into(), destination.into());
            details.insert("pending".into(), self.remote_outbox.len().into());
            details.insert(
                "pending_to_destination".into(),
                pending_to_destination.into(),
            );
            details.insert("delivered".into(), delivered.into());
            details.insert("retained".into(), self.retained.len().into());
        }
        error
    }
    fn find_key(&self, session: &str, incarnation: &str, key: &str) -> Option<Retained> {
        self.find(|item| {
            item.sender.principal() == session
                && item.sender.generation() == incarnation
                && item.idempotency_key == key
        })
    }
}
struct LockedJournal {
    _lock: std::fs::File,
    path: std::path::PathBuf,
    registry: Journal,
}
fn journal_error() -> CliError {
    CliError::runtime(
        "federation-journal-invalid",
        "federation journal is corrupt or unsupported",
        None,
    )
}
fn read_journal(context: &CliContext) -> Result<Journal, CliError> {
    let path = context.state_dir.join("coordination").join(JOURNAL_FILE);
    match super::read_private_file(&path, MAX_JOURNAL_BYTES) {
        Ok(bytes) => {
            let journal: Journal = serde_json::from_slice(&bytes).map_err(|_| journal_error())?;
            let supported = match journal.schema_version.as_str() {
                JOURNAL_VERSION | EXTENDED_JOURNAL_VERSION | AUDIT_JOURNAL_VERSION => true,
                LEGACY_JOURNAL_VERSION => journal.retained.is_empty(),
                _ => false,
            };
            if !supported
                || journal.remote_outbox.len() > MAX_PENDING
                || journal.remote_outbox.len() + journal.retained.len() > MAX_RETAINED_IDS
            {
                return Err(journal_error());
            }
            Ok(journal)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Journal {
            schema_version: JOURNAL_VERSION.into(),
            ..Journal::default()
        }),
        Err(_) => Err(journal_error()),
    }
}
// Lock order is session -> registry -> journal. Journal-only operations never
// acquire either outer lock, and all guards are dropped before HTTP.
fn lock_journal(context: &CliContext) -> Result<LockedJournal, CliError> {
    let lock = super::lock_private_store(context, "federation-journal.lock")?;
    Ok(LockedJournal {
        path: context.state_dir.join("coordination").join(JOURNAL_FILE),
        registry: read_journal(context)?,
        _lock: lock,
    })
}
impl LockedJournal {
    fn save(&mut self) -> Result<(), CliError> {
        let bytes = self.encode()?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(journal_error());
        }
        self.write(&bytes)
    }
    fn encode(&mut self) -> Result<Vec<u8>, CliError> {
        self.registry.compact(now_epoch());
        let audit = self.registry.schema_version == AUDIT_JOURNAL_VERSION
            || self.registry.remote_outbox.iter().any(|entry| {
                entry.last_attempt_at_epoch.is_some() || entry.state_changed_at_epoch.is_some()
            })
            || self.registry.retained.iter().any(|entry| {
                entry.created_at_epoch.is_some()
                    || entry.last_attempt_at_epoch.is_some()
                    || entry.state_changed_at_epoch.is_some()
            });
        let extended = self.registry.schema_version == EXTENDED_JOURNAL_VERSION
            || self.registry.remote_outbox.iter().any(|entry| {
                entry.envelope.category.is_some() || entry.envelope.forwarding.is_some()
            })
            || self
                .registry
                .retained
                .iter()
                .any(|entry| entry.category.is_some() || entry.forwarding.is_some());
        self.registry.schema_version = if audit {
            AUDIT_JOURNAL_VERSION
        } else if extended {
            EXTENDED_JOURNAL_VERSION
        } else {
            JOURNAL_VERSION
        }
        .into();
        serde_json::to_vec(&self.registry).map_err(|_| journal_error())
    }
    fn write(&self, bytes: &[u8]) -> Result<(), CliError> {
        write_atomic(&self.path, bytes, SECRET_FILE_MODE).map_err(|_| journal_error())
    }
}
// Colon is forbidden by validate_id. Old facades preserve this existing string
// field, and old reply cannot resolve it as a local session or controller.
const SENDER_PREFIX: &str = "remote:";
pub(crate) fn is_remote_sender(id: &str) -> bool {
    id.starts_with(SENDER_PREFIX)
}
fn encode_sender(address: &Address) -> String {
    let encoded =
        serde_json::to_string(&(&address.machine, &address.session_id)).expect("string tuple");
    format!("{SENDER_PREFIX}{encoded}")
}
pub(crate) fn sender_address(message: &super::mailbox::StoredMessage) -> Option<Address> {
    sender_address_metadata(&message.sender_session_id, &message.sender_incarnation)
}
pub(super) fn sender_address_metadata(
    sender_id: &str,
    sender_incarnation: &str,
) -> Option<Address> {
    let encoded = sender_id.strip_prefix(SENDER_PREFIX)?;
    let (machine, session_id): (String, String) = serde_json::from_str(encoded).ok()?;
    let address = Address {
        machine,
        session_id,
        session_incarnation: sender_incarnation.to_string(),
    };
    (encode_sender(&address) == sender_id).then_some(address)
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submit {
    pub to_machine: String,
    pub to_session: String,
    pub body: String,
    pub idempotency_key: String,
    pub reply_to: Option<String>,
    pub expires_in: Option<String>,
    pub reply_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_recipient_incarnation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<crate::cli::MessageCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forward: Option<super::forwarding::Request>,
}

/// Bounded federation HTTP client: 15-second timeout, redirects refused.
pub(crate) fn client() -> Result<reqwest::blocking::Client, CliError> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| unavailable())
}
fn unavailable() -> CliError {
    CliError::runtime(
        "remote-messaging-unavailable",
        "remote mailbox service is unavailable",
        None,
    )
}
fn invalid() -> CliError {
    CliError::data(
        "remote-message-invalid",
        "remote message envelope is invalid",
        None,
    )
}
/// Who answered an HTTP request: the federation relay (a remote host) or this
/// host's own daemon, whose refusals are local and keep their own diagnostics.
#[derive(Clone, Copy)]
enum Responder {
    Relay,
    LocalDaemon,
}
fn response_json(
    response: reqwest::blocking::Response,
    responder: Responder,
) -> Result<Value, CliError> {
    let status = response.status();
    let bytes = response.bytes().map_err(|_| unavailable())?;
    if bytes.len() > 1024 * 1024 {
        return Err(unavailable());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if status.is_success() {
        return Ok(value);
    }
    Err(match responder {
        Responder::Relay => relay_error(&value),
        Responder::LocalDaemon => local_daemon_error(&value),
    })
}
pub(super) fn local_response(response: reqwest::blocking::Response) -> Result<Value, CliError> {
    response_json(response, Responder::LocalDaemon)
}
fn local_daemon_error(value: &Value) -> CliError {
    let text = |pointer| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty() && text.len() <= 512)
    };
    let Some(code) = text("/error/code").filter(|code| {
        code.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    }) else {
        return unavailable();
    };
    let details = value
        .pointer("/error/details")
        .filter(|details| details.is_object())
        .cloned();
    CliError::data(
        code,
        text("/error/message").unwrap_or("local agent-session daemon rejected the request"),
        details,
    )
}
pub(super) fn relay_reason_code(code: &str) -> Option<&'static str> {
    [
        "unauthorized",
        "ownership-unknown",
        "origin-forbidden",
        "principal-forbidden",
        "machine-forbidden",
        "federation-disabled",
        "coordination-unauthorized",
        "ownership-not-found",
        "ownership-mismatch",
        "session-incarnation-conflict",
        "message-not-found",
        "remote-messaging-unsupported",
        "remote-message-invalid",
        "quota-exceeded",
        "rate-limited",
        "idempotency-key-conflict",
        "message-expired",
        "message-revision-conflict",
        "message-category-conflict",
        "message-forward-loop",
        "message-forward-depth-exceeded",
        "message-forward-invalid",
    ]
    .into_iter()
    .find(|known| *known == code)
}
/// Relay refusals keep only allowlisted codes and the content-free quota fields.
fn relay_error(value: &Value) -> CliError {
    let code = value
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("remote-messaging-unavailable");
    let code = relay_reason_code(code).unwrap_or("remote-messaging-unavailable");
    let details = (code == "quota-exceeded")
        .then(|| value.pointer("/error/details").and_then(Value::as_object))
        .flatten()
        .map(|details| {
            ["quota", "count", "limit", "host", "side"]
                .into_iter()
                .filter_map(|key| {
                    let field = details.get(key)?;
                    (field.is_u64() || field.as_str().is_some_and(|text| text.len() <= 128))
                        .then(|| (key.to_string(), field.clone()))
                })
                .collect::<serde_json::Map<_, _>>()
        })
        .filter(|kept| !kept.is_empty())
        .map(Value::Object);
    CliError::data(code, "remote mailbox request was rejected", details)
}
pub(crate) fn peers(
    context: &CliContext,
    config: &Config,
    session: &str,
    token: &str,
) -> Result<Value, CliError> {
    let (_, current) = authenticate_token(context, session, token)?;
    peers_from(
        config,
        &Origin::Session(Address {
            machine: config.machine.clone(),
            session_id: session.into(),
            session_incarnation: current,
        }),
    )
}
fn peers_from(config: &Config, origin: &Origin) -> Result<Value, CliError> {
    let mut url = reqwest::Url::parse(&format!("{}/api/coordination/peers/v1", config.url))
        .map_err(|_| unavailable())?;
    match origin {
        Origin::Session(source) => {
            url.query_pairs_mut()
                .append_pair("source_session_id", &source.session_id)
                .append_pair("source_incarnation", &source.session_incarnation);
        }
        Origin::Service(source) => {
            url.query_pairs_mut()
                .append_pair("source_service_id", &source.service_id)
                .append_pair("source_service_generation", &source.service_generation);
        }
    }
    let response = client()?
        .get(url)
        .bearer_auth(&config.token)
        .send()
        .map_err(|_| unavailable())?;
    let value = response_json(response, Responder::Relay)?;
    if value["schema_version"] != "agent-session.remote-peers.v1" || !value["peers"].is_array() {
        return Err(unavailable());
    }
    Ok(value)
}

pub(crate) fn submit(
    context: &CliContext,
    config: &Config,
    session: &str,
    token: &str,
    args: Submit,
) -> Result<Value, CliError> {
    let (_, source_incarnation) = authenticate_token(context, session, token)?;
    submit_origin(
        context,
        config,
        Origin::Session(Address {
            machine: config.machine.clone(),
            session_id: session.into(),
            session_incarnation: source_incarnation,
        }),
        args,
        Some(token),
        None,
        || Ok(()),
    )
}

pub(super) fn submit_service<F>(
    context: &CliContext,
    config: &Config,
    origin: super::service::Origin,
    args: super::service::Submit,
    authorize: F,
) -> Result<Value, CliError>
where
    F: Fn() -> Result<(), CliError>,
{
    if origin.machine != config.machine {
        return Err(super::service::unauthorized());
    }
    let digest = digest_bytes(&serde_json::to_vec(&args).map_err(|_| invalid())?);
    let submit = Submit {
        to_machine: args.to_machine.ok_or_else(invalid)?,
        to_session: args.to_session,
        body: args.body,
        idempotency_key: args.idempotency_key,
        reply_to: None,
        expires_in: args.expires_in,
        reply_revision: None,
        expected_recipient_incarnation: args.expected_recipient_incarnation,
        category: args.category,
        forward: None,
    };
    submit_origin(
        context,
        config,
        Origin::Service(origin),
        submit,
        None,
        Some(digest),
        authorize,
    )
}

fn submit_origin<F>(
    context: &CliContext,
    config: &Config,
    origin: Origin,
    mut args: Submit,
    token: Option<&str>,
    digest_override: Option<String>,
    authorize: F,
) -> Result<Value, CliError>
where
    F: Fn() -> Result<(), CliError>,
{
    authorize()?;
    let principal = origin.principal();
    let session = principal.as_str();
    let source_incarnation = origin.generation().to_string();
    super::validate_idempotency_key(&args.idempotency_key)?;
    if let Some(request) = &mut args.forward {
        request.normalize();
    }
    if (args.forward.is_none() && args.body.is_empty())
        || args.body.len() > 16 * 1024
        || args.idempotency_key.is_empty()
        || args.idempotency_key.len() > 256
    {
        return Err(invalid());
    }
    if args.forward.is_some()
        && (!matches!(origin, Origin::Session(_))
            || !args.body.is_empty()
            || args.category.is_some()
            || args.reply_to.is_some()
            || args.reply_revision.is_some()
            || args.expires_in.is_some())
    {
        return Err(invalid());
    }
    let digest = digest_override
        .unwrap_or_else(|| digest_bytes(&serde_json::to_vec(&args).expect("submission")));
    if matches!(origin, Origin::Service(_)) {
        let locked = super::lock_registry_observational(context)?;
        authorize()?;
        if let Some(prior) = super::idempotency_replay(
            &locked.registry,
            &args.idempotency_key,
            session,
            &source_incarnation,
            "service-message-submit",
            &digest,
        )? {
            return Ok(prior);
        }
    }
    // Replay before discovery: a retry must retain its original destination incarnation.
    {
        let locked = lock_journal(context)?;
        authorize()?;
        if let Some(item) =
            locked
                .registry
                .find_key(session, &source_incarnation, &args.idempotency_key)
        {
            if item.request_digest != digest {
                return Err(CliError::data(
                    "idempotency-key-conflict",
                    "idempotency key has different content",
                    None,
                ));
            }
            return Ok(projection(&item));
        }
    }
    let forward_source = if let Some(request) = &args.forward {
        let Origin::Session(actor) = &origin else {
            return Err(invalid());
        };
        let locked = lock_registry(context)?;
        let source =
            super::forwarding::source(&locked.registry, actor, request, now_epoch())?.clone();
        args.body = source.body.clone();
        args.category = source.category;
        Some(source)
    } else {
        None
    };
    let (destination, reply_depth) = if let Some(parent_id) = args.reply_to.as_deref() {
        let locked = lock_registry(context)?;
        let parent = locked
            .registry
            .messages
            .iter()
            .find(|m| {
                m.message_id == parent_id
                    && m.recipient_session_id == session
                    && m.recipient_incarnation == source_incarnation
            })
            .ok_or_else(|| {
                CliError::data("message-not-found", "reply message does not exist", None)
            })?;
        if parent.expires_at_epoch <= now_epoch() || parent.state == "expired" {
            return Err(CliError::data(
                "message-expired",
                "reply message expired",
                None,
            ));
        }
        if args
            .reply_revision
            .is_some_and(|expected| expected != parent.revision)
        {
            return Err(CliError::data(
                "message-revision-conflict",
                "reply revision changed",
                None,
            ));
        }
        let sender = sender_address(parent).ok_or_else(invalid)?;
        if sender.machine != args.to_machine
            || sender.session_id != args.to_session
            || parent.reply_depth >= 16
        {
            return Err(invalid());
        }
        (sender, parent.reply_depth + 1)
    } else {
        let discovered = peers_from(config, &origin)?;
        let peer = discovered["peers"]
            .as_array()
            .expect("validated peers")
            .iter()
            .find(|p| {
                p["machine"] == args.to_machine
                    && p["session_id"] == args.to_session
                    && p["messaging_supported"] == true
            })
            .ok_or_else(|| {
                CliError::data(
                    "remote-messaging-unsupported",
                    "remote recipient is not available",
                    None,
                )
            })?;
        (
            Address {
                machine: args.to_machine.clone(),
                session_id: args.to_session.clone(),
                session_incarnation: peer["session_incarnation"]
                    .as_str()
                    .ok_or_else(invalid)?
                    .into(),
            },
            0,
        )
    };
    if args
        .expected_recipient_incarnation
        .as_deref()
        .is_some_and(|expected| expected != destination.session_incarnation)
    {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "service recipient was replaced",
            None,
        ));
    }
    let expiry = super::mailbox::parse_expiry(args.expires_in.as_deref())?;
    let now = now_epoch();
    let forwarding = if let Some(source) = &forward_source {
        let Origin::Session(actor) = &origin else {
            return Err(invalid());
        };
        Some(super::forwarding::append(
            source,
            actor.clone(),
            destination.clone(),
            now,
        )?)
    } else {
        None
    };
    let envelope = Envelope {
        schema_version: envelope_version(&origin, args.category.is_some() || forwarding.is_some())
            .into(),
        message_id: uuid::Uuid::new_v4().to_string(),
        from: origin.clone(),
        to: destination,
        body_sha256: digest_bytes(args.body.as_bytes()),
        body: args.body,
        created_at_epoch: now,
        expires_at_epoch: forward_source
            .as_ref()
            .map(|source| source.expires_at_epoch)
            .unwrap_or_else(|| now.saturating_add(expiry)),
        reply_to: args.reply_to,
        reply_depth,
        category: args.category,
        forwarding,
    };
    // Authenticate again after discovery, then persist without any network-held locks.
    let _source_lock = if matches!(origin, Origin::Session(_)) {
        Some(crate::acquire_session_record_lock(context, session)?)
    } else {
        None
    };
    if let Some(token) = token {
        let (_, current) = authenticate_token(context, session, token)?;
        if current != source_incarnation {
            return Err(super::unauthorized());
        }
    }
    let locked = lock_registry(context)?;
    if let Some(token) = token {
        let broker = locked
            .registry
            .brokers
            .get(session)
            .ok_or_else(super::unauthorized)?;
        if broker.state != "ready"
            || broker.incarnation != source_incarnation
            || broker.capability_digest != digest_bytes(token.as_bytes())
        {
            return Err(super::unauthorized());
        }
    }
    authorize()?;
    if let Some(parent_id) = envelope.reply_to.as_deref() {
        let parent = locked
            .registry
            .messages
            .iter()
            .find(|m| {
                m.message_id == parent_id
                    && m.recipient_session_id == session
                    && m.recipient_incarnation == source_incarnation
            })
            .ok_or_else(invalid)?;
        if args
            .reply_revision
            .is_some_and(|expected| expected != parent.revision)
            || parent.expires_at_epoch <= now
            || sender_address(parent).as_ref() != Some(&envelope.to)
        {
            return Err(CliError::data(
                "message-revision-conflict",
                "reply parent changed before commit",
                None,
            ));
        }
    }
    if matches!(origin, Origin::Service(_))
        && let Some(prior) = super::idempotency_replay(
            &locked.registry,
            &args.idempotency_key,
            session,
            &source_incarnation,
            "service-message-submit",
            &digest,
        )?
    {
        return Ok(prior);
    }
    if let Some(request) = &args.forward {
        let Origin::Session(actor) = &origin else {
            return Err(invalid());
        };
        let source = super::forwarding::source(&locked.registry, actor, request, now_epoch())?;
        if source.body != envelope.body || source.category != envelope.category {
            return Err(invalid());
        }
    }
    let _registry_guard = locked;
    let mut locked = lock_journal(context)?;
    authorize()?;
    if let Some(item) =
        locked
            .registry
            .find_key(session, &source_incarnation, &args.idempotency_key)
    {
        if item.request_digest != digest {
            return Err(CliError::data(
                "idempotency-key-conflict",
                "idempotency key has different content",
                None,
            ));
        }
        return Ok(projection(&item));
    }
    // Expired entries retain an explicit unknown result; bounded capacity rejects, never evicts live IDs.
    locked.registry.compact(now);
    let destination = envelope.to.machine.clone();
    let journal = &locked.registry;
    let pending = journal.remote_outbox.len();
    let pending_to_destination = journal
        .remote_outbox
        .iter()
        .filter(|item| item.envelope.to.machine == destination)
        .count();
    let identities = pending + journal.retained.len();
    let refusal = if pending_to_destination >= MAX_PENDING_PER_DESTINATION {
        Some((
            "local federation outbox has too many undelivered messages for this destination",
            "federation-pending-destination",
            pending_to_destination,
            MAX_PENDING_PER_DESTINATION,
        ))
    } else if pending >= MAX_PENDING {
        Some((
            "local federation outbox has too many undelivered messages",
            "federation-pending",
            pending,
            MAX_PENDING,
        ))
    } else if identities >= MAX_RETAINED_IDS {
        Some((
            "local federation outbox retained-ID limit reached",
            "federation-retained-ids",
            identities,
            MAX_RETAINED_IDS,
        ))
    } else {
        None
    };
    if let Some(refusal) = refusal {
        return Err(journal.quota(refusal, &config.machine, &destination));
    }
    let item = Outbox {
        envelope,
        request_digest: digest,
        idempotency_key: args.idempotency_key,
        state: "queued".into(),
        attempts: 0,
        next_attempt_epoch: now,
        last_attempt_at_epoch: None,
        state_changed_at_epoch: Some(now),
        receipt: None,
        reason: None,
    };
    let outcome = projection(&item.identity());
    locked.registry.remote_outbox.push(item);
    let bytes = locked.encode()?;
    let budget = MAX_JOURNAL_BYTES - JOURNAL_HEADROOM_BYTES;
    if bytes.len() as u64 > budget {
        locked.registry.remote_outbox.pop();
        return Err(locked.registry.quota(
            (
                "local federation journal byte budget reached",
                "federation-journal-bytes",
                bytes.len(),
                budget as usize,
            ),
            &config.machine,
            &destination,
        ));
    }
    authorize()?;
    locked.write(&bytes)?;
    Ok(outcome)
}
fn projection(item: &Retained) -> Value {
    let mut value = json!({"schema_version":DELIVERY_VERSION,"message_id":item.message_id,"state":item.state,"sender":item.sender.projection(),"recipient":item.recipient,"attempts":item.attempts,"reason":item.reason,"receipt":item.receipt()});
    value["category"] = json!(item.category.unwrap_or_default());
    if let Some(forwarding) = &item.forwarding {
        value["forwarding"] = json!(forwarding);
    }
    value
}
/// Read-only reconciliation of the daemon's provider-incident outbox across
/// runtime replacement. New sends still require the current capability.
pub(crate) fn auth_incident_receipt(
    context: &CliContext,
    session: &str,
    key: &str,
    expected_digest: &str,
) -> Result<Option<Value>, CliError> {
    let locked = lock_journal(context)?;
    match locked
        .registry
        .find(|item| item.sender.principal() == session && item.idempotency_key == key)
    {
        Some(item) if item.request_digest != expected_digest => Err(CliError::data(
            "idempotency-key-conflict",
            "authentication notification identity differs",
            None,
        )),
        Some(item) => Ok(Some(projection(&item))),
        None => Ok(None),
    }
}

pub(crate) fn delivery(
    context: &CliContext,
    session: &str,
    token: &str,
    id: &str,
) -> Result<Value, CliError> {
    let (_, current) = authenticate_token(context, session, token)?;
    let locked = lock_journal(context)?;
    let item = locked
        .registry
        .find(|i| {
            i.message_id == id
                && i.sender.principal() == session
                && i.sender.generation() == current
        })
        .ok_or_else(|| CliError::data("message-not-found", "delivery does not exist", None))?;
    Ok(projection(&item))
}
pub(crate) fn drain(context: &CliContext, config: &Config) -> Result<Option<i64>, CliError> {
    let items = {
        let mut locked = lock_journal(context)?;
        let now = now_epoch();
        if locked.registry.remote_outbox.iter().any(|item| {
            matches!(item.envelope.from, Origin::Service(_))
                && matches!(item.state.as_str(), "rejected" | "delivery-unknown")
                && item.envelope.expires_at_epoch <= now
        }) {
            // Expiry wakes must compact held content even without a queued attempt.
            locked.save()?;
        }
        locked
            .registry
            .remote_outbox
            .iter()
            .filter(|i| {
                i.state == "queued"
                    && (i.next_attempt_epoch <= now_epoch()
                        || i.envelope.expires_at_epoch <= now_epoch())
            })
            .min_by_key(|i| (i.next_attempt_epoch, i.attempts))
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    };
    for item in items {
        let now = now_epoch();
        let relay_attempted = item.envelope.expires_at_epoch > now;
        let result = if !relay_attempted {
            Err(CliError::data(
                "delivery-unknown",
                "remote delivery expired without a confirmed receipt",
                None,
            ))
        } else {
            client()?
                .post(format!("{}/api/coordination/relay/v1", config.url))
                .bearer_auth(&config.token)
                .json(&item.envelope)
                .send()
                .map_err(|_| unavailable())
                .and_then(|response| response_json(response, Responder::Relay))
        };
        let mut locked = lock_journal(context)?;
        let Some(current) = locked
            .registry
            .remote_outbox
            .iter_mut()
            .find(|i| i.envelope.message_id == item.envelope.message_id && i.state == "queued")
        else {
            continue;
        };
        let previous_state = current.state.clone();
        if relay_attempted {
            current.last_attempt_at_epoch = Some(now);
        }
        current.attempts = current.attempts.saturating_add(1);
        current.next_attempt_epoch = now_epoch().saturating_add(RETRY_SECS);
        let unreachable = matches!(
            &result,
            Err(error) if matches!(
                error.code(),
                "remote-messaging-unavailable" | "coordination-unavailable"
            )
        );
        match result {
            Ok(receipt)
                if receipt["schema_version"] == DELIVERY_VERSION
                    && receipt["message_id"] == item.envelope.message_id
                    && receipt["state"] == "delivered"
                    && receipt["recipient"]
                        == serde_json::to_value(&item.envelope.to).map_err(|_| invalid())? =>
            {
                current.state = "delivered".into();
                current.receipt = Some(receipt);
                current.reason = None;
            }
            Ok(_) => {
                current.reason = Some("remote-receipt-invalid".into());
            }
            Err(error) => {
                current.reason = Some(error.code().into());
                if error.code() == "delivery-unknown" {
                    current.state = "delivery-unknown".into();
                } else if !matches!(
                    error.code(),
                    "remote-messaging-unavailable" | "rate-limited" | "coordination-unavailable"
                ) {
                    // A previous unconfirmed attempt may already have persisted.
                    // Later admission failure cannot establish nondelivery.
                    current.state = if current.attempts > 1 {
                        "delivery-unknown"
                    } else {
                        "rejected"
                    }
                    .into();
                }
            }
        }
        if current.state != previous_state {
            current.state_changed_at_epoch = Some(now_epoch());
        }
        if unreachable {
            // Envelopes that already failed share one probe per retry interval
            // to an unavailable destination session, so it cannot delay other
            // sessions or machines. A fresh envelope gets its own first attempt.
            let retry_at = now_epoch().saturating_add(RETRY_SECS);
            for other in locked.registry.remote_outbox.iter_mut().filter(|other| {
                other.state == "queued"
                    && other.attempts > 0
                    && other.envelope.to == item.envelope.to
            }) {
                other.next_attempt_epoch = other.next_attempt_epoch.max(retry_at);
            }
        }
        locked.save()?;
    }
    let locked = lock_journal(context)?;
    let now = now_epoch();
    Ok(locked
        .registry
        .remote_outbox
        .iter()
        .filter(|item| item.state == "queued" || item.retains_failed_service_body(now))
        .map(|item| {
            if item.state == "queued" {
                item.next_attempt_epoch.min(item.envelope.expires_at_epoch)
            } else {
                item.envelope.expires_at_epoch
            }
        })
        .min())
}
/// Destination admission. Quota refusals name this host as the destination.
pub(crate) fn receive<F: Into<Origin>>(
    context: &CliContext,
    machine: &str,
    envelope: Envelope<F>,
) -> Result<Value, CliError> {
    receive_admitted(context, machine, envelope.into_origin(), None, &|| Ok(()))
        .map_err(|error| super::quota_origin(error, machine, "destination"))
}

struct LocalSubmission {
    origin: super::service::Origin,
    key: String,
    digest: String,
}
fn local_replay(
    registry: &super::Registry,
    source: Option<&LocalSubmission>,
) -> Result<Option<Value>, CliError> {
    let Some(source) = source else {
        return Ok(None);
    };
    super::idempotency_replay(
        registry,
        &source.key,
        &source.origin.stored_id(),
        &source.origin.service_generation,
        "service-message-submit",
        &source.digest,
    )
}

pub(super) fn submit_local_service<F>(
    context: &CliContext,
    origin: super::service::Origin,
    args: super::service::Submit,
    digest: String,
    authorize: F,
) -> Result<Value, CliError>
where
    F: Fn() -> Result<(), CliError>,
{
    let source = LocalSubmission {
        origin: origin.clone(),
        key: args.idempotency_key,
        digest,
    };
    {
        let locked = super::lock_registry_observational(context)?;
        authorize()?;
        if let Some(prior) = local_replay(&locked.registry, Some(&source))? {
            return Ok(prior);
        }
        reject_remote_key(context, &source)?;
    }
    let recipient = crate::load_session_record(context, &args.to_session)?;
    if recipient.id != args.to_session {
        return Err(CliError::data(
            "message-not-found",
            "service recipient requires an exact session id",
            None,
        ));
    }
    let target_incarnation = incarnation(&recipient)?;
    if args
        .expected_recipient_incarnation
        .as_deref()
        .is_some_and(|expected| expected != target_incarnation)
    {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "service recipient was replaced",
            None,
        ));
    }
    let now = now_epoch();
    let envelope = Envelope {
        schema_version: if args.category.is_some() {
            SERVICE_ENVELOPE_V2
        } else {
            super::service::WIRE_VERSION
        }
        .into(),
        message_id: uuid::Uuid::new_v4().to_string(),
        from: Origin::Service(origin.clone()),
        to: Address {
            machine: origin.machine.clone(),
            session_id: args.to_session,
            session_incarnation: target_incarnation,
        },
        body_sha256: digest_bytes(args.body.as_bytes()),
        body: args.body,
        created_at_epoch: now,
        expires_at_epoch: now
            .saturating_add(super::mailbox::parse_expiry(args.expires_in.as_deref())?),
        reply_to: None,
        reply_depth: 0,
        category: args.category,
        forwarding: None,
    };
    receive_admitted(
        context,
        &origin.machine,
        envelope,
        Some(&source),
        &authorize,
    )
}
fn reject_remote_key(context: &CliContext, source: &LocalSubmission) -> Result<(), CliError> {
    if lock_journal(context)?
        .registry
        .find_key(
            &source.origin.stored_id(),
            &source.origin.service_generation,
            &source.key,
        )
        .is_some()
    {
        return Err(CliError::data(
            "idempotency-key-conflict",
            "service idempotency key belongs to another request",
            None,
        ));
    }
    Ok(())
}

fn receive_admitted(
    context: &CliContext,
    machine: &str,
    envelope: Envelope<Origin>,
    source: Option<&LocalSubmission>,
    authorize: &dyn Fn() -> Result<(), CliError>,
) -> Result<Value, CliError> {
    let now = now_epoch();
    if envelope.schema_version
        != envelope_version(
            &envelope.from,
            envelope.category.is_some() || envelope.forwarding.is_some(),
        )
        || uuid::Uuid::parse_str(&envelope.message_id).is_err()
        || envelope.to.machine != machine
        || envelope.body.is_empty()
        || envelope.body.len() > super::mailbox::BODY_MAX_BYTES
        || envelope.body_sha256 != digest_bytes(envelope.body.as_bytes())
        || envelope.reply_depth > super::mailbox::MAX_REPLY_DEPTH
        || envelope
            .expires_at_epoch
            .saturating_sub(envelope.created_at_epoch)
            > super::mailbox::MAX_EXPIRY_SECS
        || !envelope.from.valid()
        || (matches!(envelope.from, Origin::Service(_))
            && (envelope.reply_to.is_some() || envelope.reply_depth != 0))
    {
        return Err(invalid());
    }
    if let Some(provenance) = &envelope.forwarding {
        let Origin::Session(actor) = &envelope.from else {
            return Err(invalid());
        };
        if provenance
            .hops
            .last()
            .is_none_or(|hop| &hop.forwarder != actor || !hop.recipient_transfers.is_empty())
            || !provenance.valid(&envelope.to, &envelope.body, envelope.expires_at_epoch, now)
        {
            return Err(invalid());
        }
    }
    super::mailbox::validate_body(&envelope.body)?;
    authorize()?;
    let digest = digest_bytes(&serde_json::to_vec(&envelope).map_err(|_| invalid())?);
    // Durable delivery wins over later expiry or replacement. Release this lock
    // before the session lock; the final transaction repeats the same check.
    {
        let locked = super::lock_registry_observational(context)?;
        authorize()?;
        if let Some(prior) = local_replay(&locked.registry, source)? {
            return Ok(prior);
        }
        if let Some(prior) = receive_replay(&locked.registry, &envelope, &digest)? {
            return Ok(prior);
        }
    }
    if envelope.created_at_epoch > now.saturating_add(60) || envelope.expires_at_epoch <= now {
        return Err(invalid());
    }
    let _session_lock = crate::acquire_session_record_lock(context, &envelope.to.session_id)?;
    let recipient = crate::load_session_record(context, &envelope.to.session_id)?;
    if matches!(envelope.from, Origin::Service(_)) && recipient.id != envelope.to.session_id {
        return Err(CliError::data(
            "message-not-found",
            "service recipient requires an exact session id",
            None,
        ));
    }
    let mut locked = lock_registry(context)?;
    authorize()?;
    if let Some(prior) = local_replay(&locked.registry, source)? {
        return Ok(prior);
    }
    if let Some(source) = source {
        reject_remote_key(context, source)?;
    }
    if let Some(prior) = receive_replay(&locked.registry, &envelope, &digest)? {
        return Ok(prior);
    }
    if incarnation(&recipient)? != envelope.to.session_incarnation {
        return Err(CliError::data(
            "session-incarnation-conflict",
            "remote recipient was replaced",
            None,
        ));
    }
    if locked
        .registry
        .messages
        .iter()
        .any(|message| message.message_id == envelope.message_id)
    {
        return Err(CliError::data(
            "idempotency-key-conflict",
            "message ID already exists",
            None,
        ));
    }
    let broker = locked
        .registry
        .brokers
        .get(&recipient.id)
        .filter(|b| {
            b.state == "ready"
                && b.incarnation == envelope.to.session_incarnation
                && super::broker::capability_available(
                    context,
                    &recipient.id,
                    &b.incarnation,
                    &b.capability_digest,
                )
                && super::broker::heartbeat_fresh(
                    context,
                    &recipient.id,
                    &b.incarnation,
                    b.heartbeat_epoch,
                )
        })
        .ok_or_else(unavailable)?;
    let recipient_incarnation = broker.incarnation.clone();
    let now_millis = super::mailbox::now_epoch_millis();
    super::mailbox::admit_message(
        &mut locked.registry,
        &envelope.from.stored_id(),
        &recipient.id,
        envelope.body.len(),
        envelope.reply_to.as_deref(),
        now,
        now_millis,
    )?;
    let receipt = json!({"schema_version":DELIVERY_VERSION,"message_id":envelope.message_id,"state":"delivered","recipient":envelope.to,"persisted_at_epoch":now});
    let message = super::mailbox::StoredMessage {
        schema_version: "agent-session.message.v1".into(),
        message_id: envelope.message_id.clone(),
        sender_session_id: envelope.from.stored_id(),
        sender_incarnation: envelope.from.generation().to_string(),
        recipient_session_id: recipient.id.clone(),
        recipient_incarnation: recipient_incarnation.clone(),
        state: "unread".into(),
        revision: 1,
        reply_to: envelope.reply_to,
        reply_depth: envelope.reply_depth,
        created_at: timestamp(now),
        created_at_epoch: now,
        remote_created_at_epoch: Some(envelope.created_at_epoch),
        created_at_epoch_millis: now_millis,
        expires_at: timestamp(envelope.expires_at_epoch),
        expires_at_epoch: envelope.expires_at_epoch,
        terminal_at_epoch: None,
        forwarded_from_incarnation: None,
        forwarded_at_epoch: None,
        category: envelope.category,
        forwarding: envelope.forwarding,
        resume_carry: None,
        body_bytes: envelope.body.len(),
        body: envelope.body,
    };
    let notification = super::notification::schedule(
        &mut locked.registry,
        &recipient.id,
        &recipient_incarnation,
        now,
    );
    let mut local_outcome = super::mailbox::message_projection(&message);
    local_outcome["notification"] = serde_json::to_value(notification).expect("notification");
    locked.registry.messages.push(message);
    super::store_receipt(
        &mut locked.registry,
        envelope.message_id.clone(),
        RECEIVE_PRINCIPAL.into(),
        RECEIVE_INCARNATION.into(),
        RECEIVE_OPERATION.into(),
        digest,
        receipt.clone(),
        now,
    )?;
    // All existing writers retain this exact field and prune by it. Keep the
    // receipt beyond every valid envelope replay even when inbox is acknowledged.
    let key = super::receipt_key(
        RECEIVE_PRINCIPAL,
        RECEIVE_INCARNATION,
        RECEIVE_OPERATION,
        &envelope.message_id,
    );
    locked
        .registry
        .receipts
        .get_mut(&key)
        .expect("just inserted")
        .expires_at_epoch = envelope.expires_at_epoch.saturating_add(86400);
    if let Some(source) = source {
        super::store_receipt(
            &mut locked.registry,
            source.key.clone(),
            source.origin.stored_id(),
            source.origin.service_generation.clone(),
            "service-message-submit".into(),
            source.digest.clone(),
            local_outcome.clone(),
            now,
        )?;
    }
    authorize()?;
    locked.save()?;
    Ok(if source.is_some() {
        local_outcome
    } else {
        receipt
    })
}

pub(crate) fn write_endpoint(
    context: &CliContext,
    address: std::net::SocketAddr,
) -> Result<(), CliError> {
    drop(lock_registry(context)?);
    let address = if address.ip().is_unspecified() {
        std::net::SocketAddr::from(([127, 0, 0, 1], address.port()))
    } else {
        address
    };
    write_atomic(
        &context.state_dir.join("coordination/daemon-endpoint.json"),
        &serde_json::to_vec(&json!({"url":format!("http://{address}")}))
            .map_err(|_| unavailable())?,
        SECRET_FILE_MODE,
    )
    .map_err(|_| unavailable())
}
/// The loopback URL the local daemon published in
/// `coordination/daemon-endpoint.json`.
pub(crate) fn daemon_url(context: &CliContext) -> Result<reqwest::Url, CliError> {
    let endpoint: Value = serde_json::from_slice(
        &super::read_private_file(
            &context.state_dir.join("coordination/daemon-endpoint.json"),
            4096,
        )
        .map_err(|_| unavailable())?,
    )
    .map_err(|_| unavailable())?;
    let url = reqwest::Url::parse(endpoint["url"].as_str().ok_or_else(unavailable)?)
        .map_err(|_| unavailable())?;
    if url.scheme() != "http"
        || !matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(unavailable());
    }
    Ok(url)
}
fn local_request(
    context: &CliContext,
    session: &str,
    capability: Option<&Path>,
    path: &str,
    body: Option<Value>,
) -> Result<Value, CliError> {
    let capability = super::mailbox::resolve_capability_file(capability)?;
    authenticate_from_file(context, session, Some(&capability))?;
    let token = String::from_utf8(
        super::read_private_file(&capability, 256).map_err(|_| super::unauthorized())?,
    )
    .map_err(|_| super::unauthorized())?;
    let url = daemon_url(context)?;
    let client = client()?;
    let request = match body {
        Some(body) => client
            .post(format!("{}{path}", url.as_str().trim_end_matches('/')))
            .json(&body),
        None => client.get(format!("{}{path}", url.as_str().trim_end_matches('/'))),
    };
    response_json(
        request
            .bearer_auth(token.trim())
            .send()
            .map_err(|_| unavailable())?,
        Responder::LocalDaemon,
    )
}
fn envelope_version(origin: &Origin, extended: bool) -> &str {
    if !extended {
        return origin.wire_version();
    }
    match origin {
        Origin::Session(_) => ENVELOPE_V2,
        Origin::Service(_) => SERVICE_ENVELOPE_V2,
    }
}
fn with_category(mut value: Value, category: Option<crate::cli::MessageCategory>) -> Value {
    if let Some(category) = category {
        value["category"] = json!(category);
    }
    value
}
pub(crate) fn cli_forward(
    context: &CliContext,
    args: crate::cli::MessageForwardArgs,
) -> Result<Value, CliError> {
    let mut request = super::forwarding::Request {
        message: args.message,
        if_revision: args.if_revision,
        categories: args.categories,
    };
    request.normalize();
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/remote/v1", args.session),
        Some(json!({
            "to_machine": args.to_machine, "to_session": args.to_session, "body": "",
            "idempotency_key": args.idempotency_key, "reply_to": null, "expires_in": null,
            "reply_revision": null, "forward": request,
        })),
    )
}

pub(crate) fn cli_send(
    context: &CliContext,
    args: crate::cli::MessageSendArgs,
) -> Result<Value, CliError> {
    let body = super::mailbox::read_body(&args.body_file)?;
    local_request(
        context,
        &args.from_session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/remote/v1", args.from_session),
        Some(with_category(
            json!({"to_machine":args.to_machine,"to_session":args.to_session,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.reply_to,"expires_in":args.expires_in,"reply_revision":null}),
            args.category,
        )),
    )
}
pub(crate) fn cli_reply(
    context: &CliContext,
    args: &crate::cli::MessageReplyArgs,
    original: &super::mailbox::StoredMessage,
    body: String,
) -> Result<Value, CliError> {
    let sender = sender_address(original).ok_or_else(invalid)?;
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/remote/v1", args.session),
        Some(with_category(
            json!({"to_machine":sender.machine,"to_session":sender.session_id,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.message,"expires_in":null,"reply_revision":args.if_revision}),
            args.category,
        )),
    )
}
pub(crate) fn cli_peers(
    context: &CliContext,
    args: crate::cli::MessagePeersArgs,
) -> Result<Value, CliError> {
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!("/sessions/{}/messages/peers/v1", args.session),
        None,
    )
}
pub(crate) fn cli_delivery(
    context: &CliContext,
    args: crate::cli::MessageDeliveryArgs,
) -> Result<Value, CliError> {
    local_request(
        context,
        &args.session,
        args.capability_file.as_deref(),
        &format!(
            "/sessions/{}/messages/{}/delivery/v1",
            args.session, args.message
        ),
        None,
    )
}

/// Retained outbox identity survives parent inbox retention just like local reply receipts.
pub(crate) fn reply_replay(
    context: &CliContext,
    session: &str,
    incarnation: &str,
    args: &crate::cli::MessageReplyArgs,
    body: &str,
) -> Result<Option<Value>, CliError> {
    let journal = lock_journal(context)?;
    let Some(item) = journal
        .registry
        .find_key(session, incarnation, &args.idempotency_key)
    else {
        return Ok(None);
    };
    let submission = Submit {
        to_machine: item.recipient.machine.clone(),
        to_session: item.recipient.session_id.clone(),
        body: body.into(),
        idempotency_key: args.idempotency_key.clone(),
        reply_to: Some(args.message.clone()),
        expires_in: None,
        reply_revision: Some(args.if_revision),
        expected_recipient_incarnation: None,
        category: args.category,
        forward: None,
    };
    let digest = digest_bytes(&serde_json::to_vec(&submission).map_err(|_| invalid())?);
    if digest != item.request_digest {
        return Err(CliError::data(
            "idempotency-key-conflict",
            "idempotency key has different content",
            None,
        ));
    }
    Ok(Some(projection(&item)))
}

fn receive_replay(
    registry: &super::Registry,
    envelope: &Envelope<Origin>,
    digest: &str,
) -> Result<Option<Value>, CliError> {
    super::idempotency_replay(
        registry,
        &envelope.message_id,
        RECEIVE_PRINCIPAL,
        RECEIVE_INCARNATION,
        RECEIVE_OPERATION,
        digest,
    )
    .map_err(|_| {
        CliError::data(
            "idempotency-key-conflict",
            "message ID has different content",
            None,
        )
    })
}

#[cfg(test)]
#[path = "remote_inbox_regressions.rs"]
mod inbox_regressions;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    const TOKEN: &str = "fixture-private-session-capability-0000000000001";
    pub(super) fn fixture() -> (tempfile::TempDir, CliContext) {
        let temp = tempfile::TempDir::new().expect("private temp root");
        let context = CliContext {
            state_dir: temp.path().to_path_buf(),
            host: None,
        };
        let directory = context.state_dir.join("sessions/recipient/coordination");
        fs::create_dir_all(&directory).expect("session directories");
        for path in [
            context.state_dir.join("sessions"),
            context.state_dir.join("sessions/recipient"),
            directory.clone(),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .expect("private directory");
        }
        let record = json!({"schema_version":"agent-session.session.v1","id":"recipient","agent":"codex","mode":"interactive","title":null,"title_revision":0,"cwd":temp.path(),"tmux_session":"fixture","prompt_file":null,"log_file":null,"created_at":"2030-01-01T00:00:00Z","updated_at":"2030-01-01T00:00:00Z","coordination_mode":"advisory","runtime":{"kind":"tmux","tmux_session":"fixture","generation":1,"started_at":"2030-01-01T00:00:00Z","launch_id":"recipient-incarnation"}});
        write_atomic(
            &context.state_dir.join("sessions/recipient/session.json"),
            &serde_json::to_vec(&record).expect("record"),
            SECRET_FILE_MODE,
        )
        .expect("private record");
        write_atomic(
            &super::super::capability_path(&context, "recipient", "recipient-incarnation"),
            TOKEN.as_bytes(),
            SECRET_FILE_MODE,
        )
        .expect("capability");
        write_atomic(
            &directory.join("heartbeat"),
            format!("recipient-incarnation:{}\n", now_epoch()).as_bytes(),
            SECRET_FILE_MODE,
        )
        .expect("heartbeat");
        let mut locked = lock_registry(&context).expect("registry");
        locked.registry.schema_version = super::super::LEGACY_REGISTRY_VERSION.into();
        super::super::ensure_fingerprint_key(&mut locked.registry);
        locked.registry.brokers.insert(
            "recipient".into(),
            super::super::BrokerRecord {
                session_id: "recipient".into(),
                incarnation: "recipient-incarnation".into(),
                coordination_mode: crate::cli::CoordinationMode::Advisory,
                capability_digest: digest_bytes(TOKEN.as_bytes()),
                generation: 1,
                state: "ready".into(),
                heartbeat_at: timestamp(now_epoch()),
                heartbeat_epoch: now_epoch(),
                runtime_identity: None,
                runtime_identity_digest: String::new(),
                lost_since_epoch: None,
                binary_version: None,
            },
        );
        locked.save().expect("persist fixture");
        drop(locked);
        (temp, context)
    }
    pub(super) fn envelope() -> Envelope {
        Envelope {
            category: None,
            forwarding: None,
            schema_version: ENVELOPE_VERSION.into(),
            message_id: uuid::Uuid::new_v4().to_string(),
            from: Address {
                machine: "source".into(),
                session_id: "recipient".into(),
                session_incarnation: "foreign-incarnation".into(),
            },
            to: Address {
                machine: "destination".into(),
                session_id: "recipient".into(),
                session_incarnation: "recipient-incarnation".into(),
            },
            body: "bounded fixture body".into(),
            body_sha256: digest_bytes(b"bounded fixture body"),
            created_at_epoch: now_epoch(),
            expires_at_epoch: now_epoch() + 60,
            reply_to: None,
            reply_depth: 0,
        }
    }
    fn config() -> Config {
        Config {
            machine: "destination".into(),
            url: "http://127.0.0.1:1".into(),
            token: "relay-fixture-0000000000000000000000001".into(),
            ingress_token: "ingress-fixture-0000000000000000000001".into(),
        }
    }
    #[test]
    fn tagged_remote_ingress_requires_v2_and_keeps_legacy_wire_shape() {
        let (_temp, context) = fixture();
        let untagged = envelope();
        let value = serde_json::to_value(&untagged).unwrap();
        assert!(value.get("category").is_none());
        assert!(value.get("forwarding").is_none());
        let mut tagged = untagged.clone();
        tagged.category = Some(crate::cli::MessageCategory::Handoff);
        assert_eq!(
            receive(&context, "destination", tagged.clone())
                .unwrap_err()
                .code(),
            "remote-message-invalid"
        );
        tagged.schema_version = ENVELOPE_V2.into();
        let receipt = receive(&context, "destination", tagged.clone()).unwrap();
        assert_eq!(receipt["state"], "delivered");
        let locked = lock_registry(&context).unwrap();
        let saved = locked
            .registry
            .messages
            .iter()
            .find(|message| message.message_id == tagged.message_id)
            .unwrap();
        assert_eq!(saved.category, Some(crate::cli::MessageCategory::Handoff));
        assert_eq!(saved.body, "bounded fixture body");
    }

    #[test]
    fn terminal_retry_after_unconfirmed_attempt_is_unknown_but_first_rejection_is_known() {
        use std::io::{Read, Write};
        for (attempts, expected) in [(0, "rejected"), (1, "delivery-unknown")] {
            let (_temp, context) = fixture();
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let mut config = config();
            config.url = format!("http://{}", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut bytes = [0; 1024];
                    let count = stream.read(&mut bytes).unwrap();
                    assert!(count > 0, "request must finish before EOF");
                    request.extend_from_slice(&bytes[..count]);
                    assert!(request.len() <= 8192, "bounded fixture request");
                    if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let header = std::str::from_utf8(&request[..end]).unwrap();
                        let length: usize = header
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let body = r#"{"error":{"code":"session-incarnation-conflict"}}"#;
                write!(
                    stream,
                    "HTTP/1.1 409 Conflict\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            });
            let mut locked = lock_journal(&context).unwrap();
            locked.registry.remote_outbox.push(Outbox {
                envelope: envelope().into_origin(),
                request_digest: "fixture".into(),
                idempotency_key: "fixture".into(),
                state: "queued".into(),
                attempts,
                next_attempt_epoch: now_epoch(),
                last_attempt_at_epoch: None,
                state_changed_at_epoch: None,
                receipt: None,
                reason: Some("remote-messaging-unavailable".into()),
            });
            locked.save().unwrap();
            drop(locked);
            drain(&context, &config).unwrap();
            server.join().unwrap();
            let locked = lock_journal(&context).unwrap();
            assert!(locked.registry.remote_outbox.is_empty());
            assert_eq!(locked.registry.retained[0].state, expected);
            assert_eq!(
                locked.registry.retained[0].reason.as_deref(),
                Some("session-incarnation-conflict")
            );
        }
    }
    #[test]
    fn audit_expiry_without_relay_attempt_preserves_unknown_attempt_time() {
        let (_temp, context) = fixture();
        let mut item = own_outbox("audit-expired", "queued");
        item.envelope.expires_at_epoch = now_epoch() - 1;
        let mut journal = lock_journal(&context).unwrap();
        journal.registry.remote_outbox.push(item);
        journal.save().unwrap();
        drop(journal);
        drain(&context, &config()).unwrap();
        let locked = lock_journal(&context).unwrap();
        let entry = &locked.registry.retained[0];
        assert_eq!(entry.state, "delivery-unknown");
        assert_eq!(entry.last_attempt_at_epoch, None);
        assert!(entry.state_changed_at_epoch.is_some());
    }

    #[test]
    fn audit_terminal_compaction_preserves_known_times_and_prior_unknowns() {
        let (_temp, context) = fixture();
        let mut item = own_outbox("audit-times", "delivered");
        item.last_attempt_at_epoch = Some(101);
        item.state_changed_at_epoch = Some(102);
        let created = item.envelope.created_at_epoch;
        let mut journal = lock_journal(&context).unwrap();
        journal.registry.remote_outbox.push(item);
        journal.save().unwrap();
        drop(journal);
        let locked = lock_journal(&context).unwrap();
        assert_eq!(locked.registry.schema_version, AUDIT_JOURNAL_VERSION);
        let retained = &locked.registry.retained[0];
        assert_eq!(retained.created_at_epoch, Some(created));
        assert_eq!(retained.last_attempt_at_epoch, Some(101));
        assert_eq!(retained.state_changed_at_epoch, Some(102));
        let mut prior = serde_json::to_value(retained).unwrap();
        for key in [
            "created_at_epoch",
            "last_attempt_at_epoch",
            "state_changed_at_epoch",
        ] {
            prior.as_object_mut().unwrap().remove(key);
        }
        let prior: Retained = serde_json::from_value(prior).unwrap();
        assert_eq!(prior.created_at_epoch, None);
        assert_eq!(prior.last_attempt_at_epoch, None);
        assert_eq!(prior.state_changed_at_epoch, None);
    }

    #[test]
    fn federation_config_credential_and_transport_matrix() {
        use nils_test_support::{EnvGuard, GlobalStateLock};
        let lock = GlobalStateLock::new();
        let token = "a".repeat(32);
        let ingress = "b".repeat(32);
        let cases = [
            ("", "", "", true),
            ("https://relay.example", "", "", false),
            ("", token.as_str(), ingress.as_str(), false),
            (
                "http://relay.example",
                token.as_str(),
                ingress.as_str(),
                false,
            ),
            ("ftp://127.0.0.1", token.as_str(), ingress.as_str(), false),
            (
                "https://user:password@relay.example",
                token.as_str(),
                ingress.as_str(),
                false,
            ),
            (
                "https://relay.example?x=1",
                token.as_str(),
                ingress.as_str(),
                false,
            ),
            (
                "https://relay.example#fragment",
                token.as_str(),
                ingress.as_str(),
                false,
            ),
            (
                "https://relay.example",
                &token[..31],
                ingress.as_str(),
                false,
            ),
            (
                "https://relay.example",
                token.as_str(),
                &ingress[..31],
                false,
            ),
            (
                "https://relay.example",
                token.as_str(),
                token.as_str(),
                false,
            ),
            (
                "https://relay.example",
                token.as_str(),
                ingress.as_str(),
                true,
            ),
            (
                "http://localhost:8000",
                token.as_str(),
                ingress.as_str(),
                true,
            ),
            (
                "http://127.0.0.1:8000",
                token.as_str(),
                ingress.as_str(),
                true,
            ),
            ("http://[::1]:8000", token.as_str(), ingress.as_str(), true),
        ];
        for (url, token, ingress, valid) in cases {
            let _url = EnvGuard::set(&lock, "AGENT_SESSION_RELAY_URL", url);
            let _token = EnvGuard::set(&lock, "AGENT_SESSION_RELAY_TOKEN", token);
            let _ingress = EnvGuard::set(&lock, "AGENT_SESSION_RELAY_INGRESS_TOKEN", ingress);
            assert_eq!(
                Config::from_environment("fixture").is_ok(),
                valid,
                "transport: {url}"
            );
        }
    }
    #[test]
    fn shared_admission_rejects_global_quota_before_write() {
        let (_temp, context) = fixture();
        receive(&context, "destination", envelope()).unwrap();
        let mut locked = lock_registry(&context).unwrap();
        locked.registry.messages[0].recipient_session_id = "other-session".into();
        // Synthetic accounting avoids allocating 68MiB to exercise the typed policy.
        locked.registry.messages[0].body_bytes = super::super::MAX_REGISTRY_BYTES as usize;
        assert_eq!(
            super::super::mailbox::admit_message(
                &mut locked.registry,
                "other-sender",
                "recipient",
                1,
                None,
                now_epoch(),
                super::super::mailbox::now_epoch_millis()
            )
            .unwrap_err()
            .details()
            .map(|details| (details["quota"].clone(), details["side"].clone())),
            Some((json!("registry-message-bytes"), json!("local")))
        );
    }
    #[test]
    fn empty_journal_worker_does_not_read_or_maintain_registry() {
        let (_temp, context) = fixture();
        write_atomic(
            &context.state_dir.join("coordination/registry.json"),
            b"invalid registry",
            SECRET_FILE_MODE,
        )
        .unwrap();
        drain(&context, &config()).expect("empty journal is independent of registry");
        assert_eq!(
            fs::read(context.state_dir.join("coordination/registry.json")).unwrap(),
            b"invalid registry"
        );
    }
    #[test]
    fn retained_receipt_precedes_expiry_and_replaced_recipient() {
        let (_temp, context) = fixture();
        let mut message = envelope();
        message.expires_at_epoch = now_epoch() + 1;
        let receipt = receive(&context, "destination", message.clone()).expect("receive");
        std::thread::sleep(Duration::from_secs(2));
        let path = context.state_dir.join("sessions/recipient/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["runtime"]["launch_id"] = json!("replacement");
        write_atomic(
            &path,
            &serde_json::to_vec(&record).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        assert_eq!(
            receive(&context, "destination", message.clone()).unwrap(),
            receipt
        );
        message.body = "changed".into();
        message.body_sha256 = digest_bytes(message.body.as_bytes());
        assert_eq!(
            receive(&context, "destination", message)
                .unwrap_err()
                .code(),
            "idempotency-key-conflict"
        );
    }
    #[test]
    fn remote_reply_depth_limit_rejects_before_enqueue() {
        let (_temp, context) = fixture();
        let mut parent = envelope();
        parent.reply_depth = super::super::mailbox::MAX_REPLY_DEPTH;
        let parent_id = parent.message_id.clone();
        receive(&context, "destination", parent).unwrap();
        let args = Submit {
            category: None,
            forward: None,

            to_machine: "source".into(),
            to_session: "recipient".into(),
            body: "bounded reply".into(),
            idempotency_key: "depth-key-0001".into(),
            reply_to: Some(parent_id),
            expires_in: None,
            reply_revision: Some(1),
            expected_recipient_incarnation: None,
        };
        assert_eq!(
            submit(&context, &config(), "recipient", TOKEN, args)
                .unwrap_err()
                .code(),
            "remote-message-invalid"
        );
        assert!(
            lock_journal(&context)
                .unwrap()
                .registry
                .remote_outbox
                .is_empty()
        );
    }
    #[test]
    fn terminal_only_journal_returns_no_deadline_without_registry_read() {
        let (_temp, context) = fixture();
        let mut journal = lock_journal(&context).unwrap();
        journal.registry.remote_outbox.push(Outbox {
            envelope: envelope().into_origin(),
            request_digest: "fixture".into(),
            idempotency_key: "fixture".into(),
            state: "delivered".into(),
            attempts: 1,
            next_attempt_epoch: now_epoch(),
            last_attempt_at_epoch: None,
            state_changed_at_epoch: None,
            receipt: None,
            reason: None,
        });
        journal.save().unwrap();
        drop(journal);
        write_atomic(
            &context.state_dir.join("coordination/registry.json"),
            b"invalid registry",
            SECRET_FILE_MODE,
        )
        .unwrap();
        assert_eq!(drain(&context, &config()).unwrap(), None);
    }
    #[test]
    fn retained_receipt_precedes_replacement_even_before_expiry() {
        let (_temp, context) = fixture();
        let message = envelope();
        let receipt = receive(&context, "destination", message.clone()).unwrap();
        let path = context.state_dir.join("sessions/recipient/session.json");
        let mut record: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        record["runtime"]["launch_id"] = json!("replacement");
        write_atomic(
            &path,
            &serde_json::to_vec(&record).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
        assert_eq!(receive(&context, "destination", message).unwrap(), receipt);
    }
    #[test]
    fn remote_burst_admission_refuses_eleventh_before_persistence() {
        let (_temp, context) = fixture();
        for _ in 0..10 {
            receive(&context, "destination", envelope()).unwrap();
        }
        assert_eq!(
            receive(&context, "destination", envelope())
                .unwrap_err()
                .code(),
            "rate-limited"
        );
        assert_eq!(lock_registry(&context).unwrap().registry.messages.len(), 10);
    }
    #[test]
    fn remote_receive_persists_existing_origin_and_receipt_atomically() {
        let (_temp, context) = fixture();
        assert_eq!(
            lock_registry(&context)
                .expect("pre-remote")
                .registry
                .schema_version,
            super::super::LEGACY_REGISTRY_VERSION
        );
        let message = envelope();
        let id = message.message_id.clone();
        let receipt = receive(&context, "destination", message.clone()).expect("receive");
        let replay = receive(&context, "destination", message.clone())
            .expect("replay after registry disk reload");
        assert_eq!(receipt, replay);
        let mut locked = lock_registry(&context).expect("reloaded");
        assert_eq!(
            locked.registry.schema_version,
            super::super::LEGACY_REGISTRY_VERSION
        );
        assert_eq!(locked.registry.messages.len(), 1);
        assert_eq!(
            sender_address(&locked.registry.messages[0])
                .expect("remote origin")
                .machine,
            "source"
        );
        assert_eq!(
            locked.registry.messages[0].sender_incarnation,
            "foreign-incarnation"
        );
        assert_eq!(
            locked.registry.receipts[&super::super::receipt_key(
                RECEIVE_PRINCIPAL,
                RECEIVE_INCARNATION,
                RECEIVE_OPERATION,
                &id
            )]
                .outcome["state"],
            "delivered"
        );
        assert_eq!(locked.registry.messages[0].state, "unread");
        locked
            .save()
            .expect("local rewrite preserves existing schema");
        drop(locked);
        super::super::lock_registry_observational(&context).expect("existing registry supported");
        let projection = nils_common::coordination_projection::load(&context.state_dir);
        assert!(
            projection.is_ok(),
            "shared facade must read existing registry: {projection:?}"
        );
        let mut changed = message;
        changed.body = "changed".into();
        changed.body_sha256 = digest_bytes(changed.body.as_bytes());
        assert_eq!(
            receive(&context, "destination", changed)
                .expect_err("different body same ID")
                .code(),
            "idempotency-key-conflict"
        );
    }
    #[test]
    fn remote_receive_rejects_expiry_digest_and_replaced_target_before_mutation() {
        let (_temp, context) = fixture();
        let mut invalid_digest = envelope();
        invalid_digest.body_sha256 = "wrong".into();
        let mut expired = envelope();
        expired.expires_at_epoch = now_epoch() - 1;
        let mut wrong_schema = envelope();
        wrong_schema.schema_version = "agent-session.remote-message.v99".into();
        for bad in [invalid_digest, expired, wrong_schema] {
            assert_eq!(
                receive(&context, "destination", bad)
                    .expect_err("invalid envelope")
                    .code(),
                "remote-message-invalid"
            );
        }
        let mut replacement = envelope();
        replacement.to.session_incarnation = "replacement".into();
        assert_eq!(
            receive(&context, "destination", replacement)
                .expect_err("wrong incarnation")
                .code(),
            "session-incarnation-conflict"
        );
        let locked = lock_registry(&context).expect("unchanged registry");
        assert!(locked.registry.messages.is_empty());
        assert_eq!(
            locked.registry.schema_version,
            super::super::LEGACY_REGISTRY_VERSION
        );
    }
    #[test]
    fn outbox_replay_never_rediscovers_or_retargets_and_capability_is_required() {
        let (_temp, context) = fixture();
        let args = Submit {
            category: None,
            forward: None,

            to_machine: "other".into(),
            to_session: "other-agent".into(),
            body: "fixture body".into(),
            idempotency_key: "fixture-key-0001".into(),
            reply_to: None,
            expires_in: None,
            reply_revision: None,
            expected_recipient_incarnation: None,
        };
        let mut message = envelope();
        message.from = Address {
            machine: "destination".into(),
            session_id: "recipient".into(),
            session_incarnation: "recipient-incarnation".into(),
        };
        message.to = Address {
            machine: "other".into(),
            session_id: "other-agent".into(),
            session_incarnation: "original-target".into(),
        };
        let mut locked = lock_journal(&context).expect("journal");
        locked.registry.remote_outbox.push(Outbox {
            envelope: message.into_origin(),
            request_digest: digest_bytes(&serde_json::to_vec(&args).expect("digest")),
            idempotency_key: args.idempotency_key.clone(),
            state: "queued".into(),
            attempts: 0,
            next_attempt_epoch: now_epoch(),
            last_attempt_at_epoch: None,
            state_changed_at_epoch: None,
            receipt: None,
            reason: None,
        });
        locked.save().expect("save queued");
        drop(locked);
        let replay = submit(&context, &config(), "recipient", TOKEN, args.clone())
            .expect("must not contact unreachable discovery");
        let journal_path = context.state_dir.join("coordination").join(JOURNAL_FILE);
        let before = fs::read(&journal_path).unwrap();
        for destination_changed in [false, true] {
            let mut changed = args.clone();
            if destination_changed {
                changed.to_session = "new-target".into();
            } else {
                changed.body = "new-body".into();
            }
            assert_eq!(
                submit(&context, &config(), "recipient", TOKEN, changed)
                    .unwrap_err()
                    .code(),
                "idempotency-key-conflict"
            );
            assert_eq!(fs::read(&journal_path).unwrap(), before);
        }
        assert_eq!(
            replay["recipient"]["session_incarnation"],
            "original-target"
        );
        let spoof = delivery(
            &context,
            "recipient",
            "invalid-token",
            replay["message_id"].as_str().expect("id"),
        );
        assert_eq!(
            spoof.expect_err("spoof rejected").code(),
            "coordination-unauthorized"
        );
    }
    #[test]
    fn pending_expired_outbox_records_unknown_and_keeps_private_body_out_of_status() {
        let (_temp, context) = fixture();
        let mut message = envelope();
        message.from = Address {
            machine: "destination".into(),
            session_id: "recipient".into(),
            session_incarnation: "recipient-incarnation".into(),
        };
        message.expires_at_epoch = now_epoch() - 1;
        let id = message.message_id.clone();
        let mut locked = lock_journal(&context).expect("journal");
        locked.registry.remote_outbox.push(Outbox {
            envelope: message.into_origin(),
            request_digest: "digest".into(),
            idempotency_key: "expiry-key-0001".into(),
            state: "queued".into(),
            attempts: 1,
            next_attempt_epoch: 0,
            last_attempt_at_epoch: None,
            state_changed_at_epoch: None,
            receipt: None,
            reason: None,
        });
        locked.save().expect("save");
        drop(locked);
        drain(&context, &config()).expect("expire without network");
        let value = delivery(&context, "recipient", TOKEN, &id).expect("status");
        assert_eq!(value["state"], "delivery-unknown");
        assert!(!value.to_string().contains("bounded fixture body"));
    }
    #[test]
    fn reserved_sender_is_collision_proof_and_receipt_outlives_existing_ttl() {
        let (_temp, context) = fixture();
        let mut message = envelope();
        message.expires_at_epoch = now_epoch() + 7 * 86400;
        let id = message.message_id.clone();
        let receipt = receive(&context, "destination", message.clone()).expect("receive");
        let mut locked = lock_registry(&context).expect("registry");
        let stored = &locked.registry.messages[0];
        assert!(crate::validate_id(&stored.sender_session_id).is_err());
        assert_eq!(sender_address(stored), Some(message.from.clone()));
        let mut other = message.from.clone();
        other.machine.push_str("-other");
        assert_ne!(encode_sender(&other), stored.sender_session_id);
        assert!(
            sender_address(&super::super::mailbox::StoredMessage {
                category: None,
                forwarding: None,

                sender_session_id: "remote:malformed".into(),
                ..stored.clone()
            })
            .is_none()
        );
        let key = super::super::receipt_key(
            RECEIVE_PRINCIPAL,
            RECEIVE_INCARNATION,
            RECEIVE_OPERATION,
            &id,
        );
        let expiry = locked.registry.receipts[&key].expires_at_epoch;
        assert_eq!(expiry, message.expires_at_epoch + 86400);
        super::super::clean_expired(&mut locked.registry, now_epoch() + 86401);
        assert!(
            locked.registry.receipts.contains_key(&key),
            "existing sweeper must not erase valid remote dedup"
        );
        locked.save().expect("old-compatible rewrite");
        drop(locked);
        assert_eq!(
            receive(&context, "destination", message.clone()).expect("retained replay"),
            receipt
        );
        let mut collision = message.clone();
        collision.from.machine.push_str("-other");
        assert_eq!(
            receive(&context, "destination", collision)
                .expect_err("same ID foreign origin")
                .code(),
            "idempotency-key-conflict"
        );
        let mut locked = lock_registry(&context).expect("registry");
        locked.registry.receipts.clear();
        locked.save().expect("fixture remove receipt");
        drop(locked);
        assert_eq!(
            receive(&context, "destination", message)
                .expect_err("existing inbox ID collision")
                .code(),
            "idempotency-key-conflict"
        );
    }
    #[test]
    fn unsupported_journal_version_is_never_rewritten_by_local_writer() {
        let (_temp, context) = fixture();
        let path = context.state_dir.join("coordination").join(JOURNAL_FILE);
        let bytes =
            br#"{"schema_version":"agent-session.federation-journal.v99","remote_outbox":[]}"#;
        write_atomic(&path, bytes, SECRET_FILE_MODE).expect("fixture journal");
        assert!(lock_journal(&context).is_err());
        let mut locked = lock_registry(&context).expect("existing local writer still available");
        locked.save().expect("local write unaffected");
        drop(locked);
        assert_eq!(fs::read(path).expect("journal retained"), bytes);
        super::super::lock_registry_observational(&context)
            .expect("existing reader still available");
    }
    fn own_outbox(key: &str, state: &str) -> Outbox {
        let mut message = envelope();
        message.from = Address {
            machine: "destination".into(),
            session_id: "recipient".into(),
            session_incarnation: "recipient-incarnation".into(),
        };
        message.to.machine = "source".into();
        message.expires_at_epoch = now_epoch() + 86400;
        let delivered = state == "delivered";
        Outbox {
            receipt: delivered.then(|| json!({"schema_version":DELIVERY_VERSION,"message_id":message.message_id,"state":"delivered","recipient":message.to,"persisted_at_epoch":now_epoch()})),
            envelope: message.into_origin(),
            request_digest: format!("digest-{key}"),
            idempotency_key: key.into(),
            state: state.into(),
            attempts: u64::from(delivered),
            next_attempt_epoch: now_epoch(),
            last_attempt_at_epoch: None,
            state_changed_at_epoch: None,
            reason: None,
        }
    }
    fn write_v1_journal(context: &CliContext, outbox: &[Outbox]) {
        write_atomic(
            &context.state_dir.join("coordination").join(JOURNAL_FILE),
            &serde_json::to_vec(&json!({"schema_version":"agent-session.federation-journal.v1","remote_outbox":outbox})).unwrap(),
            SECRET_FILE_MODE,
        )
        .unwrap();
    }
    /// Receives a remote parent so a reply can be submitted without discovery.
    fn reply_args(context: &CliContext, key: &str) -> Submit {
        let parent = envelope();
        let parent_id = parent.message_id.clone();
        receive(context, "destination", parent).unwrap();
        Submit {
            category: None,
            forward: None,
            expected_recipient_incarnation: None,
            to_machine: "source".into(),
            to_session: "recipient".into(),
            body: "bounded reply".into(),
            idempotency_key: key.into(),
            reply_to: Some(parent_id),
            expires_in: None,
            reply_revision: Some(1),
        }
    }
    #[test]
    fn delivered_outbox_entries_do_not_block_new_sends() {
        let (_temp, context) = fixture();
        let delivered: Vec<_> = (0..256)
            .map(|n| own_outbox(&format!("delivered-key-{n:04}"), "delivered"))
            .collect();
        write_v1_journal(&context, &delivered);
        let sent = submit(
            &context,
            &config(),
            "recipient",
            TOKEN,
            reply_args(&context, "new-key-0001"),
        )
        .expect("delivered envelopes must not block a new send");
        assert_eq!(sent["state"], "queued");
        // Compaction keeps replay, delivery status and conflict detection.
        let first = &delivered[0];
        let status = delivery(&context, "recipient", TOKEN, &first.envelope.message_id)
            .expect("compacted delivery status");
        assert_eq!(status, projection(&first.identity()));
        let path = context.state_dir.join("coordination").join(JOURNAL_FILE);
        let journal = String::from_utf8(fs::read(&path).unwrap()).unwrap();
        assert!(
            !journal.contains("bounded fixture body"),
            "terminal bodies are dropped from the journal"
        );
        let mut replay = reply_args(&context, "delivered-key-0000");
        replay.reply_to = None;
        assert_eq!(
            submit(&context, &config(), "recipient", TOKEN, replay)
                .unwrap_err()
                .code(),
            "idempotency-key-conflict",
            "a compacted key still refuses changed content"
        );
    }
    fn own_queued(machine: &str, keys: std::ops::Range<usize>) -> Vec<Outbox> {
        keys.map(|n| {
            let mut item = own_outbox(&format!("{machine}-key-{n:04}"), "queued");
            item.envelope.to.machine = machine.into();
            item
        })
        .collect()
    }
    #[test]
    fn pending_quota_is_per_destination_and_names_source_counts() {
        let (_temp, context) = fixture();
        let mut locked = lock_journal(&context).unwrap();
        locked.registry.remote_outbox = own_queued("asleep", 0..MAX_PENDING_PER_DESTINATION);
        locked.registry.retained = vec![own_outbox("delivered-key", "delivered").identity()];
        locked.save().unwrap();
        drop(locked);
        submit(
            &context,
            &config(),
            "recipient",
            TOKEN,
            reply_args(&context, "unrelated-key-0001"),
        )
        .expect("an asleep destination must not block other destinations");
        let mut locked = lock_journal(&context).unwrap();
        let more = own_queued("source", 1..MAX_PENDING_PER_DESTINATION);
        locked.registry.remote_outbox.extend(more);
        locked.save().unwrap();
        drop(locked);
        let error = submit(
            &context,
            &config(),
            "recipient",
            TOKEN,
            reply_args(&context, "new-key-0001"),
        )
        .unwrap_err();
        assert_eq!(error.code(), "quota-exceeded");
        assert_eq!(
            error.message(),
            "local federation outbox has too many undelivered messages for this destination (federation-pending-destination 512/512)"
        );
        assert_eq!(
            error.details(),
            Some(&json!({
                "quota": "federation-pending-destination",
                "count": 512,
                "limit": 512,
                "host": "destination",
                "side": "source",
                "destination_machine": "source",
                "pending": 1024,
                "pending_to_destination": 512,
                "delivered": 1,
                "retained": 1,
            }))
        );
    }
    #[test]
    fn retained_identity_quota_bounds_compacted_records_separately() {
        let (_temp, context) = fixture();
        let args = reply_args(&context, "new-key-0001");
        let mut locked = lock_journal(&context).unwrap();
        locked.registry.retained = (0..MAX_RETAINED_IDS)
            .map(|n| own_outbox(&format!("retained-key-{n:05}"), "delivered").identity())
            .collect();
        locked.save().unwrap();
        drop(locked);
        let error = submit(&context, &config(), "recipient", TOKEN, args).unwrap_err();
        assert_eq!(
            error.message(),
            "local federation outbox retained-ID limit reached (federation-retained-ids 16384/16384)"
        );
        assert_eq!(error.details().unwrap()["quota"], "federation-retained-ids");
        assert_eq!(error.details().unwrap()["delivered"], 16384);
        assert_eq!(error.details().unwrap()["pending"], 0);
        // The retained-ID cap fits well inside the journal budget.
        let path = context.state_dir.join("coordination").join(JOURNAL_FILE);
        assert!(fs::metadata(path).unwrap().len() < MAX_JOURNAL_BYTES / 2);
    }
    #[test]
    fn journal_byte_budget_refuses_with_quota_before_write() {
        let (_temp, context) = fixture();
        let args = reply_args(&context, "new-key-0001");
        let mut locked = lock_journal(&context).unwrap();
        // Synthetic oversized bodies reach the byte budget below the count caps.
        locked.registry.remote_outbox = own_queued("source", 0..2);
        for item in &mut locked.registry.remote_outbox {
            item.envelope.body = "x".repeat((MAX_JOURNAL_BYTES as usize - 64 * 1024) / 2);
        }
        locked.save().unwrap();
        drop(locked);
        let path = context.state_dir.join("coordination").join(JOURNAL_FILE);
        let before = fs::read(&path).unwrap();
        let error = submit(&context, &config(), "recipient", TOKEN, args).unwrap_err();
        assert_eq!(
            error.details().unwrap()["quota"],
            "federation-journal-bytes"
        );
        assert_eq!(error.details().unwrap()["side"], "source");
        assert_eq!(error.details().unwrap()["pending"], 2);
        assert!(fs::read(&path).unwrap() == before, "refusal must not write");
    }
    #[test]
    fn unreachable_destination_backs_off_only_its_own_envelopes() {
        let (_temp, context) = fixture();
        let mut locked = lock_journal(&context).unwrap();
        locked.registry.remote_outbox = own_queued("asleep", 0..2);
        locked
            .registry
            .remote_outbox
            .extend(own_queued("awake", 0..1));
        // A healthy session on the same machine as the unavailable one.
        let mut healthy = own_queued("asleep", 2..3);
        healthy[0].envelope.to.session_id = "healthy".into();
        locked.registry.remote_outbox.extend(healthy);
        let due = locked.registry.remote_outbox[0].next_attempt_epoch;
        for item in &mut locked.registry.remote_outbox {
            item.attempts = 1;
        }
        locked.registry.remote_outbox[2].next_attempt_epoch = due + 1000;
        locked.save().unwrap();
        drop(locked);
        // Port 1 refuses the connection: a retryable transport failure.
        drain(&context, &config()).unwrap();
        let locked = lock_journal(&context).unwrap();
        let outbox = &locked.registry.remote_outbox;
        assert_eq!(
            outbox[0].reason.as_deref(),
            Some("remote-messaging-unavailable")
        );
        assert_eq!(outbox[1].attempts, 1, "only one probe per destination");
        assert!(
            outbox[1].next_attempt_epoch > due,
            "same destination backs off"
        );
        assert_eq!(
            outbox[2].next_attempt_epoch,
            due + 1000,
            "other machine is untouched"
        );
        assert_eq!(
            outbox[3].next_attempt_epoch, due,
            "another session on the same machine stays due"
        );
    }
    #[test]
    fn destination_mailbox_quota_names_recipient_quota_and_destination_host() {
        let (_temp, context) = fixture();
        receive(&context, "destination", envelope()).unwrap();
        let mut locked = lock_registry(&context).unwrap();
        let stored = locked.registry.messages[0].clone();
        for _ in 1..256 {
            locked
                .registry
                .messages
                .push(super::super::mailbox::StoredMessage {
                    category: None,
                    forwarding: None,

                    message_id: uuid::Uuid::new_v4().to_string(),
                    created_at_epoch: stored.created_at_epoch - 3600,
                    created_at_epoch_millis: 0,
                    ..stored.clone()
                });
        }
        locked.save().unwrap();
        drop(locked);
        let error = receive(&context, "destination", envelope()).unwrap_err();
        assert_eq!(error.code(), "quota-exceeded");
        assert_eq!(
            error.details(),
            Some(
                &json!({"quota":"recipient-messages","count":256,"limit":256,"host":"destination","side":"destination"})
            )
        );
    }
    #[test]
    fn local_daemon_refusal_keeps_diagnostics_and_relay_keeps_only_quota_fields() {
        let refusal = json!({"ok":false,"error":{"code":"quota-exceeded","message":"federation outbox has too many undelivered messages","details":{"quota":"federation-pending","count":256,"limit":256,"host":"sympoies","side":"source"}}});
        let local = local_daemon_error(&refusal);
        assert_eq!(local.code(), "quota-exceeded");
        assert_eq!(
            local.message(),
            "federation outbox has too many undelivered messages"
        );
        assert_eq!(local.details(), refusal.pointer("/error/details"));
        let mut remote = refusal.clone();
        remote["error"]["details"]["body"] = json!("private body");
        remote["error"]["details"]["side"] = json!("destination");
        let relayed = relay_error(&remote);
        assert_eq!(relayed.message(), "remote mailbox request was rejected");
        assert_eq!(
            relayed.details(),
            Some(
                &json!({"quota":"federation-pending","count":256,"limit":256,"host":"sympoies","side":"destination"})
            )
        );
        let unknown = json!({"error":{"code":"internal-failure","details":{"quota":"x"}}});
        assert_eq!(relay_error(&unknown).code(), "remote-messaging-unavailable");
        assert_eq!(relay_error(&unknown).details(), None);
        assert_eq!(
            local_daemon_error(&json!({"error":{"code":"Bad Code"}})).code(),
            "remote-messaging-unavailable"
        );
    }
    #[test]
    fn service_failed_deliveries_retain_bodies_without_expanding_existing_bounds() {
        for state in ["rejected", "delivery-unknown", "delivered"] {
            let managed = Outbox {
                envelope: envelope().into_origin(),
                request_digest: "digest".into(),
                idempotency_key: "key".into(),
                state: state.into(),
                attempts: 1,
                next_attempt_epoch: 0,
                last_attempt_at_epoch: None,
                state_changed_at_epoch: None,
                receipt: None,
                reason: None,
            };
            let mut service = managed.clone();
            service.envelope.schema_version = super::super::service::WIRE_VERSION.into();
            service.envelope.from = Origin::Service(super::super::service::Origin {
                machine: "source".into(),
                service_id: "reporter".into(),
                service_generation: "generation-1".into(),
            });
            let expected_body = service.envelope.body.clone();
            let expiry = service.envelope.expires_at_epoch;
            let mut journal = Journal {
                schema_version: JOURNAL_VERSION.into(),
                remote_outbox: vec![managed, service],
                retained: vec![],
            };
            journal.compact(now_epoch());
            if state == "delivered" {
                assert_eq!(journal.remote_outbox.len(), 0);
                assert_eq!(journal.retained.len(), 2);
            } else {
                assert_eq!(
                    journal.remote_outbox.len(),
                    1,
                    "failed service content must remain recoverable"
                );
                assert_eq!(journal.remote_outbox[0].envelope.body, expected_body);
                assert_eq!(
                    journal.retained.len(),
                    1,
                    "managed terminal compaction stays unchanged"
                );
            }
            journal.compact(expiry);
            assert_eq!(
                journal.remote_outbox.len(),
                0,
                "failed content expires at the original deadline"
            );
            assert_eq!(journal.retained.len(), 2);
            journal.compact(expiry + 86400);
            assert_eq!(
                journal.remote_outbox.len(),
                0,
                "existing retention deadline remains bounded"
            );
            assert_eq!(journal.retained.len(), 0);
        }
    }

    #[test]
    fn service_origin_extension_preserves_managed_envelope_wire_bytes() {
        let session_envelope = envelope();
        let bytes = serde_json::to_vec(&session_envelope).unwrap();
        assert_eq!(
            bytes,
            serde_json::to_vec(&session_envelope.into_origin()).unwrap()
        );
    }

    #[test]
    fn service_failure_expiry_uses_the_existing_daemon_drain_deadline() {
        let (_temp, context) = fixture();
        let mut envelope = envelope().into_origin();
        envelope.schema_version = super::super::service::WIRE_VERSION.into();
        envelope.from = Origin::Service(super::super::service::Origin {
            machine: "source".into(),
            service_id: "reporter".into(),
            service_generation: "generation-1".into(),
        });
        let expiry = envelope.expires_at_epoch;
        {
            let mut locked = lock_journal(&context).unwrap();
            locked.registry.remote_outbox.push(Outbox {
                envelope,
                request_digest: "digest".into(),
                idempotency_key: "key".into(),
                state: "delivery-unknown".into(),
                attempts: 1,
                next_attempt_epoch: 0,
                last_attempt_at_epoch: None,
                state_changed_at_epoch: None,
                receipt: None,
                reason: Some("session-incarnation-conflict".into()),
            });
            locked.save().unwrap();
        }
        assert_eq!(
            drain(&context, &config()).unwrap(),
            Some(expiry),
            "owner must wake to expire failed service content without new submissions"
        );
        {
            let mut locked = lock_journal(&context).unwrap();
            assert_eq!(locked.registry.remote_outbox.len(), 1);
            locked.registry.remote_outbox[0].envelope.expires_at_epoch = now_epoch() - 1;
            // Simulate time passing on disk without invoking save's compaction.
            std::fs::write(&locked.path, serde_json::to_vec(&locked.registry).unwrap()).unwrap();
        }
        assert_eq!(drain(&context, &config()).unwrap(), None);
        let locked = lock_journal(&context).unwrap();
        assert_eq!(
            locked.registry.remote_outbox.len(),
            0,
            "body removed at expiry"
        );
        assert_eq!(
            locked.registry.retained.len(),
            1,
            "body-free dedup identity retained"
        );
    }
}

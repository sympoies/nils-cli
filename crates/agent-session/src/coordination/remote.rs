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

const JOURNAL_VERSION: &str = "agent-session.federation-journal.v1";
const JOURNAL_FILE: &str = "federation-journal.json";
const MAX_JOURNAL_BYTES: u64 = 8 * 1024 * 1024;
const RECEIVE_PRINCIPAL: &str = "remote:receive";
const RECEIVE_INCARNATION: &str = "v1";
const RECEIVE_OPERATION: &str = "remote-message-receive";
const ENVELOPE_VERSION: &str = "agent-session.remote-message.v1";
const DELIVERY_VERSION: &str = "agent-session.remote-delivery.v1";
const MAX_OUTBOX: usize = 256;

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
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub schema_version: String,
    pub message_id: String,
    pub from: Address,
    pub to: Address,
    pub body: String,
    pub body_sha256: String,
    pub created_at_epoch: i64,
    pub expires_at_epoch: i64,
    pub reply_to: Option<String>,
    pub reply_depth: u8,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Outbox {
    pub envelope: Envelope,
    pub request_digest: String,
    pub idempotency_key: String,
    pub state: String,
    pub attempts: u64,
    pub next_attempt_epoch: i64,
    pub receipt: Option<Value>,
    pub reason: Option<String>,
}
// Independent source state; existing registry writers never deserialize or rewrite it.
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: String,
    remote_outbox: Vec<Outbox>,
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
            if journal.schema_version != JOURNAL_VERSION || journal.remote_outbox.len() > MAX_OUTBOX
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
    fn save(&self) -> Result<(), CliError> {
        let bytes = serde_json::to_vec(&self.registry).map_err(|_| journal_error())?;
        if bytes.len() as u64 > MAX_JOURNAL_BYTES {
            return Err(journal_error());
        }
        write_atomic(&self.path, &bytes, SECRET_FILE_MODE).map_err(|_| journal_error())
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
    let encoded = message.sender_session_id.strip_prefix(SENDER_PREFIX)?;
    let (machine, session_id): (String, String) = serde_json::from_str(encoded).ok()?;
    let address = Address {
        machine,
        session_id,
        session_incarnation: message.sender_incarnation.clone(),
    };
    (encode_sender(&address) == message.sender_session_id).then_some(address)
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
fn response_json(response: reqwest::blocking::Response) -> Result<Value, CliError> {
    let status = response.status();
    let bytes = response.bytes().map_err(|_| unavailable())?;
    if bytes.len() > 1024 * 1024 {
        return Err(unavailable());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
    if !status.is_success() {
        let code = value
            .pointer("/error/code")
            .and_then(Value::as_str)
            .unwrap_or("remote-messaging-unavailable");
        let code = match code {
            "unauthorized"
            | "ownership-unknown"
            | "origin-forbidden"
            | "principal-forbidden"
            | "machine-forbidden"
            | "federation-disabled"
            | "coordination-unauthorized"
            | "ownership-not-found"
            | "ownership-mismatch"
            | "session-incarnation-conflict"
            | "message-not-found"
            | "remote-messaging-unsupported"
            | "remote-message-invalid"
            | "quota-exceeded"
            | "rate-limited"
            | "idempotency-key-conflict"
            | "message-expired" => code,
            _ => "remote-messaging-unavailable",
        };
        return Err(CliError::data(
            code,
            "remote mailbox request was rejected",
            None,
        ));
    }
    Ok(value)
}
pub(crate) fn peers(
    context: &CliContext,
    config: &Config,
    session: &str,
    token: &str,
) -> Result<Value, CliError> {
    let (_, current) = authenticate_token(context, session, token)?;
    let mut url = reqwest::Url::parse(&format!("{}/api/coordination/peers/v1", config.url))
        .map_err(|_| unavailable())?;
    url.query_pairs_mut()
        .append_pair("source_session_id", session)
        .append_pair("source_incarnation", &current);
    let response = client()?
        .get(url)
        .bearer_auth(&config.token)
        .send()
        .map_err(|_| unavailable())?;
    let value = response_json(response)?;
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
    super::validate_idempotency_key(&args.idempotency_key)?;
    if args.body.is_empty()
        || args.body.len() > 16 * 1024
        || args.idempotency_key.is_empty()
        || args.idempotency_key.len() > 256
    {
        return Err(invalid());
    }
    let digest = digest_bytes(&serde_json::to_vec(&args).map_err(|_| invalid())?);
    // Replay before discovery: a retry must retain its original destination incarnation.
    {
        let locked = lock_journal(context)?;
        if let Some(item) = locked.registry.remote_outbox.iter().find(|item| {
            item.envelope.from.session_id == session
                && item.envelope.from.session_incarnation == source_incarnation
                && item.idempotency_key == args.idempotency_key
        }) {
            if item.request_digest != digest {
                return Err(CliError::data(
                    "idempotency-key-conflict",
                    "idempotency key has different content",
                    None,
                ));
            }
            return Ok(projection(item));
        }
    }
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
        let discovered = peers(context, config, session, token)?;
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
    let expiry = super::mailbox::parse_expiry(args.expires_in.as_deref())?;
    let now = now_epoch();
    let envelope = Envelope {
        schema_version: ENVELOPE_VERSION.into(),
        message_id: uuid::Uuid::new_v4().to_string(),
        from: Address {
            machine: config.machine.clone(),
            session_id: session.into(),
            session_incarnation: source_incarnation.clone(),
        },
        to: destination,
        body_sha256: digest_bytes(args.body.as_bytes()),
        body: args.body,
        created_at_epoch: now,
        expires_at_epoch: now.saturating_add(expiry),
        reply_to: args.reply_to,
        reply_depth,
    };
    // Authenticate again after discovery, then persist without any network-held locks.
    let _source_lock = crate::acquire_session_record_lock(context, session)?;
    let (_, current) = authenticate_token(context, session, token)?;
    if current != source_incarnation {
        return Err(super::unauthorized());
    }
    let locked = lock_registry(context)?;
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
    let _registry_guard = locked;
    let mut locked = lock_journal(context)?;
    if let Some(item) = locked.registry.remote_outbox.iter().find(|item| {
        item.envelope.from.session_id == session
            && item.envelope.from.session_incarnation == source_incarnation
            && item.idempotency_key == args.idempotency_key
    }) {
        if item.request_digest != digest {
            return Err(CliError::data(
                "idempotency-key-conflict",
                "idempotency key has different content",
                None,
            ));
        }
        return Ok(projection(item));
    }
    // Expired entries retain an explicit unknown result; bounded capacity rejects, never evicts live IDs.
    locked
        .registry
        .remote_outbox
        .retain(|i| i.envelope.expires_at_epoch.saturating_add(86400) > now);
    if locked.registry.remote_outbox.len() >= MAX_OUTBOX {
        return Err(CliError::data(
            "quota-exceeded",
            "remote outbox is full",
            None,
        ));
    }
    let item = Outbox {
        envelope,
        request_digest: digest,
        idempotency_key: args.idempotency_key,
        state: "queued".into(),
        attempts: 0,
        next_attempt_epoch: now,
        receipt: None,
        reason: None,
    };
    let outcome = projection(&item);
    locked.registry.remote_outbox.push(item);
    locked.save()?;
    Ok(outcome)
}
fn projection(item: &Outbox) -> Value {
    json!({"schema_version":DELIVERY_VERSION,"message_id":item.envelope.message_id,"state":item.state,"sender":item.envelope.from,"recipient":item.envelope.to,"attempts":item.attempts,"reason":item.reason,"receipt":item.receipt})
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
        .remote_outbox
        .iter()
        .find(|i| {
            i.envelope.message_id == id
                && i.envelope.from.session_id == session
                && i.envelope.from.session_incarnation == current
        })
        .ok_or_else(|| CliError::data("message-not-found", "delivery does not exist", None))?;
    Ok(projection(item))
}
pub(crate) fn drain(context: &CliContext, config: &Config) -> Result<Option<i64>, CliError> {
    let items = {
        let locked = lock_journal(context)?;
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
        let result = if item.envelope.expires_at_epoch <= now {
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
                .and_then(response_json)
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
        current.attempts = current.attempts.saturating_add(1);
        current.next_attempt_epoch = now_epoch().saturating_add(5);
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
        locked.save()?;
    }
    let locked = lock_journal(context)?;
    Ok(locked
        .registry
        .remote_outbox
        .iter()
        .filter(|i| i.state == "queued")
        .map(|i| i.next_attempt_epoch.min(i.envelope.expires_at_epoch))
        .min())
}
pub(crate) fn receive(
    context: &CliContext,
    machine: &str,
    envelope: Envelope,
) -> Result<Value, CliError> {
    let now = now_epoch();
    if envelope.schema_version != ENVELOPE_VERSION
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
        || envelope.from.machine.is_empty()
        || envelope.from.session_id.is_empty()
        || envelope.from.session_incarnation.is_empty()
    {
        return Err(invalid());
    }
    let digest = digest_bytes(&serde_json::to_vec(&envelope).map_err(|_| invalid())?);
    // Durable delivery wins over later expiry or replacement. Release this lock
    // before the session lock; the final transaction repeats the same check.
    {
        let locked = super::lock_registry_observational(context)?;
        if let Some(prior) = receive_replay(&locked.registry, &envelope, &digest)? {
            return Ok(prior);
        }
    }
    if envelope.created_at_epoch > now.saturating_add(60) || envelope.expires_at_epoch <= now {
        return Err(invalid());
    }
    let _session_lock = crate::acquire_session_record_lock(context, &envelope.to.session_id)?;
    let recipient = crate::load_session_record(context, &envelope.to.session_id)?;
    let mut locked = lock_registry(context)?;
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
        &locked.registry,
        &encode_sender(&envelope.from),
        &recipient.id,
        envelope.body.len(),
        now,
        now_millis,
    )?;
    let receipt = json!({"schema_version":DELIVERY_VERSION,"message_id":envelope.message_id,"state":"delivered","recipient":envelope.to,"persisted_at_epoch":now});
    let message = super::mailbox::StoredMessage {
        schema_version: "agent-session.message.v1".into(),
        message_id: envelope.message_id.clone(),
        sender_session_id: encode_sender(&envelope.from),
        sender_incarnation: envelope.from.session_incarnation,
        recipient_session_id: recipient.id.clone(),
        recipient_incarnation: recipient_incarnation.clone(),
        state: "unread".into(),
        revision: 1,
        reply_to: envelope.reply_to,
        reply_depth: envelope.reply_depth,
        created_at: timestamp(now),
        created_at_epoch: now,
        created_at_epoch_millis: now_millis,
        expires_at: timestamp(envelope.expires_at_epoch),
        expires_at_epoch: envelope.expires_at_epoch,
        terminal_at_epoch: None,
        forwarded_from_incarnation: None,
        forwarded_at_epoch: None,
        body_bytes: envelope.body.len(),
        body: envelope.body,
    };
    super::notification::schedule(
        &mut locked.registry,
        &recipient.id,
        &recipient_incarnation,
        now,
    );
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
    locked.save()?;
    Ok(receipt)
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
        Some(
            json!({"to_machine":args.to_machine,"to_session":args.to_session,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.reply_to,"expires_in":args.expires_in,"reply_revision":null}),
        ),
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
        Some(
            json!({"to_machine":sender.machine,"to_session":sender.session_id,"body":body,"idempotency_key":args.idempotency_key,"reply_to":args.message,"expires_in":null,"reply_revision":args.if_revision}),
        ),
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
    let Some(item) = journal.registry.remote_outbox.iter().find(|item| {
        item.envelope.from.session_id == session
            && item.envelope.from.session_incarnation == incarnation
            && item.idempotency_key == args.idempotency_key
    }) else {
        return Ok(None);
    };
    let submission = Submit {
        to_machine: item.envelope.to.machine.clone(),
        to_session: item.envelope.to.session_id.clone(),
        body: body.into(),
        idempotency_key: args.idempotency_key.clone(),
        reply_to: Some(args.message.clone()),
        expires_in: None,
        reply_revision: Some(args.if_revision),
    };
    let digest = digest_bytes(&serde_json::to_vec(&submission).map_err(|_| invalid())?);
    if digest != item.request_digest {
        return Err(CliError::data(
            "idempotency-key-conflict",
            "idempotency key has different content",
            None,
        ));
    }
    Ok(Some(projection(item)))
}

fn receive_replay(
    registry: &super::Registry,
    envelope: &Envelope,
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
mod tests {
    use super::*;
    use pretty_assertions::{assert_eq, assert_ne};
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    const TOKEN: &str = "fixture-private-session-capability-0000000000001";
    fn fixture() -> (tempfile::TempDir, CliContext) {
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
        locked.registry.schema_version = super::super::CLAIM_FENCE_REGISTRY_VERSION.into();
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
    fn envelope() -> Envelope {
        Envelope {
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
                envelope: envelope(),
                request_digest: "fixture".into(),
                idempotency_key: "fixture".into(),
                state: "queued".into(),
                attempts,
                next_attempt_epoch: now_epoch(),
                receipt: None,
                reason: Some("remote-messaging-unavailable".into()),
            });
            locked.save().unwrap();
            drop(locked);
            drain(&context, &config).unwrap();
            server.join().unwrap();
            let locked = lock_journal(&context).unwrap();
            assert_eq!(locked.registry.remote_outbox[0].state, expected);
            assert_eq!(
                locked.registry.remote_outbox[0].reason.as_deref(),
                Some("session-incarnation-conflict")
            );
        }
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
                &locked.registry,
                "other-sender",
                "recipient",
                1,
                now_epoch(),
                super::super::mailbox::now_epoch_millis()
            )
            .unwrap_err()
            .code(),
            "quota-exceeded"
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
            to_machine: "source".into(),
            to_session: "recipient".into(),
            body: "bounded reply".into(),
            idempotency_key: "depth-key-0001".into(),
            reply_to: Some(parent_id),
            expires_in: None,
            reply_revision: Some(1),
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
            envelope: envelope(),
            request_digest: "fixture".into(),
            idempotency_key: "fixture".into(),
            state: "delivered".into(),
            attempts: 1,
            next_attempt_epoch: now_epoch(),
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
            super::super::CLAIM_FENCE_REGISTRY_VERSION
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
            super::super::CLAIM_FENCE_REGISTRY_VERSION
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
        super::super::ensure_recovery_registry_schema(&context).expect("existing facade supported");
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
            super::super::CLAIM_FENCE_REGISTRY_VERSION
        );
    }
    #[test]
    fn outbox_replay_never_rediscovers_or_retargets_and_capability_is_required() {
        let (_temp, context) = fixture();
        let args = Submit {
            to_machine: "other".into(),
            to_session: "other-agent".into(),
            body: "fixture body".into(),
            idempotency_key: "fixture-key-0001".into(),
            reply_to: None,
            expires_in: None,
            reply_revision: None,
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
            envelope: message,
            request_digest: digest_bytes(&serde_json::to_vec(&args).expect("digest")),
            idempotency_key: args.idempotency_key.clone(),
            state: "queued".into(),
            attempts: 0,
            next_attempt_epoch: now_epoch(),
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
            envelope: message,
            request_digest: "digest".into(),
            idempotency_key: "expiry-key-0001".into(),
            state: "queued".into(),
            attempts: 1,
            next_attempt_epoch: 0,
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
        super::super::ensure_recovery_registry_schema(&context)
            .expect("existing reader still available");
    }
}

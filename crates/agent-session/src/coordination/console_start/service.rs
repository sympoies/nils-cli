//! Console starts for explicitly admitted services (`session-coordination-v1`,
//! "Service Console starts").
//!
//! A service is neither a managed session nor a worker delegation. The daemon
//! authenticates its dedicated credential against a separate admission
//! document, checks the exact owner, target machine and expiry the document
//! admits, and relays the start with the service's selectors. The aggregator,
//! not this crate, decides that the relaying machine's service may act for the
//! owner. The shared operation receipt makes a repeat or an uncertain result
//! reconcile to the same operation instead of creating another candidate.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::Args as ClapArgs;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::operation::{self, Origin, Receipt, Scope};
use super::{Config, diagnostics, relay_create};
use crate::coordination::{read_private_file, service};
use crate::lineage::{self, LineageSeed};
use crate::{CliContext, CliError};

pub(crate) const ROUTE: &str = "/coordination/services/console-start/v1";
const ADMISSION_VERSION: &str = "agent-session.console-start-services.v1";
const RECEIPT_SCHEMA: &str = "agent-session.console-start-service-receipt.v1";
const MAX_ADMISSION_BYTES: u64 = 32 * 1024;
const MAX_SERVICES: usize = 32;
const MAX_LISTED: usize = 64;
const MAX_LIFETIME_SECONDS: u64 = 86400;
/// The members a service may set in `session`; lineage, work, role, resume and
/// origin belong to the daemon or are not available to a service.
const SESSION_MEMBERS: [&str; 11] = [
    "agent",
    "cwd",
    "workspace",
    "launch_env",
    "title_mode",
    "title",
    "prompt",
    "agent_args",
    "agent_profile",
    "codex_account",
    "claude_account",
];

/// `start --via-console` as an admitted service. The members are required
/// together, and none combines with a parent, role, work or resume.
#[derive(ClapArgs, Debug, Clone, Default)]
pub struct ServiceArgs {
    /// Admitted service ID; starts a Console-owned root as that service.
    #[arg(
        long,
        value_name = "ID",
        requires_all = ["via_console", "service_generation", "credential_file", "owner"],
        conflicts_with_all = ["no_parent", "role", "program", "issues", "no_inherit_work", "resume"]
    )]
    pub service: Option<String>,
    /// Admitted generation of the service.
    #[arg(long, value_name = "GENERATION", requires = "service")]
    pub service_generation: Option<String>,
    /// Private service credential file; never a session capability.
    #[arg(long, value_name = "PATH", value_hint = clap::ValueHint::FilePath, requires = "service")]
    pub credential_file: Option<PathBuf>,
    /// Console owner the service acts for.
    #[arg(long, value_name = "OWNER", requires = "service")]
    pub owner: Option<String>,
    /// RFC 3339 time after which a new start with this operation is refused.
    #[arg(
        long,
        value_name = "TIMESTAMP",
        requires = "service",
        conflicts_with = "reconcile"
    )]
    pub expires_at: Option<String>,
}

/// A complete service request: clap guarantees every member with `--service`.
pub(super) struct Admitted<'a> {
    service: &'a str,
    generation: &'a str,
    pub(super) credential_file: &'a Path,
    owner: &'a str,
    expires_at: Option<&'a str>,
}

impl ServiceArgs {
    pub(super) fn admitted(&self) -> Option<Admitted<'_>> {
        Some(Admitted {
            service: self.service.as_deref()?,
            generation: self.service_generation.as_deref()?,
            credential_file: self.credential_file.as_deref()?,
            owner: self.owner.as_deref()?,
            expires_at: self.expires_at.as_deref(),
        })
    }
}

fn refusal(code: &'static str, message: &'static str, next_action: &str, usage: bool) -> CliError {
    let details = Some(json!({
        "retryable": false, "next_action": next_action, "recovery": {"kind": "operator"}
    }));
    if usage {
        CliError::usage(code, message, details)
    } else {
        CliError::data(code, message, details)
    }
}

/// Content-free: never says which part of the admission failed.
pub(crate) fn unauthorized() -> CliError {
    refusal(
        "console-start-service-unauthorized",
        "service console start is not authorized",
        "request_service_admission",
        false,
    )
}

/// The request body could not be read as a JSON document.
pub(crate) fn invalid_request() -> CliError {
    invalid("the service console start request is invalid")
}

fn invalid(message: &'static str) -> CliError {
    refusal(
        "console-start-service-invalid",
        message,
        "inspect_request_contract",
        true,
    )
}

fn owner_denied() -> CliError {
    refusal(
        "console-start-service-owner-denied",
        "the service is not admitted for this owner",
        "request_service_admission",
        false,
    )
}

fn target_denied() -> CliError {
    refusal(
        "console-start-service-target-denied",
        "the service is not admitted for this target machine",
        "request_service_admission",
        false,
    )
}

fn expired() -> CliError {
    refusal(
        "console-start-service-expired",
        "the request is expired or exceeds the admitted lifetime",
        "submit_new_request",
        false,
    )
}

fn selector(value: &str) -> bool {
    service::valid_selector(value)
}

/// Machine names are Console's: 1-128 printable ASCII characters, no spaces.
fn machine_name(value: &str) -> bool {
    (1..=128).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_graphic())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    schema_version: String,
    services: Vec<Entry>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    service_id: String,
    service_generation: String,
    credential_file: PathBuf,
    owners: Vec<String>,
    machines: Vec<String>,
    max_lifetime_seconds: u64,
}
impl Entry {
    fn valid(&self) -> bool {
        selector(&self.service_id)
            && selector(&self.service_generation)
            && self.credential_file.is_absolute()
            && (1..=MAX_LISTED).contains(&self.owners.len())
            && self.owners.iter().all(|owner| selector(owner))
            && (1..=MAX_LISTED).contains(&self.machines.len())
            && self.machines.iter().all(|machine| machine_name(machine))
            && (1..=MAX_LIFETIME_SECONDS).contains(&self.max_lifetime_seconds)
    }
}

/// The service credential, read from a private regular file.
pub(super) fn credential(path: &Path) -> Result<String, CliError> {
    let token = String::from_utf8(read_private_file(path, 257).map_err(|_| unauthorized())?)
        .map_err(|_| unauthorized())?;
    let token = token.trim().to_string();
    token_shape(&token)
        .then_some(token)
        .ok_or_else(unauthorized)
}

fn token_shape(token: &str) -> bool {
    (32..=256).contains(&token.len()) && token.bytes().all(|b| b.is_ascii_graphic())
}

/// The admitted entry for exactly this service, generation and credential.
fn authenticate(
    admission_file: Option<&Path>,
    request: &Request,
    token: &str,
    forbidden: &[&str],
) -> Result<Entry, CliError> {
    if !token_shape(token)
        || forbidden
            .iter()
            .any(|other| crate::serve::constant_time_eq(token, other))
    {
        return Err(unauthorized());
    }
    let file = admission_file.ok_or_else(unauthorized)?;
    let admission: Admission = serde_json::from_slice(
        &read_private_file(file, MAX_ADMISSION_BYTES).map_err(|_| unauthorized())?,
    )
    .map_err(|_| unauthorized())?;
    let mut ids = HashSet::new();
    if admission.schema_version != ADMISSION_VERSION
        || admission.services.len() > MAX_SERVICES
        || admission
            .services
            .iter()
            .any(|entry| !entry.valid() || !ids.insert(&entry.service_id))
    {
        return Err(unauthorized());
    }
    let entry = admission
        .services
        .into_iter()
        .find(|entry| {
            entry.service_id == request.service_id
                && entry.service_generation == request.service_generation
        })
        .ok_or_else(unauthorized)?;
    if !crate::serve::constant_time_eq(token, &credential(&entry.credential_file)?) {
        return Err(unauthorized());
    }
    Ok(entry)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    service_id: String,
    service_generation: String,
    owner: String,
    operation_id: String,
    #[serde(default)]
    reconcile: bool,
    machine: Option<String>,
    expires_at: Option<String>,
    session: Option<Map<String, Value>>,
}

impl Request {
    /// A closed request: a reconcile names only the operation, a start also
    /// names its target, expiry and a session built from allowed members.
    fn parse(body: &Value) -> Result<Self, CliError> {
        let request: Self = serde_json::from_value(body.clone())
            .map_err(|_| invalid("the service console start request is invalid"))?;
        let shape = if request.reconcile {
            request.machine.is_none() && request.expires_at.is_none() && request.session.is_none()
        } else {
            request.machine.as_deref().is_some_and(machine_name)
                && request.expires_at.as_deref().and_then(epoch).is_some()
                && request.session.as_ref().is_some_and(|session| {
                    session
                        .keys()
                        .all(|key| SESSION_MEMBERS.contains(&key.as_str()))
                })
        };
        if !shape
            || !selector(&request.service_id)
            || !selector(&request.service_generation)
            || !selector(&request.owner)
            || !operation::valid_id(&request.operation_id)
        {
            return Err(invalid("the service console start request is invalid"));
        }
        Ok(request)
    }
}

fn epoch(timestamp: &str) -> Option<i64> {
    timestamp
        .parse::<jiff::Timestamp>()
        .ok()
        .map(jiff::Timestamp::as_second)
}

/// A new start is refused after its expiry or beyond the admitted lifetime.
fn require_live(expires_at: &str, max_lifetime_seconds: u64) -> Result<(), CliError> {
    let expires = epoch(expires_at).ok_or_else(|| invalid("expires_at is invalid"))?;
    let now = crate::coordination::now_epoch();
    if expires <= now || expires > now.saturating_add(max_lifetime_seconds as i64) {
        return Err(expired());
    }
    Ok(())
}

/// The service's root session: lineage is set here, never by the caller.
fn child_session(
    machine: &str,
    session: &Map<String, Value>,
) -> Result<Map<String, Value>, CliError> {
    if let Some(workspace) = session.get("workspace").filter(|value| !value.is_null()) {
        crate::origin::validate_workspace(
            workspace
                .as_str()
                .ok_or_else(|| invalid("the service console start request is invalid"))?,
        )?;
    }
    let mut session = session.clone();
    session.insert(
        "lineage".into(),
        LineageSeed::root(machine, lineage::STARTER_CONSOLE, lineage::VIA_CONSOLE).to_create_json(),
    );
    Ok(session)
}

/// What a consumer needs to keep its previous session until a verified create.
fn owner_receipt(receipt: &Receipt, operation_id: &str) -> Value {
    let value = receipt.value();
    json!({
        "schema_version": RECEIPT_SCHEMA,
        "service_id": value["principal"]["service_id"],
        "service_generation": value["principal"]["service_generation"],
        "owner": value["owner"],
        "machine": value["machine"],
        "operation_id": operation_id,
        "state": value["state"],
        "safe_to_retry": value["safe_to_retry"],
        "session": value.get("session").cloned().unwrap_or(Value::Null)
    })
}

fn with_receipt(mut result: Value, receipt: &Receipt, operation_id: &str) -> Value {
    result["receipt"] = owner_receipt(receipt, operation_id);
    result
}

/// `POST /coordination/services/console-start/v1`: authenticate the admitted
/// service, then start or reconcile. No lock is held across the network call.
pub(crate) fn relay_route(
    context: &CliContext,
    federation: Option<&Config>,
    admission_file: Option<&Path>,
    token: &str,
    forbidden: &[&str],
    body: &Value,
) -> Result<Value, CliError> {
    let request = Request::parse(body)?;
    let operation = request.operation_id.clone();
    diagnostics::run(&operation, true, || {
        relay_operation(
            context,
            federation,
            admission_file,
            token,
            forbidden,
            request,
        )
    })
}

fn relay_operation(
    context: &CliContext,
    federation: Option<&Config>,
    admission_file: Option<&Path>,
    token: &str,
    forbidden: &[&str],
    request: Request,
) -> Result<Value, CliError> {
    let entry = authenticate(admission_file, &request, token, forbidden)?;
    if !entry.owners.contains(&request.owner) {
        return Err(owner_denied());
    }
    let Some(config) = federation else {
        return Err(super::disabled());
    };
    let operation_id = request.operation_id.as_str();
    let origin = Origin::service(
        &config.machine,
        &request.service_id,
        &request.service_generation,
        &request.owner,
    );
    let scope = Scope::Service {
        id: &request.service_id,
        generation: &request.service_generation,
        owner: &request.owner,
    };
    if request.reconcile {
        let (receipt, _) = Receipt::begin(
            context,
            &origin,
            operation_id,
            &json!({"operation_id": operation_id, "reconcile": true}),
            &config.machine,
        )?;
        // A receipt belongs to the owner it was started for.
        if receipt.value()["owner"] != request.owner.as_str() {
            return Err(operation::not_found());
        }
        let result = receipt.reconcile(config, &scope)?;
        return Ok(with_receipt(result, &receipt, operation_id));
    }
    let (machine, expires_at, session) = (
        request.machine.as_deref().unwrap_or_default(),
        request.expires_at.as_deref().unwrap_or_default(),
        request.session.as_ref().cloned().unwrap_or_default(),
    );
    if !entry.machines.iter().any(|admitted| admitted == machine) {
        return Err(target_denied());
    }
    let child = child_session(&config.machine, &session)?;
    // An already admitted operation replays its stored outcome, even after expiry.
    if !Receipt::exists(context, &origin, operation_id)? {
        require_live(expires_at, entry.max_lifetime_seconds)?;
    }
    let fingerprint = json!({
        "operation_id": operation_id, "owner": request.owner, "machine": machine,
        "expires_at": expires_at, "session": session
    });
    let (mut receipt, submit) =
        Receipt::begin(context, &origin, operation_id, &fingerprint, machine)?;
    if !submit {
        let result = receipt.reconcile(config, &scope)?;
        return Ok(with_receipt(result, &receipt, operation_id));
    }
    let source = json!({
        "source_service_id": request.service_id,
        "source_service_generation": request.service_generation,
        "owner": request.owner
    });
    let result = relay_create(config, source, Some(machine), child, operation_id);
    if receipt.finish(context, &result).is_err() {
        return Err(super::persist_failed(&result));
    }
    result.map(|result| with_receipt(result, &receipt, operation_id))
}

/// The CLI's request to its own daemon.
pub(super) fn cli_body(
    service: &Admitted,
    machine: Option<&str>,
    session: Value,
    operation_id: &str,
    reconcile: bool,
) -> Result<Value, CliError> {
    let mut body = json!({
        "service_id": service.service,
        "service_generation": service.generation,
        "owner": service.owner,
        "operation_id": operation_id
    });
    if reconcile {
        body["reconcile"] = json!(true);
        return Ok(body);
    }
    let (Some(machine), Some(expires_at)) = (machine, service.expires_at) else {
        return Err(invalid(
            "a service console start needs --machine and --expires-at",
        ));
    };
    body["machine"] = json!(machine);
    body["expires_at"] = json!(expires_at);
    body["session"] = session;
    Ok(body)
}

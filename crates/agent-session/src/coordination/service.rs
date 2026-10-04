//! Narrow, explicitly admitted service submission; never session authority.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{digest_bytes, mailbox, read_private_file, remote};
use crate::{CliContext, CliError, cli};

pub(crate) const ROUTE: &str = "/coordination/services/messages/v1";
pub(crate) const WIRE_VERSION: &str = "agent-session.remote-service-message.v1";
const ADMISSION_VERSION: &str = "agent-session.mailbox-services.v1";
const SENDER_PREFIX: &str = "service:";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct Origin {
    pub machine: String,
    pub service_id: String,
    pub service_generation: String,
}
impl Origin {
    pub fn valid(&self) -> bool {
        valid_selector(&self.machine)
            && valid_selector(&self.service_id)
            && valid_selector(&self.service_generation)
    }
    pub fn stored_id(&self) -> String {
        format!(
            "{SENDER_PREFIX}{}",
            serde_json::to_string(&(&self.machine, &self.service_id)).expect("string tuple")
        )
    }
}
pub(crate) fn sender_origin(message: &mailbox::StoredMessage) -> Option<Origin> {
    let encoded = message.sender_session_id.strip_prefix(SENDER_PREFIX)?;
    let (machine, service_id) = serde_json::from_str(encoded).ok()?;
    let origin = Origin {
        machine,
        service_id,
        service_generation: message.sender_incarnation.clone(),
    };
    (origin.valid() && origin.stored_id() == message.sender_session_id).then_some(origin)
}
pub(crate) fn is_service_sender(id: &str) -> bool {
    id.starts_with(SENDER_PREFIX)
}
fn valid_selector(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Admission {
    schema_version: String,
    services: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    service_id: String,
    service_generation: String,
    credential_file: PathBuf,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Submit {
    pub service_id: String,
    pub service_generation: String,
    pub to_machine: Option<String>,
    pub to_session: String,
    pub body: String,
    pub idempotency_key: String,
    pub expires_in: Option<String>,
    pub expected_recipient_incarnation: Option<String>,
}

pub(crate) fn unauthorized() -> CliError {
    CliError::data(
        "mailbox-service-unauthorized",
        "service submission is not authorized",
        Some(json!({
            "retryable": false, "next_action": "request_service_admission", "recovery": {"kind": "operator"}
        })),
    )
}
pub(crate) fn authenticate(
    admission_file: Option<&Path>,
    service_id: &str,
    generation: &str,
    token: &str,
    forbidden_tokens: &[&str],
) -> Result<(), CliError> {
    if !valid_selector(service_id)
        || !valid_selector(generation)
        || !(32..=256).contains(&token.len())
        || token.bytes().any(|b| !b.is_ascii_graphic())
        || forbidden_tokens
            .iter()
            .any(|other| crate::serve::constant_time_eq(token, other))
    {
        return Err(unauthorized());
    }
    let file = admission_file.ok_or_else(unauthorized)?;
    let admission: Admission =
        serde_json::from_slice(&read_private_file(file, 32 * 1024).map_err(|_| unauthorized())?)
            .map_err(|_| unauthorized())?;
    let mut ids = HashSet::new();
    if admission.schema_version != ADMISSION_VERSION
        || admission.services.len() > 32
        || admission.services.iter().any(|entry| {
            !valid_selector(&entry.service_id)
                || !valid_selector(&entry.service_generation)
                || !entry.credential_file.is_absolute()
                || !ids.insert(&entry.service_id)
        })
    {
        return Err(unauthorized());
    }
    let entry = admission
        .services
        .iter()
        .find(|entry| entry.service_id == service_id && entry.service_generation == generation)
        .ok_or_else(unauthorized)?;
    let expected = credential(&entry.credential_file)?;
    if !crate::serve::constant_time_eq(token, &expected) {
        return Err(unauthorized());
    }
    Ok(())
}
fn credential(path: &Path) -> Result<String, CliError> {
    let token = String::from_utf8(read_private_file(path, 257).map_err(|_| unauthorized())?)
        .map_err(|_| unauthorized())?;
    let token = token.trim().to_string();
    if !(32..=256).contains(&token.len()) || token.bytes().any(|b| !b.is_ascii_graphic()) {
        return Err(unauthorized());
    }
    Ok(token)
}

pub(crate) fn submit<F>(
    context: &CliContext,
    machine: &str,
    config: Option<&remote::Config>,
    args: Submit,
    authorize: F,
) -> Result<Value, CliError>
where
    F: Fn() -> Result<(), CliError>,
{
    authorize()?;
    mailbox::validate_body(&args.body)?;
    mailbox::parse_expiry(args.expires_in.as_deref())?;
    super::validate_idempotency_key(&args.idempotency_key)?;
    let origin = Origin {
        machine: machine.into(),
        service_id: args.service_id.clone(),
        service_generation: args.service_generation.clone(),
    };
    if !origin.valid()
        || !valid_selector(&args.to_session)
        || args
            .to_machine
            .as_deref()
            .is_some_and(|m| !valid_selector(m))
    {
        return Err(CliError::usage(
            "mailbox-service-request-invalid",
            "service request selectors are invalid",
            None,
        ));
    }
    if args.to_machine.as_deref().is_some_and(|m| m != machine) {
        let config = config.ok_or_else(|| {
            CliError::unavailable(
                "remote-messaging-unavailable",
                "federation is disabled",
                None,
            )
        })?;
        return remote::submit_service(context, config, origin, args, authorize);
    }
    let digest = digest_bytes(&serde_json::to_vec(&args).map_err(|_| unauthorized())?);
    remote::submit_local_service(context, origin, args, digest, authorize)
}

/// Add recovery fields without exposing transport, paths, credentials or bodies.
pub(crate) fn recovery(error: CliError) -> CliError {
    let data = error.into_inner();
    // Preserve known contract codes and exits, never an IO/peer-supplied message or details.
    let (code, message) = match data.code.as_str() {
        "mailbox-service-unauthorized" => (
            "mailbox-service-unauthorized",
            "service submission is not authorized",
        ),
        "mailbox-service-request-invalid" => (
            "mailbox-service-request-invalid",
            "service request is invalid or oversized",
        ),
        "invalid-duration" => (
            "mailbox-expiry-invalid",
            "mailbox expiry must be positive and no more than 7 days",
        ),
        "mailbox-service-forbidden" => (
            "mailbox-service-forbidden",
            "service submission accepts direct loopback connections only",
        ),
        "mailbox-body-invalid" => (
            "mailbox-body-invalid",
            "mailbox body is empty, invalid UTF-8 or contains forbidden controls",
        ),
        "mailbox-body-too-large" => ("mailbox-body-too-large", "mailbox body exceeds 16 KiB"),
        "mailbox-expiry-invalid" => (
            "mailbox-expiry-invalid",
            "mailbox expiry must be positive and no more than 7 days",
        ),
        "idempotency-key-conflict" => (
            "idempotency-key-conflict",
            "idempotency key has different content",
        ),
        "idempotency-key-reused" => (
            "idempotency-key-reused",
            "idempotency key has different content",
        ),
        "idempotency-key-invalid" => ("idempotency-key-invalid", "idempotency key is invalid"),
        "quota-exceeded" => ("quota-exceeded", "mailbox submission quota exceeded"),
        "rate-limited" => ("rate-limited", "mailbox submission rate exceeded"),
        "session-incarnation-conflict" => (
            "session-incarnation-conflict",
            "recipient incarnation changed",
        ),
        "session-not-found" | "message-not-found" => {
            ("message-not-found", "recipient is unavailable")
        }
        "remote-messaging-unsupported" => (
            "remote-messaging-unsupported",
            "remote recipient is unavailable",
        ),
        "coordination-unavailable" => (
            "coordination-unavailable",
            "mailbox coordination is unavailable",
        ),
        "remote-messaging-unavailable" => (
            "remote-messaging-unavailable",
            "mailbox transport is unavailable",
        ),
        _ => (
            "mailbox-service-unavailable",
            "service submission is unavailable",
        ),
    };
    let retryable = matches!(
        code,
        "remote-messaging-unavailable"
            | "coordination-unavailable"
            | "rate-limited"
            | "quota-exceeded"
    );
    let next_action = if code == "mailbox-service-unauthorized" {
        "request_service_admission"
    } else if retryable {
        "retry_same_submission"
    } else {
        "inspect_submission_contract"
    };
    let details = json!({"retryable":retryable, "next_action":next_action, "recovery":{"kind": if retryable { "same_idempotency_key" } else { "operator" }}});
    let exit_code = match code {
        "remote-messaging-unavailable"
        | "coordination-unavailable"
        | "mailbox-service-unavailable"
        | "remote-messaging-unsupported" => nils_common::cli_contract::exit::UNAVAILABLE,
        "mailbox-service-request-invalid" | "mailbox-expiry-invalid" => {
            nils_common::cli_contract::exit::USAGE
        }
        _ => nils_common::cli_contract::exit::DATA,
    };
    CliError::with_exit_code(code, message, Some(details), exit_code)
}
pub(crate) fn cli_send(
    context: &CliContext,
    args: cli::MessageServiceSendArgs,
) -> Result<Value, CliError> {
    let body = mailbox::read_body(&args.body_file)?;
    let token = credential(&args.credential_file)?;
    let url = remote::daemon_url(context)?;
    let response = remote::client()?
        .post(format!("{}{ROUTE}", url.as_str().trim_end_matches('/')))
        .bearer_auth(token)
        .json(&Submit {
            service_id: args.service,
            service_generation: args.service_generation,
            to_machine: args.to_machine,
            to_session: args.to_session,
            body,
            idempotency_key: args.idempotency_key,
            expires_in: args.expires_in,
            expected_recipient_incarnation: args.expected_recipient_incarnation,
        })
        .send()
        .map_err(|_| {
            CliError::unavailable(
                "remote-messaging-unavailable",
                "local mailbox daemon is unavailable",
                None,
            )
        })?;
    remote::local_response(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    const TOKEN: &str = "fixture-service-credential-0123456789";
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let credential = temp.path().join("credential");
        let admission = temp.path().join("admission.json");
        fs::write(&credential, TOKEN).unwrap();
        fs::write(&admission, serde_json::to_vec(&json!({"schema_version":ADMISSION_VERSION, "services":[{"service_id":"reporter", "service_generation":"generation-1", "credential_file":credential}]})).unwrap()).unwrap();
        for path in [&credential, &admission] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        (temp, credential, admission)
    }
    #[test]
    fn admission_never_grants_operator_or_other_generation_authority() {
        let (_temp, _credential, admission) = fixture();
        assert!(authenticate(Some(&admission), "reporter", "generation-1", TOKEN, &[]).is_ok());
        for (id, generation, forbidden) in [
            ("missing", "generation-1", vec![]),
            ("reporter", "old", vec![]),
            ("reporter", "generation-1", vec![TOKEN]),
        ] {
            assert_eq!(
                authenticate(Some(&admission), id, generation, TOKEN, &forbidden)
                    .unwrap_err()
                    .code(),
                "mailbox-service-unauthorized"
            );
        }
        assert_eq!(
            authenticate(None, "reporter", "generation-1", TOKEN, &[])
                .unwrap_err()
                .code(),
            "mailbox-service-unauthorized"
        );
    }
    #[test]
    fn fifo_credential_is_rejected_without_waiting_for_a_writer() {
        let (_temp, credential, admission) = fixture();
        fs::remove_file(&credential).unwrap();
        let name = std::ffi::CString::new(credential.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            sender
                .send(authenticate(
                    Some(&admission),
                    "reporter",
                    "generation-1",
                    TOKEN,
                    &[],
                ))
                .unwrap()
        });
        let result = receiver.recv_timeout(std::time::Duration::from_secs(1));
        if result.is_err() {
            // Release a regressed blocking reader before asserting; leave no thread or FIFO leak.
            let _writer = fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&credential)
                .unwrap();
        }
        worker.join().unwrap();
        assert!(
            result.is_ok(),
            "unsafe credentials must not block a daemon worker"
        );
        assert_eq!(
            result.unwrap().unwrap_err().code(),
            "mailbox-service-unauthorized"
        );
    }
    #[test]
    fn recovery_is_content_free_even_for_io_or_peer_canaries() {
        let error = recovery(CliError::runtime(
            "unexpected-peer-code",
            "PRIVATE-CANARY-PATH-BODY-TOKEN",
            Some(json!({"body":"PRIVATE-CANARY"})),
        ));
        let data = error.into_inner();
        assert_eq!(data.code, "mailbox-service-unavailable");
        assert!(!data.message.contains("PRIVATE-CANARY"));
        assert!(!data.details.unwrap().to_string().contains("PRIVATE-CANARY"));
    }
}

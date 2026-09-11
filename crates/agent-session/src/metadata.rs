use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Read;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use jiff::Zoned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::cli::{MetadataArgs, MetadataAttachArgs, MetadataCommand, MetadataShowArgs};
use crate::{
    CliContext, CliError, SESSION_DOCUMENT_VERSION, SessionRecord, acquire_new_session_record_lock,
    ensure_private_session_child_ancestor, ensure_same_session_identity, open_session_ancestor_at,
    private_session_state_root, render_error, render_session_document_for_write,
    render_single_success, session_ancestor_untrusted, session_dir, session_effective_uid,
    validate_id, validate_session_ancestor_path_identity, write_private_file_at,
};

const REQUEST_VERSION: &str = "agent-session.metadata-attachment.request.v1";
const STATE_VERSION: &str = "agent-session.public-metadata.v1";
const VIEW_VERSION: &str = "agent-session.public-metadata-view.v1";
const REQUEST_MAX_BYTES: usize = 1024;
const LABEL_MAX_BYTES: usize = 64;
const VALUE_MAX_BYTES: usize = 256;
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;
const ATTACHMENT_MAX_COUNT: usize = 8;
const RECEIPT_MAX_COUNT: usize = ATTACHMENT_MAX_COUNT;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetadataAttachmentRequest {
    schema_version: String,
    label: String,
    value: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SessionMetadataState {
    schema_version: String,
    revision: u64,
    attachments: BTreeMap<String, MetadataAttachment>,
    receipts: BTreeMap<String, MetadataReceipt>,
}

impl Default for SessionMetadataState {
    fn default() -> Self {
        Self {
            schema_version: STATE_VERSION.to_string(),
            revision: 0,
            attachments: BTreeMap::new(),
            receipts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetadataAttachment {
    attachment_id: String,
    label: String,
    revision: u64,
    evidence_digest: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MetadataReceipt {
    request_digest: String,
    attachment_id: String,
    revision: u64,
    evidence_digest: String,
}

#[derive(Debug, Serialize)]
struct MetadataAttachmentView {
    attachment_id: String,
    label: String,
    revision: u64,
    evidence_digest: String,
}

impl From<&MetadataAttachment> for MetadataAttachmentView {
    fn from(value: &MetadataAttachment) -> Self {
        Self {
            attachment_id: value.attachment_id.clone(),
            label: value.label.clone(),
            revision: value.revision,
            evidence_digest: value.evidence_digest.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct MetadataAttachResult {
    id: String,
    metadata: MetadataAttachmentView,
    revision: u64,
    evidence_digest: String,
    replayed: bool,
}

#[derive(Debug, Serialize)]
struct MetadataShowResult {
    schema_version: &'static str,
    id: String,
    revision: u64,
    attachments: Vec<MetadataAttachmentView>,
    matching_count: usize,
}

pub(crate) fn run_metadata(context: &CliContext, args: MetadataArgs) -> i32 {
    match args.command {
        MetadataCommand::Attach(args) => match attach_metadata(context, &args) {
            Ok(result) => {
                render_single_success("metadata-attach", args.format, &result, render_attach_text)
            }
            Err(error) => render_error("metadata-attach", args.format, error),
        },
        MetadataCommand::Show(args) => match show_metadata(context, &args) {
            Ok(result) => {
                render_single_success("metadata-show", args.format, &result, render_show_text)
            }
            Err(error) => render_error("metadata-show", args.format, error),
        },
    }
}

fn attach_metadata(
    context: &CliContext,
    args: &MetadataAttachArgs,
) -> Result<MetadataAttachResult, CliError> {
    validate_id(&args.id)?;
    validate_idempotency_key(&args.idempotency_key)?;
    let request = read_request(&args.request_file)?;
    validate_request(&request)?;
    let canonical_request = serde_json::to_vec(&request).map_err(|_| {
        request_error(
            "metadata-request-invalid",
            "metadata request could not be canonicalized",
        )
    })?;
    let request_digest = digest(b"agent-session.metadata-request.v1\0", &canonical_request);
    let idempotency_digest = digest(
        b"agent-session.metadata-idempotency.v1\0",
        args.idempotency_key.as_bytes(),
    );

    let observed = read_exact_record(context, &args.id)?;
    let state_root = private_session_state_root(context)?;
    let _lock = acquire_new_session_record_lock(context, &state_root, &args.id)?;
    let mut record = read_exact_record(context, &args.id)?;
    ensure_same_session_identity(&observed, &record)?;
    validate_metadata_state(&record)?;

    if let Some(receipt) = record
        .public_metadata
        .as_ref()
        .and_then(|state| state.receipts.get(&idempotency_digest))
    {
        if receipt.request_digest != request_digest {
            return Err(metadata_error(
                "metadata-idempotency-conflict",
                "idempotency key is already bound to different metadata",
                false,
                "use-new-idempotency-key",
                "idempotency-key-replacement",
            ));
        }
        let attachment = record
            .public_metadata
            .as_ref()
            .and_then(|state| state.attachments.get(&request.label))
            .filter(|attachment| attachment.attachment_id == receipt.attachment_id)
            .ok_or_else(|| metadata_state_invalid("metadata receipt target is missing"))?;
        return Ok(attach_result(&record.id, attachment, true));
    }

    let state = record
        .public_metadata
        .get_or_insert_with(SessionMetadataState::default);
    if state.revision != args.if_revision {
        return Err(CliError::data(
            "metadata-revision-conflict",
            "public metadata revision is stale",
            Some(json!({
                "expected_revision": args.if_revision,
                "current_revision": state.revision,
                "retryable": true,
                "next_action": "read-metadata-and-retry",
                "recovery": {
                    "kind": "metadata-revision-refresh",
                    "owner": "caller",
                    "automatic": false
                }
            })),
        ));
    }
    if state.attachments.contains_key(&request.label) {
        return Err(metadata_error(
            "metadata-label-conflict",
            "public metadata label already exists",
            false,
            "choose-new-metadata-label",
            "metadata-label-replacement",
        ));
    }
    if state.attachments.len() >= ATTACHMENT_MAX_COUNT || state.receipts.len() >= RECEIPT_MAX_COUNT
    {
        return Err(metadata_error(
            "metadata-capacity-exceeded",
            "public metadata attachment capacity is exhausted",
            false,
            "inspect-session-metadata",
            "metadata-capacity-review",
        ));
    }

    let revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| metadata_state_invalid("public metadata revision cannot advance"))?;
    let attachment_id = format!(
        "sha256:{}",
        digest(
            b"agent-session.metadata-attachment.v1\0",
            uuid::Uuid::new_v4().as_bytes(),
        )
    );
    let evidence_digest = digest(
        b"agent-session.metadata-evidence.v1\0",
        format!("{}\0{}\0{}", record.id, revision, attachment_id).as_bytes(),
    );
    let attachment = MetadataAttachment {
        attachment_id,
        label: request.label,
        revision,
        evidence_digest: format!("sha256:{evidence_digest}"),
    };
    let receipt = MetadataReceipt {
        request_digest,
        attachment_id: attachment.attachment_id.clone(),
        revision,
        evidence_digest: attachment.evidence_digest.clone(),
    };
    state.revision = revision;
    state
        .attachments
        .insert(attachment.label.clone(), attachment.clone());
    state.receipts.insert(idempotency_digest, receipt);
    record.updated_at = Zoned::now().timestamp().to_string();
    write_exact_record(context, &record)?;
    Ok(attach_result(&record.id, &attachment, false))
}

fn show_metadata(
    context: &CliContext,
    args: &MetadataShowArgs,
) -> Result<MetadataShowResult, CliError> {
    validate_id(&args.id)?;
    if let Some(label) = args.label.as_deref() {
        validate_label(label)?;
    }
    let record = read_exact_record(context, &args.id)?;
    validate_metadata_state(&record)?;
    let revision = record
        .public_metadata
        .as_ref()
        .map_or(0, |state| state.revision);
    let attachments = record
        .public_metadata
        .as_ref()
        .into_iter()
        .flat_map(|state| state.attachments.values())
        .filter(|attachment| {
            args.label
                .as_deref()
                .is_none_or(|label| attachment.label == label)
        })
        .map(MetadataAttachmentView::from)
        .collect::<Vec<_>>();
    Ok(MetadataShowResult {
        schema_version: VIEW_VERSION,
        id: record.id,
        revision,
        matching_count: attachments.len(),
        attachments,
    })
}

fn attach_result(
    id: &str,
    attachment: &MetadataAttachment,
    replayed: bool,
) -> MetadataAttachResult {
    MetadataAttachResult {
        id: id.to_string(),
        metadata: MetadataAttachmentView::from(attachment),
        revision: attachment.revision,
        evidence_digest: attachment.evidence_digest.clone(),
        replayed,
    }
}

fn read_request(path: &Path) -> Result<MetadataAttachmentRequest, CliError> {
    let mut file = open_private_input(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((REQUEST_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            request_error(
                "metadata-request-read-failed",
                "metadata request could not be read",
            )
        })?;
    if bytes.len() > REQUEST_MAX_BYTES {
        return Err(request_error(
            "metadata-request-too-large",
            "metadata request exceeds 1024 bytes",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        request_error(
            "metadata-request-invalid",
            "metadata request is not valid versioned JSON",
        )
    })
}

fn validate_request(request: &MetadataAttachmentRequest) -> Result<(), CliError> {
    if request.schema_version != REQUEST_VERSION {
        return Err(request_error(
            "metadata-request-version-unsupported",
            "metadata request schema_version is unsupported",
        ));
    }
    validate_label(&request.label)?;
    validate_value(&request.label, &request.value)
}

fn validate_label(label: &str) -> Result<(), CliError> {
    if label.is_empty()
        || label.len() > LABEL_MAX_BYTES
        || !label.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
        || !label.as_bytes()[0].is_ascii_lowercase()
    {
        return Err(request_error(
            "metadata-label-invalid",
            "metadata label must be 1-64 bytes and use lowercase ASCII identifiers",
        ));
    }
    let normalized = label.replace(['-', '.'], "_");
    if [
        "token",
        "secret",
        "password",
        "passwd",
        "credential",
        "authorization",
        "api_key",
        "apikey",
        "access_key",
        "private_key",
        "command",
        "argv",
        "environment",
        "env",
        "path",
        "cwd",
        "executable",
        "script",
        "shell",
    ]
    .iter()
    .any(|forbidden| {
        normalized.split('_').any(|part| part == *forbidden) || normalized == *forbidden
    }) {
        return Err(request_error(
            "metadata-label-forbidden",
            "metadata label is reserved for sensitive or executable state",
        ));
    }
    if !matches!(
        label,
        "topic"
            | "category"
            | "status"
            | "priority"
            | "source"
            | "workflow"
            | "component"
            | "stage"
            | "acceptance.synthetic"
    ) {
        return Err(request_error(
            "metadata-label-unsupported",
            "metadata label is not in the reviewed public label registry",
        ));
    }
    Ok(())
}

fn validate_value(label: &str, value: &str) -> Result<(), CliError> {
    if value.is_empty() || value.len() > VALUE_MAX_BYTES || value.chars().any(char::is_control) {
        return Err(request_error(
            "metadata-value-invalid",
            "metadata value must be 1-256 UTF-8 bytes without control characters",
        ));
    }
    let trimmed = value.trim();
    if trimmed != value {
        return Err(request_error(
            "metadata-value-invalid",
            "metadata value must not have leading or trailing whitespace",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let credential_shaped = lower.starts_with("bearer ")
        || lower.starts_with("basic ")
        || lower.starts_with("sk-")
        || lower.starts_with("sk_live_")
        || lower.starts_with("sk_test_")
        || lower.starts_with("pk_live_")
        || lower.starts_with("rk_live_")
        || lower.starts_with("ghp_")
        || lower.starts_with("github_pat_")
        || lower.starts_with("glpat-")
        || lower.starts_with("npm_")
        || lower.starts_with("pypi-")
        || lower.starts_with("hf_")
        || lower.starts_with("xoxb-")
        || lower.starts_with("xoxp-")
        || (lower.starts_with("akia") && trimmed.len() >= 20)
        || (lower.starts_with("aiza") && trimmed.len() >= 20)
        || lower.contains("-----begin private key-----")
        || telegram_token_shaped(trimmed);
    if credential_shaped {
        return Err(request_error(
            "metadata-value-sensitive",
            "metadata value resembles a credential",
        ));
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with("~/")
        || trimmed.starts_with("./")
        || trimmed.starts_with("../")
        || lower.starts_with("file://")
        || (trimmed.len() >= 2
            && trimmed.as_bytes()[0].is_ascii_alphabetic()
            && trimmed.as_bytes()[1] == b':')
    {
        return Err(request_error(
            "metadata-value-path-forbidden",
            "metadata value must not contain a filesystem path",
        ));
    }
    let value_bytes = trimmed.as_bytes();
    let is_separator = |byte: u8| matches!(byte, b'-' | b'.');
    let grammar_valid = if label == "acceptance.synthetic" {
        let Some(suffix) = trimmed.strip_prefix("DSH-METADATA-") else {
            return Err(request_error(
                "metadata-value-invalid",
                "acceptance.synthetic values must use the DSH-METADATA public marker grammar",
            ));
        };
        !suffix.is_empty()
            && suffix.len() <= 64
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && suffix
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            && suffix
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
            && !suffix.as_bytes().windows(2).any(|pair| pair == b"--")
    } else {
        value_bytes.len() <= 64
            && value_bytes.iter().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || is_separator(*byte)
            })
            && value_bytes.first().is_some_and(u8::is_ascii_alphanumeric)
            && value_bytes.last().is_some_and(u8::is_ascii_alphanumeric)
            && !value_bytes
                .windows(2)
                .any(|pair| is_separator(pair[0]) && is_separator(pair[1]))
    };
    if !grammar_valid {
        return Err(request_error(
            "metadata-value-invalid",
            "metadata value does not match its bounded public identifier grammar",
        ));
    }
    Ok(())
}

fn telegram_token_shaped(value: &str) -> bool {
    let Some((prefix, suffix)) = value.split_once(':') else {
        return false;
    };
    prefix.len() >= 6
        && prefix.bytes().all(|byte| byte.is_ascii_digit())
        && suffix.len() >= 20
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn validate_idempotency_key(key: &str) -> Result<(), CliError> {
    if key.is_empty()
        || key.len() > IDEMPOTENCY_KEY_MAX_BYTES
        || !key.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(request_error(
            "metadata-idempotency-key-invalid",
            "idempotency key must be 1-128 printable ASCII bytes",
        ));
    }
    Ok(())
}

fn validate_metadata_state(record: &SessionRecord) -> Result<(), CliError> {
    let Some(state) = record.public_metadata.as_ref() else {
        return Ok(());
    };
    let revisions = state
        .attachments
        .values()
        .map(|attachment| attachment.revision)
        .collect::<std::collections::BTreeSet<_>>();
    let receipt_attachment_ids = state
        .receipts
        .values()
        .map(|receipt| receipt.attachment_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if state.schema_version != STATE_VERSION
        || state.attachments.len() > ATTACHMENT_MAX_COUNT
        || state.receipts.len() > RECEIPT_MAX_COUNT
        || state.receipts.len() != state.attachments.len()
        || receipt_attachment_ids.len() != state.receipts.len()
        || state.revision != state.attachments.len() as u64
        || revisions.len() != state.attachments.len()
        || revisions.iter().copied().ne(1..=state.revision)
        || state.attachments.iter().any(|(label, attachment)| {
            let expected_evidence_digest = format!(
                "sha256:{}",
                digest(
                    b"agent-session.metadata-evidence.v1\0",
                    format!(
                        "{}\0{}\0{}",
                        record.id, attachment.revision, attachment.attachment_id
                    )
                    .as_bytes(),
                )
            );
            label != &attachment.label
                || validate_label(&attachment.label).is_err()
                || !valid_prefixed_sha256(&attachment.attachment_id)
                || !valid_prefixed_sha256(&attachment.evidence_digest)
                || expected_evidence_digest != attachment.evidence_digest
                || attachment.revision == 0
                || attachment.revision > state.revision
        })
        || state.receipts.iter().any(|(key_digest, receipt)| {
            !valid_sha256_hex(key_digest)
                || !valid_sha256_hex(&receipt.request_digest)
                || !valid_prefixed_sha256(&receipt.attachment_id)
                || !valid_prefixed_sha256(&receipt.evidence_digest)
                || receipt.revision == 0
                || receipt.revision > state.revision
                || !state.attachments.values().any(|attachment| {
                    attachment.attachment_id == receipt.attachment_id
                        && attachment.revision == receipt.revision
                        && attachment.evidence_digest == receipt.evidence_digest
                })
        })
    {
        return Err(metadata_state_invalid("public metadata state is invalid"));
    }
    Ok(())
}

fn valid_prefixed_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(valid_sha256_hex)
}

fn valid_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn digest(domain: &[u8], value: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(domain);
    digest.update(value);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn request_error(code: &'static str, message: &'static str) -> CliError {
    CliError::usage(
        code,
        message,
        Some(json!({
            "retryable": false,
            "next_action": "correct-metadata-request",
            "recovery": {
                "kind": "metadata-request-correction",
                "owner": "caller",
                "automatic": false
            }
        })),
    )
}

fn metadata_error(
    code: &'static str,
    message: &'static str,
    retryable: bool,
    next_action: &'static str,
    recovery_kind: &'static str,
) -> CliError {
    CliError::data(
        code,
        message,
        Some(json!({
            "retryable": retryable,
            "next_action": next_action,
            "recovery": {
                "kind": recovery_kind,
                "owner": "caller",
                "automatic": false
            }
        })),
    )
}

fn metadata_state_invalid(message: &'static str) -> CliError {
    metadata_error(
        "metadata-state-invalid",
        message,
        false,
        "inspect-session-metadata",
        "metadata-state-repair",
    )
}

#[cfg(unix)]
struct ExactSessionStorage {
    state_path: PathBuf,
    sessions_path: PathBuf,
    session_path: PathBuf,
    state: fs::File,
    sessions: fs::File,
    session: fs::File,
}

#[cfg(unix)]
fn open_exact_session_storage(
    context: &CliContext,
    id: &str,
) -> Result<ExactSessionStorage, CliError> {
    validate_id(id)?;
    let record_path = session_dir(context, id).join("session.json");
    match fs::symlink_metadata(&record_path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(CliError::runtime(
                "session-not-found",
                format!("session not found: {id}"),
                Some(json!({ "id": id })),
            ));
        }
        Err(_) => return Err(record_untrusted("metadata-unavailable")),
    }
    let state_root = private_session_state_root(context)?;
    let sessions_path = context.state_dir.join("sessions");
    let sessions = ensure_private_session_child_ancestor(
        &state_root.directory,
        c"sessions",
        &sessions_path,
        "sessions",
    )?;
    let session_path = session_dir(context, id);
    let name = std::ffi::CString::new(id).expect("validated session id");
    let session = open_session_ancestor_at(&sessions, &name, "session")?;
    let session_metadata = session
        .metadata()
        .map_err(|_| session_ancestor_untrusted("session", "metadata-unavailable"))?;
    if session_metadata.mode() & 0o077 != 0 {
        return Err(session_ancestor_untrusted(
            "session",
            "permissions-not-private",
        ));
    }
    validate_session_ancestor_path_identity(&state_root.path, &state_root.directory, "state-root")?;
    validate_session_ancestor_path_identity(&sessions_path, &sessions, "sessions")?;
    validate_session_ancestor_path_identity(&session_path, &session, "session")?;
    Ok(ExactSessionStorage {
        state_path: state_root.path,
        sessions_path,
        session_path,
        state: state_root.directory,
        sessions,
        session,
    })
}

#[cfg(unix)]
fn read_exact_record(context: &CliContext, id: &str) -> Result<SessionRecord, CliError> {
    let storage = open_exact_session_storage(context, id)?;
    storage.validate()?;
    let descriptor = unsafe {
        libc::openat(
            storage.session.as_raw_fd(),
            c"session.json".as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if descriptor < 0 {
        return Err(record_untrusted("open-failed"));
    }
    let mut file = unsafe { fs::File::from_raw_fd(descriptor) };
    let metadata = file
        .metadata()
        .map_err(|_| record_untrusted("metadata-unavailable"))?;
    if !metadata.is_file()
        || metadata.uid() != metadata_record_effective_uid()
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(record_untrusted("ownership-or-mode-invalid"));
    }
    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(|_| record_untrusted("read-failed"))?;
    storage.validate()?;
    let record: SessionRecord = serde_json::from_str(&contents).map_err(|_| {
        CliError::data(
            "session-json-invalid",
            "managed session record is not valid JSON",
            None,
        )
    })?;
    if record.schema_version != SESSION_DOCUMENT_VERSION {
        return Err(CliError::data(
            "unsupported-session-version",
            "managed session record schema_version is unsupported",
            None,
        ));
    }
    if record.id != id {
        return Err(CliError::data(
            "session-record-mismatch",
            "managed session record does not match the exact session id",
            None,
        ));
    }
    Ok(record)
}

#[cfg(not(unix))]
fn read_exact_record(context: &CliContext, id: &str) -> Result<SessionRecord, CliError> {
    crate::load_session_record(context, id)
}

#[cfg(unix)]
fn write_exact_record(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let storage = open_exact_session_storage(context, &record.id)?;
    storage.validate()?;
    let bytes = render_session_document_for_write(record)?;
    write_private_file_at(
        &storage.session,
        &storage.session_path.join("session.json"),
        "session.json",
        &bytes,
    )?;
    storage.validate()
}

#[cfg(not(unix))]
fn write_exact_record(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    crate::write_session_document(context, record)
}

#[cfg(unix)]
impl ExactSessionStorage {
    fn validate(&self) -> Result<(), CliError> {
        validate_session_ancestor_path_identity(&self.state_path, &self.state, "state-root")?;
        validate_session_ancestor_path_identity(&self.sessions_path, &self.sessions, "sessions")?;
        validate_session_ancestor_path_identity(&self.session_path, &self.session, "session")
    }
}

#[cfg(unix)]
fn metadata_record_effective_uid() -> u32 {
    #[cfg(debug_assertions)]
    if let Ok(value) = std::env::var("NILS_AGENT_SESSION_TEST_METADATA_RECORD_UID")
        && let Ok(value) = value.parse::<u32>()
    {
        return value;
    }
    session_effective_uid()
}

#[cfg(unix)]
fn record_untrusted(reason: &'static str) -> CliError {
    metadata_error(
        "metadata-session-record-untrusted",
        "managed session record is not owner-private",
        false,
        "repair-session-record",
        reason,
    )
}

#[cfg(unix)]
fn open_private_input(path: &Path) -> Result<fs::File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| {
            request_error(
                "metadata-request-untrusted",
                "metadata request file could not be opened safely",
            )
        })?;
    let metadata = file.metadata().map_err(|_| {
        request_error(
            "metadata-request-untrusted",
            "metadata request file metadata is unavailable",
        )
    })?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(request_error(
            "metadata-request-untrusted",
            "metadata request file must be one owner-private regular file",
        ));
    }
    Ok(file)
}

#[cfg(not(unix))]
fn open_private_input(path: &Path) -> Result<fs::File, CliError> {
    fs::File::open(path).map_err(|_| {
        request_error(
            "metadata-request-read-failed",
            "metadata request could not be read",
        )
    })
}

fn render_attach_text(result: &MetadataAttachResult) -> String {
    format!(
        "{} metadata {} attached at revision {}{}\n",
        result.id,
        result.metadata.label,
        result.revision,
        if result.replayed { " (replay)" } else { "" }
    )
}

fn render_show_text(result: &MetadataShowResult) -> String {
    if result.attachments.is_empty() {
        return format!(
            "{} has no matching public metadata at revision {}\n",
            result.id, result.revision
        );
    }
    let mut text = format!(
        "{} public metadata revision {}\n",
        result.id, result.revision
    );
    for attachment in &result.attachments {
        text.push_str(&format!(
            "{} ({})\n",
            attachment.label, attachment.attachment_id
        ));
    }
    text
}

//! Signed records for transferring one stopped managed provider conversation.
//!
//! Authenticating a record and comparing observations does not consume a move.
//! The lifecycle owner must make its own durable admission or abort transition.

use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod ssh;
#[cfg(test)]
mod tests;

pub use ssh::Verifier;

pub const TICKET_SCHEMA: &str = "agent-session.resume-move-ticket.v1";
pub const STOP_PROOF_SCHEMA: &str = "agent-session.resume-move-stop-proof.v1";
pub const NO_WRITER_SCHEMA: &str = "agent-session.resume-move-no-writer.v1";
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
pub const MAX_SIGNATURE_BYTES: usize = 1024;
const MAX_AGE_SECONDS: i64 = 3600;
const CLOCK_SKEW_SECONDS: i64 = 60;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SignedRecord {
    pub payload: String,
    pub signature: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Codex,
    Claude,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub move_id: String,
    pub nonce: String,
    pub source_host: String,
    pub destination_host: String,
    pub source_session_id: String,
    pub source_incarnation: String,
    pub provider: Provider,
    pub provider_session_id: String,
    pub account: String,
    pub destination_checkout_sha256: String,
}

impl Binding {
    fn validate(&self) -> Result<(), Refusal> {
        if !host_identity(&self.source_host)
            || !host_identity(&self.destination_host)
            || self.source_host == self.destination_host
            || !canonical_uuid(&self.move_id)
            || !canonical_uuid(&self.nonce)
            || !canonical_uuid(&self.source_incarnation)
            || !canonical_uuid(&self.provider_session_id)
            || self.source_session_id.len() > 128
            || crate::validate_id(&self.source_session_id).is_err()
            || !nils_common::provider_runtime::accounts::is_valid_account_nickname(&self.account)
            || !valid_digest(&self.destination_checkout_sha256)
        {
            return Err(Refusal::ProofInvalid);
        }
        Ok(())
    }

    fn check(&self, expected: &Self) -> Result<(), Refusal> {
        if self.source_host != expected.source_host
            || self.destination_host != expected.destination_host
        {
            return Err(Refusal::TargetMismatch);
        }
        if self.move_id != expected.move_id || self.nonce != expected.nonce {
            return Err(Refusal::MoveMismatch);
        }
        if self.source_session_id != expected.source_session_id
            || self.source_incarnation != expected.source_incarnation
            || self.provider != expected.provider
            || self.provider_session_id != expected.provider_session_id
        {
            return Err(Refusal::SessionMismatch);
        }
        if self.account != expected.account {
            return Err(Refusal::AccountMismatch);
        }
        if self.destination_checkout_sha256 != expected.destination_checkout_sha256 {
            return Err(Refusal::PathMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryRole {
    CodexRollout,
    ClaudeTranscript,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub role: HistoryRole,
    pub relative_path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl History {
    fn validate(&self, binding: &Binding) -> Result<(), Refusal> {
        let filename_suffix = format!("{}.jsonl", binding.provider_session_id);
        let valid_path = match (binding.provider, self.role) {
            (Provider::Codex, HistoryRole::CodexRollout) => {
                match self.relative_path.split('/').collect::<Vec<_>>().as_slice() {
                    ["sessions", year, month, day, name] => {
                        decimal_component(year, 4)
                            && decimal_component(month, 2)
                            && decimal_component(day, 2)
                            && name.len() <= 255
                            && name.starts_with("rollout-")
                            && name.ends_with(&format!("-{filename_suffix}"))
                            && name.bytes().all(|byte| {
                                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
                            })
                    }
                    _ => false,
                }
            }
            (Provider::Claude, HistoryRole::ClaudeTranscript) => {
                self.relative_path == filename_suffix
            }
            _ => false,
        };
        if !valid_path || self.bytes == 0 || !valid_digest(&self.sha256) {
            return Err(Refusal::HistoryUnsupported);
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    pub schema_version: String,
    pub binding: Binding,
    pub issued_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StopProof {
    pub schema_version: String,
    pub ticket_sha256: String,
    pub binding: Binding,
    pub source_generation: u64,
    pub account_binding_revision: u64,
    pub source_checkout_sha256: String,
    pub stopped: bool,
    pub issued_at: i64,
    pub expires_at: i64,
    pub history: History,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AbortVerdict {
    AbortedNoWriter,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoWriterReceipt {
    pub schema_version: String,
    pub binding: Binding,
    pub stop_proof_sha256: String,
    pub verdict: AbortVerdict,
}

/// A record authenticated against local trust and checked for structural validity.
/// Freshness, local observations, and move transitions are checked separately.
#[derive(Debug)]
pub struct Verified<T> {
    record: T,
    sha256: String,
}

impl<T> Verified<T> {
    pub fn record(&self) -> &T {
        &self.record
    }

    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Observations from the final destination admission boundary.
pub struct Observation<'a> {
    pub binding: &'a Binding,
    pub history: &'a History,
    pub recorded_source_checkout_sha256: &'a str,
    pub destination_writer_present: bool,
    pub now: i64,
}

impl Verified<Ticket> {
    pub fn check(&self, expected: &Binding, now: i64) -> Result<(), Refusal> {
        self.record.binding.check(expected)?;
        check_time(now, self.record.issued_at, self.record.expires_at)
    }
}

impl Verified<StopProof> {
    pub fn check(
        &self,
        ticket: &Verified<Ticket>,
        observed: Observation<'_>,
    ) -> Result<(), Refusal> {
        let proof = &self.record;
        proof.binding.check(observed.binding)?;
        ticket.check(&proof.binding, observed.now)?;
        if proof.ticket_sha256 != ticket.sha256 {
            return Err(Refusal::MoveMismatch);
        }
        check_time(observed.now, proof.issued_at, proof.expires_at)?;
        if proof.issued_at < ticket.record.issued_at.saturating_sub(CLOCK_SKEW_SECONDS)
            || proof.expires_at > ticket.record.expires_at
        {
            return Err(Refusal::ProofExpired);
        }
        if !proof.stopped {
            return Err(Refusal::SourceLive);
        }
        if proof.source_checkout_sha256 != observed.recorded_source_checkout_sha256 {
            return Err(Refusal::PathMismatch);
        }
        if proof.history != *observed.history {
            return Err(Refusal::HistoryStale);
        }
        if observed.destination_writer_present {
            return Err(Refusal::DestinationWriterPresent);
        }
        Ok(())
    }
}

impl Verified<NoWriterReceipt> {
    /// Abort remains verifiable after the stop proof expires for admission.
    pub fn check(&self, proof: &Verified<StopProof>) -> Result<(), Refusal> {
        if !proof.record.stopped {
            return Err(Refusal::SourceLive);
        }
        self.record.binding.check(&proof.record.binding)?;
        if self.record.stop_proof_sha256 != proof.sha256 {
            return Err(Refusal::MoveMismatch);
        }
        Ok(())
    }
}

impl Verifier {
    pub fn verify_ticket(
        &self,
        signed: &SignedRecord,
        destination: &str,
    ) -> Result<Verified<Ticket>, Refusal> {
        let record: Verified<Ticket> = self.authenticate(signed, destination, TICKET_SCHEMA)?;
        let ticket = &record.record;
        if ticket.schema_version != TICKET_SCHEMA || ticket.binding.destination_host != destination
        {
            return Err(Refusal::ProofInvalid);
        }
        ticket.binding.validate()?;
        check_interval(ticket.issued_at, ticket.expires_at)?;
        Ok(record)
    }

    pub fn verify_stop_proof(
        &self,
        signed: &SignedRecord,
        source: &str,
    ) -> Result<Verified<StopProof>, Refusal> {
        let record: Verified<StopProof> = self.authenticate(signed, source, STOP_PROOF_SCHEMA)?;
        let proof = &record.record;
        if proof.schema_version != STOP_PROOF_SCHEMA
            || proof.binding.source_host != source
            || proof.source_generation == 0
            || proof.account_binding_revision == 0
            || !valid_digest(&proof.ticket_sha256)
            || !valid_digest(&proof.source_checkout_sha256)
        {
            return Err(Refusal::ProofInvalid);
        }
        proof.binding.validate()?;
        proof.history.validate(&proof.binding)?;
        check_interval(proof.issued_at, proof.expires_at)?;
        Ok(record)
    }

    pub fn verify_no_writer_receipt(
        &self,
        signed: &SignedRecord,
        destination: &str,
    ) -> Result<Verified<NoWriterReceipt>, Refusal> {
        let record: Verified<NoWriterReceipt> =
            self.authenticate(signed, destination, NO_WRITER_SCHEMA)?;
        let receipt = &record.record;
        if receipt.schema_version != NO_WRITER_SCHEMA
            || receipt.binding.destination_host != destination
            || !valid_digest(&receipt.stop_proof_sha256)
        {
            return Err(Refusal::ProofInvalid);
        }
        receipt.binding.validate()?;
        Ok(record)
    }

    fn authenticate<T: serde::de::DeserializeOwned>(
        &self,
        signed: &SignedRecord,
        principal: &str,
        namespace: &'static str,
    ) -> Result<Verified<T>, Refusal> {
        if !host_identity(principal) {
            return Err(Refusal::ProofInvalid);
        }
        self.verify_signature(signed, principal, namespace)?;
        let record = serde_json::from_str(&signed.payload).map_err(|_| Refusal::ProofInvalid)?;
        Ok(Verified {
            record,
            sha256: sha256(signed.payload.as_bytes()),
        })
    }
}

fn check_interval(issued: i64, expires: i64) -> Result<(), Refusal> {
    if issued <= 0 || expires <= issued || expires - issued > MAX_AGE_SECONDS {
        return Err(Refusal::ProofExpired);
    }
    Ok(())
}

fn check_time(now: i64, issued: i64, expires: i64) -> Result<(), Refusal> {
    if now > expires || issued > now.saturating_add(CLOCK_SKEW_SECONDS) {
        return Err(Refusal::ProofExpired);
    }
    Ok(())
}

fn host_identity(value: &str) -> bool {
    value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.'))
}

fn canonical_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| !id.is_nil() && id.to_string() == value)
}

fn decimal_component(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub fn sha256(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    ProofInvalid,
    ProofExpired,
    MoveMismatch,
    SessionMismatch,
    TargetMismatch,
    AccountMismatch,
    SourceLive,
    PathMismatch,
    HistoryStale,
    HistoryUnsupported,
    DestinationWriterPresent,
    SshKeygenUnavailable,
    TrustUnavailable,
}

impl Refusal {
    pub fn code(self) -> &'static str {
        match self {
            Self::ProofInvalid => "resume-move-proof-invalid",
            Self::ProofExpired => "resume-move-proof-expired",
            Self::MoveMismatch => "resume-move-binding-mismatch",
            Self::SessionMismatch => "resume-move-session-mismatch",
            Self::TargetMismatch => "resume-move-target-mismatch",
            Self::AccountMismatch => "resume-account-mismatch",
            Self::SourceLive => "resume-source-live",
            Self::PathMismatch => "resume-move-path-mismatch",
            Self::HistoryStale => "resume-move-history-stale",
            Self::HistoryUnsupported => "resume-move-history-unsupported",
            Self::DestinationWriterPresent => "resume-move-destination-writer-present",
            Self::SshKeygenUnavailable => "resume-move-ssh-keygen-unavailable",
            Self::TrustUnavailable => "resume-move-trust-unavailable",
        }
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for Refusal {}

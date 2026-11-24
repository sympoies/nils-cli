//! Destination move reservations and the exclusive admission/abort transition.

use std::collections::BTreeMap;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{AgentKind, CliContext, CliError, coordination, start_resume};

use super::{
    AbortVerdict, Binding, History, NO_WRITER_SCHEMA, NoWriterReceipt, Observation, Provider,
    Refusal, SignedRecord, StopProof, TICKET_SCHEMA, Ticket, Verified, Verifier, canonical_uuid,
    cli_refusal, host_identity, sha256, source, valid_digest,
};

const STORE: &str = "resume-move-destination.v1.json";
const CONTROL_LOCK: &str = "resume-move-destination.lock";
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Prepared,
    Staged,
    WriterAdmitted,
    Created,
    AbortedNoWriter,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    plan: source::Plan,
    checkout: Checkout,
    ticket: SignedRecord,
    proof: Option<SignedRecord>,
    outcome: Outcome,
    created: Option<Created>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Created {
    session_id: String,
    incarnation: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Checkout {
    cwd: PathBuf,
    device: u64,
    inode: u64,
    git: Option<source::GitCheckout>,
}

pub(super) struct Candidate {
    binding: Binding,
    checkout: Checkout,
}

type Moves = BTreeMap<String, Entry>;

#[derive(Serialize)]
pub(super) struct Preparation {
    pub(super) ticket: SignedRecord,
    pub(super) provider_home: PathBuf,
}

#[derive(Clone, Copy)]
pub(super) struct AdmissionRequest<'a> {
    pub(super) move_id: &'a str,
    pub(super) account: &'a str,
    pub(super) provider_session_id: &'a str,
    pub(super) cwd: &'a Path,
    pub(super) destination_host: &'a str,
}

pub(super) enum Admission {
    New {
        provider_home: PathBuf,
        _writer: File,
    },
    Existing {
        session_id: String,
        incarnation: String,
    },
}

pub(super) fn check_plan(
    context: &CliContext,
    plan: &source::Plan,
    cwd: &Path,
    move_id: &str,
    host: &str,
) -> Result<Candidate, CliError> {
    let candidate = candidate(plan, cwd, move_id, host)?;
    ensure_empty_destination(context, &candidate.binding)?;
    let moves = read_moves(context)?;
    if moves
        .get(move_id)
        .is_some_and(|move_record| move_record.outcome == Outcome::AbortedNoWriter)
    {
        return Err(aborted());
    }
    if moves.iter().any(|(id, entry)| {
        entry.plan.provider == plan.provider
            && entry.plan.provider_session_id == plan.provider_session_id
            && entry.outcome != Outcome::AbortedNoWriter
            && id != move_id
    }) {
        return Err(cli_refusal(Refusal::DestinationWriterPresent));
    }
    let roster = crate::codex_account::list_accounts()
        .map_err(|_| start_resume::refusal("resume-unsupported"))?;
    if !roster.iter().any(|account| account.account == plan.account) {
        return Err(cli_refusal(Refusal::AccountMismatch));
    }
    if !crate::codex_app_server::resume_transport_supported(&crate::resolve_agent_bin(
        AgentKind::Codex,
        None,
    )) {
        return Err(start_resume::refusal("resume-unsupported"));
    }
    Ok(candidate)
}

impl Candidate {
    pub(super) fn view(&self) -> Value {
        json!({
            "move_id": self.binding.move_id,
            "state": "ready",
            "provider": self.binding.provider,
            "provider_session_id": self.binding.provider_session_id,
            "account": self.binding.account,
        })
    }
}

pub(super) fn prepare(
    context: &CliContext,
    plan: source::Plan,
    candidate: Candidate,
    verifier: &Verifier,
    key: &Path,
    now: i64,
) -> Result<Preparation, CliError> {
    let _control = lock(context)?;
    let move_id = candidate.binding.move_id.clone();
    let mut moves = read_moves(context)?;
    if let Some(entry) = moves.get(&move_id) {
        let ticket = entry.ticket(verifier)?;
        let mut expected = candidate.binding;
        expected.nonce.clone_from(&ticket.record().binding.nonce);
        ticket
            .record()
            .binding
            .check(&expected)
            .map_err(cli_refusal)?;
        if entry.plan != plan || entry.checkout != candidate.checkout {
            return Err(cli_refusal(Refusal::MoveMismatch));
        }
        if entry.outcome == Outcome::AbortedNoWriter {
            return Err(aborted());
        }
        if !matches!(entry.outcome, Outcome::Prepared | Outcome::Staged) {
            return Err(cli_refusal(Refusal::DestinationWriterPresent));
        }
        sync_directory(context)?;
        return Ok(Preparation {
            ticket: entry.ticket.clone(),
            provider_home: provider_home(context, &move_id),
        });
    }
    ensure_empty_destination(context, &candidate.binding)?;
    deny_reserved(&moves, plan.provider, &plan.provider_session_id)?;
    let provider_home = provider_home(context, &move_id);
    create_provider_home(&provider_home)?;
    let ticket = verifier
        .sign_ticket(
            &Ticket {
                schema_version: TICKET_SCHEMA.into(),
                binding: candidate.binding,
                issued_at: now,
                expires_at: now.saturating_add(super::MAX_AGE_SECONDS),
            },
            key,
        )
        .map_err(cli_refusal)?;
    verifier
        .verify_ticket(&ticket, &plan.destination_host)
        .map_err(cli_refusal)?;
    moves.insert(
        move_id,
        Entry {
            plan,
            checkout: candidate.checkout,
            ticket: ticket.clone(),
            proof: None,
            outcome: Outcome::Prepared,
            created: None,
        },
    );
    write_moves(context, &moves)?;
    Ok(Preparation {
        ticket,
        provider_home,
    })
}

#[cfg(test)]
pub(super) fn prepare_fixture(
    context: &CliContext,
    plan: &source::Plan,
    cwd: &Path,
    move_id: &str,
    verifier: &Verifier,
    key: &Path,
    now: i64,
) -> Result<Preparation, CliError> {
    prepare(
        context,
        plan.clone(),
        candidate(plan, cwd, move_id, &plan.destination_host)?,
        verifier,
        key,
        now,
    )
}

pub(super) fn stage(
    context: &CliContext,
    move_id: &str,
    proof: &SignedRecord,
    verifier: &Verifier,
    host: &str,
    now: i64,
) -> Result<(), CliError> {
    let _control = lock(context)?;
    let mut moves = read_moves(context)?;
    let entry = entry_mut(&mut moves, move_id)?;
    entry.check_host(host)?;
    match entry.outcome {
        Outcome::Prepared | Outcome::Staged => {}
        Outcome::AbortedNoWriter => return Err(aborted()),
        _ => return Err(cli_refusal(Refusal::DestinationWriterPresent)),
    }
    let signed = entry.validate_proof(verifier, proof)?;
    if entry
        .proof
        .as_ref()
        .is_some_and(|prior| sha256(prior.payload.as_bytes()) != signed.sha256())
    {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    let home = provider_home(context, move_id);
    let ticket = entry.ticket(verifier)?;
    entry.check_snapshot(context, &ticket, &signed, &home, now)?;
    entry.proof = Some(proof.clone());
    entry.outcome = Outcome::Staged;
    write_moves(context, &moves)
}

pub(super) fn check(
    context: &CliContext,
    verifier: &Verifier,
    request: AdmissionRequest<'_>,
    now: i64,
) -> Result<(), CliError> {
    verify_admission(context, &read_moves(context)?, verifier, request, now).map(|_| ())
}

pub(super) fn admit(
    context: &CliContext,
    verifier: &Verifier,
    request: AdmissionRequest<'_>,
    now: i64,
) -> Result<Admission, CliError> {
    let _control = lock(context)?;
    let mut moves = read_moves(context)?;
    let verified = verify_admission(context, &moves, verifier, request, now)?;
    if let Some(created) = &verified.created {
        return Ok(Admission::Existing {
            session_id: created.session_id.clone(),
            incarnation: created.incarnation.clone(),
        });
    }
    let entry = entry_mut(&mut moves, request.move_id)?;
    entry.outcome = Outcome::WriterAdmitted;
    write_moves(context, &moves)?;
    let writer = start_resume::committed_move_conversation_lock(
        context,
        AgentKind::Codex,
        request.provider_session_id,
    )?;
    Ok(Admission::New {
        provider_home: provider_home(context, request.move_id),
        _writer: writer,
    })
}

pub(super) fn mark_created(
    context: &CliContext,
    move_id: &str,
    session_id: &str,
    incarnation: &str,
) -> Result<(), CliError> {
    let _control = lock(context)?;
    let mut moves = read_moves(context)?;
    let entry = entry_mut(&mut moves, move_id)?;
    if entry.outcome != Outcome::WriterAdmitted || !canonical_uuid(incarnation) {
        return Err(outcome_unknown());
    }
    entry.created = Some(Created {
        session_id: session_id.to_owned(),
        incarnation: incarnation.to_owned(),
    });
    entry.outcome = Outcome::Created;
    write_moves(context, &moves)
}

pub(super) fn abort(
    context: &CliContext,
    move_id: &str,
    verifier: &Verifier,
    key: &Path,
    host: &str,
    supplied: Option<&SignedRecord>,
) -> Result<SignedRecord, CliError> {
    let _control = lock(context)?;
    let mut moves = read_moves(context)?;
    let entry = entry_mut(&mut moves, move_id)?;
    entry.check_host(host)?;
    if matches!(entry.outcome, Outcome::WriterAdmitted | Outcome::Created) {
        return Err(cli_refusal(Refusal::DestinationWriterPresent));
    }
    let ticket = entry.ticket(verifier)?;
    ensure_empty_destination(context, &ticket.record().binding)?;
    let explicit = supplied.or(entry.proof.as_ref());
    let proof = explicit
        .map(|proof| entry.validate_proof(verifier, proof))
        .transpose()?;
    if let (Some(prior), Some(proof)) = (&entry.proof, &proof)
        && sha256(prior.payload.as_bytes()) != proof.sha256()
    {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    if let Some(explicit) = explicit {
        entry.proof = Some(explicit.clone());
    }
    entry.outcome = Outcome::AbortedNoWriter;
    write_moves(context, &moves)?;
    let proof = proof.ok_or_else(proof_missing)?;
    verifier
        .sign_no_writer_receipt(
            &NoWriterReceipt {
                schema_version: NO_WRITER_SCHEMA.into(),
                binding: proof.record().binding.clone(),
                stop_proof_sha256: proof.sha256().to_owned(),
                verdict: AbortVerdict::AbortedNoWriter,
            },
            key,
        )
        .map_err(cli_refusal)
}

pub(super) fn status(context: &CliContext, move_id: &str) -> Result<Value, CliError> {
    let moves = read_moves(context)?;
    let entry = entry(&moves, move_id)?;
    Ok(json!({
        "move_id": move_id,
        "state": entry.outcome,
        "provider": entry.plan.provider,
        "provider_session_id": entry.plan.provider_session_id,
        "created": entry.created,
    }))
}

pub(crate) fn writer_guard(
    context: &CliContext,
    agent: AgentKind,
    thread: &str,
) -> Result<Option<File>, CliError> {
    let provider = match agent {
        AgentKind::Codex => Provider::Codex,
        AgentKind::Claude => Provider::Claude,
        AgentKind::Dsh => return Ok(None),
    };
    let control = lock(context)?;
    deny_reserved(&read_moves(context)?, provider, thread)?;
    Ok(Some(control))
}

fn deny_reserved(moves: &Moves, provider: Provider, thread: &str) -> Result<(), CliError> {
    if moves.values().any(|entry| {
        entry.plan.provider == provider
            && entry.plan.provider_session_id == thread
            && matches!(
                entry.outcome,
                Outcome::Prepared | Outcome::Staged | Outcome::WriterAdmitted
            )
    }) {
        return Err(start_resume::refusal("resume-move-destination-reserved"));
    }
    Ok(())
}

fn verify_admission<'a>(
    context: &CliContext,
    moves: &'a Moves,
    verifier: &Verifier,
    request: AdmissionRequest<'_>,
    now: i64,
) -> Result<&'a Entry, CliError> {
    let entry = entry(moves, request.move_id)?;
    entry.check_host(request.destination_host)?;
    let ticket = entry.ticket(verifier)?;
    let binding = &ticket.record().binding;
    if binding.provider_session_id != request.provider_session_id
        || binding.provider != Provider::Codex
    {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    if binding.account != request.account {
        return Err(cli_refusal(Refusal::AccountMismatch));
    }
    if Checkout::observe(request.cwd)? != entry.checkout {
        return Err(cli_refusal(Refusal::PathMismatch));
    }
    match entry.outcome {
        Outcome::AbortedNoWriter => return Err(aborted()),
        Outcome::WriterAdmitted => return Err(outcome_unknown()),
        Outcome::Created if entry.created.is_some() => return Ok(entry),
        Outcome::Created => return Err(outcome_unknown()),
        Outcome::Prepared => return Err(proof_missing()),
        Outcome::Staged => {}
    }
    let proof = entry.proof.as_ref().ok_or_else(proof_missing)?;
    let proof = entry.validate_proof(verifier, proof)?;
    entry.check_snapshot(
        context,
        &ticket,
        &proof,
        &provider_home(context, request.move_id),
        now,
    )?;
    Ok(entry)
}

impl Entry {
    fn check_host(&self, host: &str) -> Result<(), CliError> {
        if host != self.plan.destination_host {
            return Err(cli_refusal(Refusal::TargetMismatch));
        }
        Ok(())
    }

    fn ticket(&self, verifier: &Verifier) -> Result<Verified<Ticket>, CliError> {
        let record = verifier
            .verify_ticket(&self.ticket, &self.plan.destination_host)
            .map_err(cli_refusal)?;
        if record.record().binding.destination_checkout_sha256 != self.checkout.digest()? {
            return Err(cli_refusal(Refusal::PathMismatch));
        }
        Ok(record)
    }

    fn validate_proof(
        &self,
        verifier: &Verifier,
        proof: &SignedRecord,
    ) -> Result<Verified<StopProof>, CliError> {
        let proof = verifier
            .verify_stop_proof(proof, &self.plan.source_host)
            .map_err(cli_refusal)?;
        let ticket = self.ticket(verifier)?;
        let record = proof.record();
        record
            .binding
            .check(&ticket.record().binding)
            .map_err(cli_refusal)?;
        if record.ticket_sha256 != ticket.sha256()
            || record.source_generation != self.plan.source_generation
            || record.account_binding_revision != self.plan.account_binding_revision
        {
            return Err(cli_refusal(Refusal::MoveMismatch));
        }
        if record.source_checkout_sha256 != self.plan.source_checkout_sha256 {
            return Err(cli_refusal(Refusal::PathMismatch));
        }
        if record.history.relative_path != self.plan.history.relative_path {
            return Err(cli_refusal(Refusal::HistoryUnsupported));
        }
        if !record.stopped {
            return Err(cli_refusal(Refusal::SourceLive));
        }
        Ok(proof)
    }

    fn check_snapshot(
        &self,
        context: &CliContext,
        ticket: &Verified<Ticket>,
        proof: &Verified<StopProof>,
        home: &Path,
        now: i64,
    ) -> Result<(), CliError> {
        let binding = &ticket.record().binding;
        ensure_empty_destination(context, binding)?;
        if Checkout::observe(&self.checkout.cwd)? != self.checkout {
            return Err(cli_refusal(Refusal::PathMismatch));
        }
        let (history, recorded_cwd) = read_history(home, &proof.record().history, binding)?;
        proof
            .check(
                ticket,
                Observation {
                    binding,
                    history: &history,
                    recorded_source_checkout_sha256: &recorded_cwd,
                    destination_writer_present: false,
                    now,
                },
            )
            .map_err(cli_refusal)
    }
}

impl Checkout {
    fn digest(&self) -> Result<String, CliError> {
        let encoded = serde_json::to_vec(self).map_err(|_| state_unavailable())?;
        Ok(sha256(&encoded))
    }

    fn observe(cwd: &Path) -> Result<Self, CliError> {
        let mismatch = || cli_refusal(Refusal::PathMismatch);
        if !cwd.is_absolute() {
            return Err(mismatch());
        }
        let cwd = crate::resolve_cwd(Some(cwd)).map_err(|_| mismatch())?;
        let metadata = fs::metadata(&cwd).map_err(|_| mismatch())?;
        Ok(Self {
            git: source::git_checkout(&cwd),
            cwd,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

fn candidate(
    plan: &source::Plan,
    cwd: &Path,
    move_id: &str,
    host: &str,
) -> Result<Candidate, CliError> {
    if plan.provider != Provider::Codex {
        return Err(start_resume::refusal("resume-unsupported"));
    }
    if !host_identity(host) || plan.destination_host != host {
        return Err(cli_refusal(Refusal::TargetMismatch));
    }
    if plan.source_generation == 0 || plan.account_binding_revision == 0 {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    let checkout = Checkout::observe(cwd)?;
    let same_path =
        sha256(checkout.cwd.as_os_str().as_encoded_bytes()) == plan.source_checkout_sha256;
    if !valid_digest(&plan.source_checkout_sha256)
        || (!same_path && (checkout.git.is_none() || checkout.git != plan.git_checkout))
    {
        return Err(cli_refusal(Refusal::PathMismatch));
    }
    let binding = Binding {
        move_id: move_id.to_owned(),
        nonce: uuid::Uuid::new_v4().to_string(),
        source_host: plan.source_host.clone(),
        destination_host: plan.destination_host.clone(),
        source_session_id: plan.source_session_id.clone(),
        source_incarnation: plan.source_incarnation.clone(),
        provider: plan.provider,
        provider_session_id: plan.provider_session_id.clone(),
        account: plan.account.clone(),
        destination_checkout_sha256: checkout.digest()?,
    };
    binding.validate().map_err(cli_refusal)?;
    History {
        role: plan.history.role,
        relative_path: plan.history.relative_path.clone(),
        bytes: plan.history.bytes,
        sha256: sha256(b""),
    }
    .validate(&binding)
    .map_err(cli_refusal)?;
    Ok(Candidate { binding, checkout })
}

fn read_history(
    home: &Path,
    expected: &History,
    binding: &Binding,
) -> Result<(History, String), CliError> {
    let missing = || start_resume::refusal("resume-history-missing");
    let unsupported = || cli_refusal(Refusal::HistoryUnsupported);
    let stale = || cli_refusal(Refusal::HistoryStale);
    // The authorised root is a B-local private directory. No recorded source
    // path is used to open a file on B.
    expected.validate(binding).map_err(cli_refusal)?;
    let path = home.join(&expected.relative_path);
    if fs::canonicalize(&path).map_err(|_| missing())? != path {
        return Err(unsupported());
    }
    validate_history_directories(home, &path)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| missing())?;
    let before = file.metadata().map_err(|_| missing())?;
    // SAFETY: geteuid takes no arguments and cannot fail.
    let uid = unsafe { libc::geteuid() };
    if !before.is_file() || before.nlink() != 1 || before.uid() != uid || before.mode() & 0o077 != 0
    {
        return Err(unsupported());
    }
    if before.len() != expected.bytes {
        return Err(stale());
    }
    let mut metadata = Vec::new();
    BufReader::new((&mut file).take(64 * 1024 + 1))
        .read_until(b'\n', &mut metadata)
        .map_err(|_| unsupported())?;
    if metadata.len() > 64 * 1024 {
        return Err(unsupported());
    }
    let value: Value = serde_json::from_slice(&metadata).map_err(|_| unsupported())?;
    let row = &value["payload"];
    if value["type"] != "session_meta"
        || row.get("history_mode").is_some_and(|mode| mode != "legacy")
    {
        return Err(unsupported());
    }
    if row["id"] != binding.provider_session_id {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    let recorded_cwd = sha256(row["cwd"].as_str().ok_or_else(unsupported)?.as_bytes());
    file.rewind().map_err(|_| stale())?;
    let mut body = file.take(expected.bytes.saturating_add(1));
    let mut hasher = Sha256::new();
    let mut bytes = 0;
    let mut buffer = [0u8; 8192];
    loop {
        let count = body.read(&mut buffer).map_err(|_| stale())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        bytes += count as u64;
    }
    let after = fs::symlink_metadata(&path).map_err(|_| stale())?;
    if bytes != expected.bytes
        || after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.len() != before.len()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(stale());
    }
    Ok((
        History {
            role: expected.role,
            relative_path: expected.relative_path.clone(),
            bytes,
            sha256: hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        },
        recorded_cwd,
    ))
}

fn validate_history_directories(home: &Path, file: &Path) -> Result<(), CliError> {
    // SAFETY: geteuid takes no arguments and cannot fail.
    let uid = unsafe { libc::geteuid() };
    let unsupported = || cli_refusal(Refusal::HistoryUnsupported);
    let relative = file.strip_prefix(home).map_err(|_| unsupported())?;
    let mut directory = home.to_path_buf();
    for component in relative.components() {
        let metadata = fs::symlink_metadata(&directory).map_err(|_| unsupported())?;
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
            return Err(unsupported());
        }
        // Only the manifest's one file belongs in the staging home. A sibling
        // could redirect the provider through a copied index or configuration.
        let mut entries = fs::read_dir(&directory).map_err(|_| unsupported())?;
        let child = entries
            .next()
            .ok_or_else(unsupported)?
            .map_err(|_| unsupported())?;
        if child.file_name() != component.as_os_str() || entries.next().is_some() {
            return Err(unsupported());
        }
        directory.push(component.as_os_str());
    }
    Ok(())
}

fn ensure_empty_destination(context: &CliContext, binding: &Binding) -> Result<(), CliError> {
    let agent = match binding.provider {
        Provider::Codex => AgentKind::Codex,
        Provider::Claude => AgentKind::Claude,
    };
    let path = coordination_dir(context).join(start_resume::conversation_lock_name(
        agent,
        &binding.provider_session_id,
    ));
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(cli_refusal(Refusal::DestinationWriterPresent)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(state_unavailable()),
    }
    if start_resume::has_matching_record(context, agent, &binding.provider_session_id)? {
        return Err(cli_refusal(Refusal::DestinationWriterPresent));
    }
    Ok(())
}

fn lock(context: &CliContext) -> Result<File, CliError> {
    coordination::lock_private_store(context, CONTROL_LOCK).map_err(|_| state_unavailable())
}

fn coordination_dir(context: &CliContext) -> PathBuf {
    context.state_dir.join("coordination")
}

fn provider_home(context: &CliContext, move_id: &str) -> PathBuf {
    coordination_dir(context)
        .join("move-imports")
        .join(move_id)
        .join("provider")
}

fn create_provider_home(home: &Path) -> Result<(), CliError> {
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(home)
        .map_err(|_| state_unavailable())?;
    if fs::canonicalize(home).map_err(|_| state_unavailable())? != home {
        return Err(state_unavailable());
    }
    Ok(())
}

fn read_moves(context: &CliContext) -> Result<Moves, CliError> {
    let path = coordination_dir(context).join(STORE);
    let bytes = match coordination::read_private_file(&path, MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Moves::new()),
        Err(_) => return Err(state_unavailable()),
    };
    serde_json::from_slice(&bytes).map_err(|_| state_unavailable())
}

fn write_moves(context: &CliContext, moves: &Moves) -> Result<(), CliError> {
    let bytes = serde_json::to_vec(moves).map_err(|_| state_unavailable())?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(state_unavailable());
    }
    nils_common::fs::write_atomic(&coordination_dir(context).join(STORE), &bytes, 0o600)
        .map_err(|_| state_unavailable())?;
    sync_directory(context)
}

fn sync_directory(context: &CliContext) -> Result<(), CliError> {
    File::open(coordination_dir(context))
        .and_then(|file| file.sync_all())
        .map_err(|_| state_unavailable())
}

fn entry<'a>(moves: &'a Moves, id: &str) -> Result<&'a Entry, CliError> {
    if !canonical_uuid(id) {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    moves.get(id).ok_or_else(proof_missing)
}

fn entry_mut<'a>(moves: &'a mut Moves, id: &str) -> Result<&'a mut Entry, CliError> {
    if !canonical_uuid(id) {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    moves.get_mut(id).ok_or_else(proof_missing)
}

fn state_unavailable() -> CliError {
    cli_refusal(Refusal::StateUnavailable)
}

fn aborted() -> CliError {
    start_resume::refusal("resume-move-aborted")
}

fn proof_missing() -> CliError {
    start_resume::refusal("resume-move-proof-missing")
}

pub(super) fn outcome_unknown() -> CliError {
    CliError::runtime(
        "resume-move-outcome-unknown",
        "writer admission occurred and its outcome is unresolved",
        None,
    )
}

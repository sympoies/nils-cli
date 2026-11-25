//! Durable source evidence and its exact rollback transition.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AgentKind, CliContext, CliError, coordination, start_resume};

use super::{
    Binding, History, Provider, Refusal, STOP_PROOF_SCHEMA, SignedRecord, StopProof, Verifier,
    canonical_uuid, cli_refusal, sha256, source,
};

const STORE: &str = "resume-move-source.v1.json";
const STORE_LOCK: &str = "resume-move-source.lock";
const MAX_STORE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    binding: Binding,
    proof: SignedRecord,
    proof_sha256: String,
    recovered: bool,
}

type Fences = BTreeMap<String, Fence>;

pub(super) struct Export<'a> {
    pub session: &'a str,
    pub incarnation: &'a str,
    pub source_host: &'a str,
    pub destination: &'a str,
    pub ticket: &'a SignedRecord,
    pub signing_key: &'a Path,
}

/// Issue or replay a signed stopped snapshot. The source record remains in place.
/// The caller enables this operation only with working destination abort support.
pub(super) fn export(
    context: &CliContext,
    verifier: &Verifier,
    request: Export<'_>,
    tmux: &Path,
) -> Result<SignedRecord, CliError> {
    let ticket = verifier
        .verify_ticket(request.ticket, request.destination)
        .map_err(cli_refusal)?;
    let binding = &ticket.record().binding;
    if binding.source_host != request.source_host {
        return Err(cli_refusal(Refusal::TargetMismatch));
    }
    if binding.source_session_id != request.session
        || binding.source_incarnation != request.incarnation
    {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    let agent = agent_kind(binding.provider);
    let _conversation =
        start_resume::raw_conversation_lock(context, agent, &binding.provider_session_id)?;
    let _ledger = coordination::lock_private_store(context, STORE_LOCK)?;
    let mut fences = read_fences(context)?;
    if let Some(fence) = fences.get(&binding.move_id) {
        let prior = verifier
            .verify_stop_proof(&fence.proof, &binding.source_host)
            .map_err(cli_refusal)?;
        if fence.recovered
            || fence.binding != *binding
            || prior.record().ticket_sha256 != ticket.sha256()
            || prior.sha256() != fence.proof_sha256
        {
            return Err(cli_refusal(Refusal::MoveMismatch));
        }
        sync_fence_directory(context)?;
        return Ok(fence.proof.clone());
    }
    ensure_unfenced_in(&fences, agent, &binding.provider_session_id)?;
    let now = jiff::Timestamp::now().as_second();
    ticket.check(binding, now).map_err(cli_refusal)?;

    let selected = source::inspect(
        context,
        request.session,
        request.incarnation,
        request.source_host,
        request.destination,
    )?;
    check_binding(binding, &selected.plan)?;

    let (records, _sources) =
        start_resume::matching_sources(context, agent, &binding.provider_session_id, None)?;
    let unknown = || start_resume::refusal("resume-source-unknown");
    let selected_record = records
        .iter()
        .find(|record| record.id == request.session)
        .ok_or_else(unknown)?;
    if selected_record.runtime.as_ref().is_none_or(|runtime| {
        runtime.launch_id != request.incarnation
            || runtime.generation != selected.plan.source_generation
    }) {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    for record in &records {
        start_resume::prove_stopped(context, record, tmux)?;
        let (_, account, revision, root) = source::binding(record)?;
        if account != binding.account
            || (record.id == selected_record.id
                && revision != selected.plan.account_binding_revision)
        {
            return Err(cli_refusal(Refusal::AccountMismatch));
        }
        if fs::canonicalize(root).ok().as_deref() != Some(selected.provider_root.as_path()) {
            return Err(unknown());
        }
        let cwd = crate::resolve_cwd(Some(Path::new(&record.cwd))).map_err(|_| unknown())?;
        if sha256(cwd.as_os_str().as_encoded_bytes()) != selected.plan.source_checkout_sha256 {
            return Err(cli_refusal(Refusal::PathMismatch));
        }
    }

    let history = read_snapshot(binding, &selected)?;
    let issued_at = jiff::Timestamp::now().as_second();
    let expires_at = ticket.record().expires_at;
    super::check_interval(issued_at, expires_at).map_err(cli_refusal)?;
    let proof = StopProof {
        schema_version: STOP_PROOF_SCHEMA.into(),
        ticket_sha256: ticket.sha256().into(),
        binding: binding.clone(),
        source_generation: selected.plan.source_generation,
        account_binding_revision: selected.plan.account_binding_revision,
        source_checkout_sha256: selected.plan.source_checkout_sha256,
        stopped: true,
        issued_at,
        expires_at,
        history,
    };
    let signed = verifier
        .sign_stop_proof(&proof, request.signing_key)
        .map_err(cli_refusal)?;
    verifier
        .verify_stop_proof(&signed, &binding.source_host)
        .map_err(cli_refusal)?;
    fences.insert(
        binding.move_id.clone(),
        Fence {
            binding: binding.clone(),
            proof_sha256: sha256(signed.payload.as_bytes()),
            proof: signed.clone(),
            recovered: false,
        },
    );
    write_fences(context, &fences)?;
    Ok(signed)
}

/// Release an exact retained source fence after the destination durably aborts.
pub(super) fn recover(
    context: &CliContext,
    verifier: &Verifier,
    move_id: &str,
    source_host: &str,
    receipt: &SignedRecord,
) -> Result<(), CliError> {
    if !canonical_uuid(move_id) {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    let observed = read_fences(context)?;
    let observed = observed
        .get(move_id)
        .ok_or_else(|| cli_refusal(Refusal::MoveMismatch))?;
    let binding = &observed.binding;
    if binding.source_host != source_host {
        return Err(cli_refusal(Refusal::TargetMismatch));
    }
    let agent = agent_kind(binding.provider);
    let _conversation =
        start_resume::raw_conversation_lock(context, agent, &binding.provider_session_id)?;
    let _ledger = coordination::lock_private_store(context, STORE_LOCK)?;
    let mut fences = read_fences(context)?;
    let fence = fences
        .get_mut(move_id)
        .ok_or_else(|| cli_refusal(Refusal::MoveMismatch))?;
    if fence.binding != *binding || fence.proof_sha256 != observed.proof_sha256 {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    let proof = verifier
        .verify_stop_proof(&fence.proof, &binding.source_host)
        .map_err(cli_refusal)?;
    if proof.sha256() != fence.proof_sha256 || proof.record().binding != fence.binding {
        return Err(cli_refusal(Refusal::MoveMismatch));
    }
    let receipt = verifier
        .verify_no_writer_receipt(receipt, &binding.destination_host)
        .map_err(cli_refusal)?;
    receipt.check(&proof).map_err(cli_refusal)?;
    if fence.recovered {
        return sync_fence_directory(context);
    }
    let selected = source::inspect(
        context,
        &binding.source_session_id,
        &binding.source_incarnation,
        source_host,
        &binding.destination_host,
    )?;
    check_binding(binding, &selected.plan)?;
    let prior = proof.record();
    if selected.plan.source_generation != prior.source_generation
        || selected.plan.account_binding_revision != prior.account_binding_revision
    {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    if selected.plan.source_checkout_sha256 != prior.source_checkout_sha256 {
        return Err(cli_refusal(Refusal::PathMismatch));
    }
    if read_snapshot(binding, &selected)? != prior.history {
        return Err(cli_refusal(Refusal::HistoryStale));
    }
    fence.recovered = true;
    write_fences(context, &fences)
}

pub(crate) fn ensure_unfenced(
    context: &CliContext,
    agent: AgentKind,
    conversation: &str,
) -> Result<(), CliError> {
    if !matches!(agent, AgentKind::Codex | AgentKind::Claude) {
        return Ok(());
    }
    ensure_unfenced_in(&read_fences(context)?, agent, conversation)
}

fn ensure_unfenced_in(
    fences: &Fences,
    agent: AgentKind,
    conversation: &str,
) -> Result<(), CliError> {
    if fences.values().any(|fence| {
        !fence.recovered
            && agent_kind(fence.binding.provider) == agent
            && fence.binding.provider_session_id == conversation
    }) {
        return Err(cli_refusal(Refusal::SourceFenced));
    }
    Ok(())
}

fn check_binding(binding: &Binding, plan: &source::Plan) -> Result<(), CliError> {
    let expected = Binding {
        source_host: plan.source_host.clone(),
        destination_host: plan.destination_host.clone(),
        source_session_id: plan.source_session_id.clone(),
        source_incarnation: plan.source_incarnation.clone(),
        provider: plan.provider,
        provider_session_id: plan.provider_session_id.clone(),
        account: plan.account.clone(),
        ..binding.clone()
    };
    binding.check(&expected).map_err(cli_refusal)
}

fn read_snapshot(binding: &Binding, selected: &source::Source) -> Result<History, CliError> {
    let missing = || start_resume::refusal("resume-history-missing");
    let stale = || cli_refusal(Refusal::HistoryStale);
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&selected.history_path)
        .map_err(|_| missing())?;
    let before = file.metadata().map_err(|_| missing())?;
    if !before.is_file() || before.nlink() != 1 {
        return Err(cli_refusal(Refusal::HistoryUnsupported));
    }
    validate_history_meta(binding, &selected.plan, &mut file)?;
    let mut hasher = Sha256::new();
    let mut snapshot = (&mut file).take(before.len().saturating_add(1));
    let mut bytes = 0;
    let mut buffer = [0u8; 8192];
    loop {
        let read = snapshot.read(&mut buffer).map_err(|_| missing())?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        hasher.update(&buffer[..read]);
    }
    let after = fs::symlink_metadata(&selected.history_path).map_err(|_| stale())?;
    if bytes != before.len()
        || after.len() != before.len()
        || after.dev() != before.dev()
        || after.ino() != before.ino()
        || after.modified().ok() != before.modified().ok()
    {
        return Err(stale());
    }
    let history = History {
        role: selected.plan.history.role,
        relative_path: selected.plan.history.relative_path.clone(),
        bytes,
        sha256: hasher
            .finalize()
            .iter()
            .map(|value| format!("{value:02x}"))
            .collect(),
    };
    history.validate(binding).map_err(cli_refusal)?;
    Ok(history)
}

fn validate_history_meta(
    binding: &Binding,
    plan: &source::Plan,
    file: &mut File,
) -> Result<(), CliError> {
    if binding.provider != Provider::Codex {
        return Ok(());
    }
    let unsupported = || cli_refusal(Refusal::HistoryUnsupported);
    let mut line = Vec::new();
    {
        let mut first = BufReader::new((&mut *file).take(64 * 1024 + 1));
        first
            .read_until(b'\n', &mut line)
            .map_err(|_| unsupported())?;
    }
    if line.len() > 64 * 1024 {
        return Err(unsupported());
    }
    let line: serde_json::Value = serde_json::from_slice(&line).map_err(|_| unsupported())?;
    if line["type"] != "session_meta"
        || line["payload"]
            .get("history_mode")
            .is_some_and(|value| value != "legacy")
    {
        return Err(unsupported());
    }
    if line["payload"]["id"] != binding.provider_session_id {
        return Err(cli_refusal(Refusal::SessionMismatch));
    }
    if line["payload"]["cwd"]
        .as_str()
        .is_none_or(|cwd| sha256(cwd.as_bytes()) != plan.source_checkout_sha256)
    {
        return Err(cli_refusal(Refusal::PathMismatch));
    }
    file.rewind().map_err(|_| unsupported())
}

fn ledger_path(context: &CliContext) -> PathBuf {
    context.state_dir.join("coordination").join(STORE)
}

fn read_fences(context: &CliContext) -> Result<Fences, CliError> {
    let bytes = match coordination::read_private_file(&ledger_path(context), MAX_STORE_BYTES) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Fences::new()),
        Err(_) => return Err(cli_refusal(Refusal::StateUnavailable)),
    };
    serde_json::from_slice(&bytes).map_err(|_| cli_refusal(Refusal::StateUnavailable))
}

fn write_fences(context: &CliContext, fences: &Fences) -> Result<(), CliError> {
    let fail = || cli_refusal(Refusal::StateUnavailable);
    let path = ledger_path(context);
    let serialized = serde_json::to_vec(fences).map_err(|_| fail())?;
    if serialized.len() as u64 > MAX_STORE_BYTES {
        return Err(fail());
    }
    nils_common::fs::write_atomic(&path, &serialized, 0o600).map_err(|_| fail())?;
    sync_fence_directory(context)
}

fn sync_fence_directory(context: &CliContext) -> Result<(), CliError> {
    let path = context.state_dir.join("coordination");
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| cli_refusal(Refusal::StateUnavailable))
}

fn agent_kind(provider: Provider) -> AgentKind {
    match provider {
        Provider::Codex => AgentKind::Codex,
        Provider::Claude => AgentKind::Claude,
    }
}

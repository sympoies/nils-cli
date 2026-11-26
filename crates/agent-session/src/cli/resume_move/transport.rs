//! Destination transport projections over the native move engine and ledger.
use std::path::Path;

use axum::http::StatusCode;
use nils_common::cli_contract::OutputFormat;
use serde_json::{Value, json};

use crate::coordination::console_resume_move::{refusal::Failure, wire::Session};
use crate::{CliContext, cli};

use super::destination::{self, Entry, Outcome};
use super::{StopProof, Ticket, Verified, Verifier, args, source};

pub(crate) fn configured() -> bool {
    source::local_host().is_ok()
        && args::verifier_paths().is_ok_and(|(ssh_keygen, allowed_signers)| {
            Verifier::check_config(&ssh_keygen, &allowed_signers).is_ok()
        })
}

pub(crate) fn start_wire(
    context: &CliContext,
    tmux: &Path,
    machine: &str,
    session: Session,
) -> Result<Value, Failure> {
    if !configured() {
        return Err(Failure::before_start(
            StatusCode::SERVICE_UNAVAILABLE,
            "resume-move-unavailable",
        ));
    }
    let Session {
        move_id,
        agent,
        resume,
        account,
        cwd,
    } = session;
    let args = cli::StartArgs {
        resume: Some(resume),
        move_id: Some(move_id.clone()),
        check_resume: false,
        initial_resume_account_explicit: Some(true),
        workspace: None,
        app_server_managed: true,
        initial_codex_account: None,
        initial_codex_account_source: None,
        initial_title_state: None,
        initial_agent_profile: None,
        initial_provider_config_dir: None,
        initial_profile_auto_resume_supported: None,
        initial_profile_graceful_shutdown: None,
        initial_codex_usage_account: None,
        initial_lineage: None,
        agent: agent.agent(),
        cwd: Some(cwd),
        title: None,
        id: None,
        prompt: None,
        prompt_file: None,
        prompt_stdin: false,
        notice: Default::default(),
        tmux_bin: Some(tmux.to_owned()),
        agent_bin: None,
        agent_args: Vec::new(),
        launch_env: Vec::new(),
        coordination_mode: Default::default(),
        paste_delay_ms: cli::DEFAULT_PASTE_DELAY_MS,
        via_console: false,
        console_operation: Default::default(),
        machine: None,
        account: Some(account),
        agent_profile: None,
        no_parent: true,
        forge_initiator: None,
        attest_role: false,
        role: None,
        program: None,
        issues: Vec::new(),
        no_inherit_work: true,
        format: OutputFormat::Json,
        title_mode: "auto".into(),
    };
    let result = args::start(context, args);
    let entry = destination::read_entry(context, &move_id).ok();
    let state = entry.as_ref().map(|entry| entry.outcome);
    match result {
        Ok(_) => {
            let (id, incarnation) = entry
                .as_ref()
                .and_then(|entry| entry.created_identity(&move_id))
                .ok_or_else(|| Failure::unknown(state))?;
            Ok(json!({
                "machine":machine,"move_id":move_id,"state":"created",
                "session_id":id,"session_incarnation":incarnation
            }))
        }
        Err(error) => Err(Failure::after_start(error.code(), state)),
    }
}

pub(crate) fn status_projection(
    context: &CliContext,
    machine: &str,
    move_id: &str,
) -> Result<Value, Failure> {
    let disabled = || Failure::read(StatusCode::SERVICE_UNAVAILABLE, "resume-move-unavailable");
    if !configured() {
        return Err(disabled());
    }
    let host = source::local_host().map_err(|_| disabled())?;
    let entry = destination::read_entry(context, move_id).map_err(|error| {
        if error.code() == "resume-move-proof-missing" {
            Failure::read(StatusCode::NOT_FOUND, "resume-move-proof-missing")
        } else {
            Failure::read(
                StatusCode::SERVICE_UNAVAILABLE,
                "resume-move-state-unavailable",
            )
        }
    })?;
    entry.check_host(&host).map_err(|_| {
        Failure::read(
            StatusCode::UNPROCESSABLE_ENTITY,
            "resume-move-target-mismatch",
        )
    })?;
    let (session_id, session_incarnation) = if entry.outcome == Outcome::Created {
        let (id, incarnation) = entry.created_identity(move_id).ok_or_else(|| {
            Failure::read(
                StatusCode::UNPROCESSABLE_ENTITY,
                "resume-move-session-mismatch",
            )
        })?;
        (Some(id), Some(incarnation))
    } else {
        (None, None)
    };
    let (freshness, expires) = proof_freshness(&entry, move_id, args::now());
    Ok(json!({
        "machine":machine,"move_id":move_id,"state":entry.outcome,
        "session_id":session_id,"session_incarnation":session_incarnation,
        "proof_freshness":freshness,"proof_expires_at_epoch":expires
    }))
}

fn proof_freshness(entry: &Entry, move_id: &str, now: i64) -> (&'static str, Option<i64>) {
    let Some(signed) = &entry.proof else {
        return ("missing", None);
    };
    let verified = || {
        let verifier = args::verifier().ok()?;
        let ticket = entry.ticket(&verifier).ok()?;
        let proof = entry.validate_proof(&verifier, &ticket, signed).ok()?;
        (proof.record().binding.move_id == move_id).then_some((ticket, proof))
    };
    let Some((ticket, proof)) = verified() else {
        return ("unverified", None);
    };
    verified_freshness(&ticket, &proof, now)
}

pub(super) fn verified_freshness(
    ticket: &Verified<Ticket>,
    proof: &Verified<StopProof>,
    now: i64,
) -> (&'static str, Option<i64>) {
    let epoch = proof.record().expires_at;
    // Test timing at the last valid second when the proof has already expired.
    // That distinguishes a valid expired window from evidence that was never valid.
    let at = now.min(epoch);
    if ticket.check(&proof.record().binding, at).is_err() || proof.check_time(ticket, at).is_err() {
        return ("unverified", None);
    }
    if now > epoch {
        ("expired", Some(epoch))
    } else {
        ("fresh", Some(epoch))
    }
}

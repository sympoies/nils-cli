use std::path::{Path, PathBuf};

use clap::{Args, Subcommand, ValueHint};
use nils_common::cli_contract::OutputFormat;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AgentKind, CliContext, CliError, InitialClaudeAccount, ProviderResume, StartView, cli,
    coordination, lifecycle, start_resume,
};

use super::{
    Provider, Refusal, SignedRecord, Verifier, claude, cli_refusal, destination, source,
    source_fence,
};

const MAX_RECORD_BYTES: u64 =
    (6 * (super::MAX_PAYLOAD_BYTES + super::MAX_SIGNATURE_BYTES) + 256) as u64;

#[derive(Debug, Args)]
pub struct MoveArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect the exact portable source history without stopping the source.
    Plan {
        /// Managed source session ID.
        #[arg(long)]
        session: String,
        /// Exact source runtime incarnation UUID.
        #[arg(long)]
        incarnation: String,
        /// Configured destination signer identity.
        #[arg(long)]
        destination: String,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Export a stopped source only when the complete move protocol is available.
    Export {
        /// Managed source session ID.
        #[arg(long)]
        session: String,
        /// Exact source runtime incarnation UUID.
        #[arg(long)]
        incarnation: String,
        /// Configured destination signer identity.
        #[arg(long)]
        destination: String,
        /// File containing the signed destination ticket.
        #[arg(long, value_hint = ValueHint::FilePath)]
        ticket_file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Inspect or reserve the local destination for an exact source plan.
    Prepare {
        /// Unique UUID for this move.
        #[arg(long)]
        move_id: String,
        /// File containing the read-only source plan.
        #[arg(long, value_hint = ValueHint::FilePath)]
        plan_file: PathBuf,
        /// Local destination checkout or working directory.
        #[arg(long, value_hint = ValueHint::DirPath)]
        cwd: PathBuf,
        /// Validate the destination without writing a reservation.
        #[arg(long)]
        check: bool,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Verify a source proof and the exact privately staged provider history.
    Stage {
        /// Exact move UUID.
        #[arg(long)]
        move_id: String,
        /// File containing the signed source stop proof.
        #[arg(long, value_hint = ValueHint::FilePath)]
        proof_file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Read the durable destination outcome for an exact move.
    Status {
        /// Exact move UUID.
        #[arg(long)]
        move_id: String,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Verify or record a terminal abort before destination writer admission.
    Abort {
        /// Exact move UUID.
        #[arg(long)]
        move_id: String,
        /// Signed source proof when staging has not recorded it.
        #[arg(long, value_hint = ValueHint::FilePath)]
        proof_file: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
    /// Release a source move fence using a verified destination receipt.
    Recover {
        /// Move UUID used to fence the source.
        #[arg(long)]
        move_id: String,
        /// File containing the signed destination no-writer receipt.
        #[arg(long, value_hint = ValueHint::FilePath)]
        receipt_file: PathBuf,
        #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
        format: OutputFormat,
    },
}

impl MoveArgs {
    pub(crate) fn format(&self) -> OutputFormat {
        match self.command {
            Command::Plan { format, .. }
            | Command::Export { format, .. }
            | Command::Recover { format, .. }
            | Command::Prepare { format, .. }
            | Command::Stage { format, .. }
            | Command::Status { format, .. }
            | Command::Abort { format, .. } => format,
        }
    }

    pub(crate) fn name(&self) -> &'static str {
        match self.command {
            Command::Plan { .. } => "resume-move-plan",
            Command::Export { .. } => "resume-move-export",
            Command::Recover { .. } => "resume-move-recover",
            Command::Prepare { .. } => "resume-move-prepare",
            Command::Stage { .. } => "resume-move-stage",
            Command::Status { .. } => "resume-move-status",
            Command::Abort { .. } => "resume-move-abort",
        }
    }
}

pub(crate) fn run(context: &CliContext, args: MoveArgs) -> i32 {
    match args.command {
        Command::Plan {
            session,
            incarnation,
            destination,
            format,
        } => {
            const COMMAND: &str = "resume-move-plan";
            match super::source::plan(context, &session, &incarnation, &destination) {
                Ok(plan) => crate::render_single_success(COMMAND, format, &plan, |_| {
                    "source move plan available\n".into()
                }),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
        Command::Export {
            session,
            incarnation,
            destination,
            ticket_file,
            format,
        } => {
            const COMMAND: &str = "resume-move-export";
            let result = (|| {
                let verifier = verifier()?;
                let ticket = read_record(&ticket_file)?;
                let key = signing_key()?;
                source_fence::export(
                    context,
                    &verifier,
                    source_fence::Export {
                        session: &session,
                        incarnation: &incarnation,
                        source_host: &source::local_host()?,
                        destination: &destination,
                        ticket: &ticket,
                        signing_key: &key,
                    },
                    &crate::resolve_tmux_bin(None),
                )
            })();
            match result {
                Ok(proof) => crate::render_single_success(COMMAND, format, &proof, |_| {
                    "stopped source proof available\n".into()
                }),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
        Command::Prepare {
            move_id,
            plan_file,
            cwd,
            check,
            format,
        } => {
            let result = (|| {
                let plan = read_plan(&plan_file)?;
                let candidate = destination::check_plan(
                    context,
                    &plan,
                    &cwd,
                    &move_id,
                    &source::local_host()?,
                )?;
                if check {
                    return Ok(candidate.view());
                }
                let prepared = destination::prepare(
                    context,
                    plan,
                    candidate,
                    &verifier()?,
                    &signing_key()?,
                    now(),
                )?;
                serde_json::to_value(prepared).map_err(|_| cli_refusal(Refusal::StateUnavailable))
            })();
            render("resume-move-prepare", format, result, |_| {
                "destination move preparation ready\n".into()
            })
        }
        Command::Stage {
            move_id,
            proof_file,
            format,
        } => {
            let result = (|| {
                let proof = read_record(&proof_file)?;
                destination::stage(
                    context,
                    &move_id,
                    &proof,
                    &verifier()?,
                    &source::local_host()?,
                    now(),
                )?;
                destination::status(context, &move_id)
            })();
            render("resume-move-stage", format, result, |_| {
                "destination move staged\n".into()
            })
        }
        Command::Status { move_id, format } => render(
            "resume-move-status",
            format,
            destination::status(context, &move_id),
            |_| "destination move status available\n".into(),
        ),
        Command::Abort {
            move_id,
            proof_file,
            format,
        } => {
            let result = (|| {
                let proof = proof_file.as_deref().map(read_record).transpose()?;
                let receipt = destination::abort(
                    context,
                    &move_id,
                    &verifier()?,
                    &signing_key()?,
                    &source::local_host()?,
                    proof.as_ref(),
                )?;
                serde_json::to_value(receipt).map_err(|_| cli_refusal(Refusal::StateUnavailable))
            })();
            render("resume-move-abort", format, result, |_| {
                "destination recorded a no-writer abort\n".into()
            })
        }
        Command::Recover {
            move_id,
            receipt_file,
            format,
        } => {
            const COMMAND: &str = "resume-move-recover";
            let result = (|| {
                let verifier = verifier()?;
                let receipt = read_record(&receipt_file)?;
                source_fence::recover(
                    context,
                    &verifier,
                    &move_id,
                    &source::local_host()?,
                    &receipt,
                )
            })();
            match result {
                Ok(()) => crate::render_single_success(
                    COMMAND,
                    format,
                    &json!({"move_id": move_id, "source_fence": "released"}),
                    |_| "source fence released\n".into(),
                ),
                Err(error) => crate::render_error(COMMAND, format, error),
            }
        }
    }
}

fn verifier() -> Result<Verifier, CliError> {
    let ssh_keygen = crate::non_empty_env("AGENT_SESSION_RESUME_MOVE_SSH_KEYGEN")
        .map(PathBuf::from)
        .ok_or_else(|| cli_refusal(Refusal::SshKeygenUnavailable))?;
    let allowlist = configured_path("AGENT_SESSION_RESUME_MOVE_ALLOWED_SIGNERS_FILE")?;
    Verifier::new(&ssh_keygen, &allowlist).map_err(cli_refusal)
}

fn configured_path(name: &str) -> Result<PathBuf, CliError> {
    crate::non_empty_env(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| cli_refusal(Refusal::TrustUnavailable))
}

fn read_record(path: &Path) -> Result<SignedRecord, CliError> {
    let bytes = coordination::read_private_file(path, MAX_RECORD_BYTES)
        .map_err(|_| cli_refusal(Refusal::ProofInvalid))?;
    serde_json::from_slice(&bytes).map_err(|_| cli_refusal(Refusal::ProofInvalid))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanEnvelope {
    schema_version: String,
    ok: bool,
    data: source::Plan,
}

fn read_plan(path: &Path) -> Result<source::Plan, CliError> {
    let malformed = || cli_refusal(Refusal::ProofInvalid);
    let bytes = coordination::read_private_file(path, MAX_RECORD_BYTES).map_err(|_| malformed())?;
    let envelope: PlanEnvelope = serde_json::from_slice(&bytes).map_err(|_| malformed())?;
    if envelope.schema_version != "cli.agent-session.resume-move-plan.v1" || !envelope.ok {
        return Err(malformed());
    }
    Ok(envelope.data)
}

fn signing_key() -> Result<PathBuf, CliError> {
    configured_path("AGENT_SESSION_RESUME_MOVE_SIGNING_KEY_FILE")
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn render(
    command: &'static str,
    format: OutputFormat,
    result: Result<Value, CliError>,
    render_text: fn(&Value) -> String,
) -> i32 {
    match result {
        Ok(data) => crate::render_single_success(command, format, &data, render_text),
        Err(error) => crate::render_error(command, format, error),
    }
}

pub(crate) fn move_preflight(
    context: &CliContext,
    args: &cli::StartArgs,
) -> Result<Value, CliError> {
    let host = source::local_host()?;
    let request = admission_request(args, &host)?;
    destination::check(context, &verifier()?, request, now())?;
    Ok(json!({
        "agent": request.provider.agent().as_str(),
        "provider_session_id": request.provider_session_id,
        "machine": host,
        "account": request.account,
        "move_id": request.move_id,
    }))
}

pub(crate) fn start(context: &CliContext, mut args: cli::StartArgs) -> Result<StartView, CliError> {
    let host = source::local_host()?;
    let request = admission_request(&args, &host)?;
    let verifier = verifier()?;
    let move_id = request.move_id.to_owned();
    let managed_session_id = request.managed_session_id();
    let thread = request.provider_session_id.to_owned();
    let account = request.account.to_owned();
    let launch_cwd =
        crate::resolve_cwd(Some(request.cwd)).map_err(|_| cli_refusal(Refusal::PathMismatch))?;
    let provider = request.provider;
    let agent = provider.agent();
    let agent_bin = crate::resolve_agent_bin(agent, args.agent_bin.as_deref());
    if provider == Provider::Codex {
        if !crate::codex_app_server::resume_transport_supported(&agent_bin) {
            return Err(start_resume::refusal("resume-unsupported"));
        }
        let roster = crate::codex_account::list_accounts()
            .map_err(|_| start_resume::refusal("resume-unsupported"))?;
        if !roster.iter().any(|item| item.account == account) {
            return Err(cli_refusal(Refusal::AccountMismatch));
        }
    }
    // Readiness is deterministic and must refuse before admission persists a writer.
    // A created replay returns its existing session and needs no new runtime.
    let replay = destination::check(context, &verifier, request, now())?;
    if provider == Provider::Codex
        && !replay
        && !crate::codex_app_server::resume_runtime_ready(&agent_bin, &context.state_dir)
    {
        return Err(start_resume::refusal("resume-unsupported"));
    }
    let moved = destination::admit(context, &verifier, request, now())?;
    let unknown = destination::outcome_unknown;
    let destination::Admission::New {
        provider_home,
        history,
        claude_target,
        _writer,
    } = moved
    else {
        let destination::Admission::Existing {
            session_id,
            incarnation,
        } = moved
        else {
            unreachable!("known move admission")
        };
        let record = crate::load_session_record(context, &session_id).map_err(|_| unknown())?;
        if record
            .runtime
            .as_ref()
            .is_none_or(|runtime| runtime.launch_id != incarnation)
        {
            return Err(unknown());
        }
        return Ok(StartView {
            format: args.format,
            result: crate::session_view(context, &record, None, None),
            prompt_delivery_observation: None,
        });
    };
    args.cwd = Some(launch_cwd);
    args.account = Some(account.clone());
    let claude_account = match provider {
        Provider::Codex => {
            args.initial_codex_account = Some(account);
            args.initial_codex_account_source = Some("explicit".into());
            args.initial_provider_config_dir = Some(provider_home);
            None
        }
        Provider::Claude => {
            let target = claude_target.ok_or_else(unknown)?;
            let config = target.prepare_account(&account).map_err(|_| unknown())?;
            claude::install_transcript(
                &target,
                &provider_home.join(&history.relative_path),
                &history,
            )
            .map_err(|_| unknown())?;
            args.initial_provider_config_dir = Some(config);
            Some(InitialClaudeAccount {
                account,
                selection_source: "explicit",
            })
        }
    };
    let session_id = managed_session_id;
    args.id = Some(session_id.clone());
    let cwd = args.cwd.as_deref().ok_or_else(unknown)?;
    let launch_args =
        crate::canonical_provider_resume_args(agent, &crate::display_path(cwd), &thread)
            .ok_or_else(unknown)?;
    let provider_resume = ProviderResume {
        provider: agent.as_str().into(),
        session_id: thread,
        captured_at: jiff::Timestamp::now().to_string(),
        capture_method: "cross-host-stop-proof-v1".into(),
        resume_args: launch_args.clone(),
        extra: Default::default(),
    };
    let view = lifecycle::attempt_created(context, &session_id, "start", || {
        crate::start_session_inner_unjournaled(
            context,
            args,
            crate::StartFailureDisposition::ReturnError,
            claude_account,
            Some(crate::InitialProviderPlan {
                launch_args,
                provider_resume: Some(provider_resume),
            }),
        )
    })
    .map_err(|_| unknown())?;
    let record = crate::load_session_record(context, &view.result.id).map_err(|_| unknown())?;
    let runtime = record.runtime.as_ref().ok_or_else(unknown)?;
    destination::mark_created(context, &move_id, &view.result.id, &runtime.launch_id)
        .map_err(|_| unknown())?;
    Ok(view)
}

fn admission_request<'a>(
    args: &'a cli::StartArgs,
    host: &'a str,
) -> Result<destination::AdmissionRequest<'a>, CliError> {
    let unsupported = || start_resume::refusal("resume-unsupported");
    let request = start_resume::Request::from_args(args);
    let provider = match request.validate()? {
        AgentKind::Codex => Provider::Codex,
        AgentKind::Claude => Provider::Claude,
        _ => return Err(unsupported()),
    };
    if args.via_console || !args.agent_args.is_empty() {
        return Err(unsupported());
    }
    if args
        .id
        .as_ref()
        .is_some_and(|id| crate::validate_id(id).is_err())
    {
        return Err(unsupported());
    }
    Ok(destination::AdmissionRequest {
        provider,
        session_id: args.id.as_deref(),
        destination_host: host,
        move_id: args.move_id.as_deref().ok_or_else(unsupported)?,
        account: args
            .account
            .as_deref()
            .ok_or_else(|| cli_refusal(Refusal::AccountMismatch))?,
        provider_session_id: args.resume.as_deref().ok_or_else(unsupported)?,
        cwd: args
            .cwd
            .as_deref()
            .ok_or_else(|| cli_refusal(Refusal::PathMismatch))?,
    })
}

//! Read-back and one-shot recovery of a keyed delivery on its owning host.
use super::*;
use clap::{Args, Subcommand, ValueHint};
use nils_common::cli_contract::OutputFormat;
use send_guard::key::{self, Claim, Composer, Evidence};
use std::path::PathBuf;

#[derive(Debug, Args)]
pub struct AttemptArgs {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// Read the result of a guarded send or notice.
    Status(Target),
    /// Submit the exact retained input of a proven non-submission once.
    Recover(Target),
}

#[derive(Debug, Args)]
struct Target {
    /// Managed session ID or an unambiguous prefix.
    session: String,
    /// Runtime launch ID from the original attempt.
    #[arg(long)]
    expect_runtime: String,
    /// Idempotency key from the original send or notice.
    #[arg(long)]
    idempotency_key: String,
    #[arg(long, value_hint = ValueHint::FilePath)]
    tmux_bin: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

impl AttemptArgs {
    pub(crate) fn command(&self) -> &'static str {
        match self.action {
            Action::Status(_) => "attempt.status",
            Action::Recover(_) => "attempt.recover",
        }
    }

    pub(crate) fn format(&self) -> OutputFormat {
        match &self.action {
            Action::Status(target) | Action::Recover(target) => target.format,
        }
    }
}

#[derive(Serialize)]
struct ResultSummary<'a> {
    id: &'a str,
    runtime_id: &'a str,
    #[serde(flatten)]
    outcome: Outcome,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    recovery_enter_sent: bool,
}

#[derive(Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Outcome {
    Submitted,
    NotSubmitted { composer: &'static str },
    Unknown,
}

impl Outcome {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Submitted => "submitted",
            Self::NotSubmitted { .. } => "not_submitted",
            Self::Unknown => "unknown",
        }
    }
}

pub(crate) fn run(context: &CliContext, args: AttemptArgs) -> i32 {
    let command = args.command();
    let format = args.format();
    let (target, recover) = match args.action {
        Action::Status(target) => (target, false),
        Action::Recover(target) => (target, true),
    };
    let result = (|| {
        let record = load_session_record(context, &target.session)?;
        let tmux = resolve_tmux_bin(target.tmux_bin.as_deref());
        let outcome = if recover {
            recover_retained(context, &record, &target, &tmux)?;
            Outcome::Unknown
        } else {
            status(context, &record, &target, &tmux)?
        };
        Ok((record, outcome))
    })();
    match result {
        Ok((record, outcome)) => crate::render_single_success(
            command,
            format,
            &ResultSummary {
                id: &record.id,
                runtime_id: &target.expect_runtime,
                outcome,
                recovery_enter_sent: recover,
            },
            |result| {
                let detail = match result.outcome {
                    Outcome::NotSubmitted { composer } => format!(" (composer {composer})"),
                    _ if result.recovery_enter_sent => " (one Enter sent)".to_string(),
                    _ => String::new(),
                };
                format!("{} {}{}\n", result.id, result.outcome.as_str(), detail)
            },
        ),
        Err(error) => crate::render_error(command, format, error),
    }
}

fn bound_claim(
    context: &CliContext,
    record: &SessionRecord,
    target: &Target,
) -> Result<Option<Claim>, CliError> {
    let claim = key::load(context, record, &target.idempotency_key)?;
    Ok(claim.filter(|_| {
        record.runtime.as_ref().is_some_and(|runtime| {
            !runtime.launch_id.is_empty() && runtime.launch_id == target.expect_runtime
        })
    }))
}

fn status(
    context: &CliContext,
    record: &SessionRecord,
    target: &Target,
    tmux: &Path,
) -> Result<Outcome, CliError> {
    let Some(claim) = bound_claim(context, record, target)? else {
        return Ok(Outcome::Unknown);
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    let guards = send_guard::GuardArgs::default();
    let admission = send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &guards,
        tmux_bin: tmux,
        verify_composer: false,
        deadline,
    };
    let outcome = match &claim.evidence {
        Evidence::Submitted => Outcome::Submitted,
        Evidence::Unknown => Outcome::Unknown,
        Evidence::NotSubmitted { composer } => Outcome::NotSubmitted {
            composer: match composer {
                Composer::Absent => "absent",
                Composer::Unknown => "unknown",
                Composer::Pasted {
                    input_sha256,
                    fence,
                    record_updated_at,
                } => match observe_input(&admission) {
                    Some(input) if input.is_empty() => "absent",
                    Some(input)
                        if key::input_digest(&input) == *input_sha256
                            && record.updated_at == *record_updated_at
                            && fence_matches(tmux, record, fence.as_deref()) =>
                    {
                        "exact"
                    }
                    _ => "unknown",
                },
            },
        },
        Evidence::NativePending {
            identity_sha256,
            client_id,
            input_sha256,
        } => {
            if native_match(record, identity_sha256)
                && codex_app_server::guarded::Client::connect(record, deadline)
                    .ok()
                    .and_then(|mut client| {
                        client.find_client_input(client_id, |text| {
                            key::input_digest(text) == *input_sha256
                        })
                    })
                    .is_some()
                && load_session_record(context, &record.id)
                    .is_ok_and(|current| native_match(&current, identity_sha256))
            {
                Outcome::Submitted
            } else {
                Outcome::Unknown
            }
        }
    };
    // The read is optimistic. Any concurrent claim change or runtime replacement
    // invalidates the observations made since the first snapshot.
    let unchanged = load_session_record(context, &record.id)
        .ok()
        .and_then(|current| {
            if matches!(outcome, Outcome::NotSubmitted { composer: "exact" })
                && current.updated_at != record.updated_at
            {
                return None;
            }
            bound_claim(context, &current, target).ok().flatten()
        })
        .as_ref()
        == Some(&claim);
    Ok(if unchanged { outcome } else { Outcome::Unknown })
}

fn native_match(record: &SessionRecord, expected: &str) -> bool {
    crate::serve::prompt_receipts::native_identity(record).as_deref() == Some(expected)
}

fn observe_input(admission: &send_guard::Admission<'_>) -> Option<String> {
    admission.identity("cleanup").ok()?;
    let target = format!("{}:0.0", admission.record.tmux_session);
    let pane = send_guard::capture_styled(
        admission.tmux_bin,
        &target,
        admission
            .deadline
            .saturating_duration_since(Instant::now())
            .min(crate::PANE_OBSERVATION_COMMAND_TIMEOUT),
    )?;
    let input = send_submit::exact_composer_input(&admission.record.agent, &pane)?;
    admission.identity("cleanup").ok()?;
    Some(input)
}

const FENCE_FORMAT: &str = "#{session_attached}|#{session_last_attached}|#{session_id}|#{pane_id}|#{pane_pid}|#{pane_in_mode}|#{pane_dead}";

fn pane_from_fence(fence: &str) -> Option<&str> {
    let mut fields = fence.split('|');
    let ["0", last_attached, session, pane, pid, "0", "0"] =
        std::array::from_fn(|_| fields.next().unwrap_or("!"))
    else {
        return None;
    };
    let is_number =
        |value: &str| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
    let now = Timestamp::now().as_second();
    if fields.next().is_some()
        || !(last_attached.is_empty()
            || last_attached
                .parse::<i64>()
                .is_ok_and(|value| value >= 0 && value < now))
        || !session.strip_prefix('$').is_some_and(is_number)
        || !pane.strip_prefix('%').is_some_and(is_number)
        || !is_number(pid)
    {
        return None;
    }
    Some(pane)
}

pub(super) fn terminal_fence(tmux: &Path, record: &SessionRecord) -> Option<String> {
    let mut command = ProcessCommand::new(tmux);
    let target = format!("{}:0.0", record.tmux_session);
    command.args(["display-message", "-t", &target, "-p", FENCE_FORMAT]);
    let output = crate::run_output_with_timeout_and_strict_cap(
        command,
        crate::PANE_OBSERVATION_COMMAND_TIMEOUT,
        512,
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let stamp = String::from_utf8(output.stdout).ok()?;
    let stamp = stamp.trim_end_matches('\n');
    pane_from_fence(stamp)?;
    Some(stamp.to_owned())
}

fn fence_matches(tmux: &Path, record: &SessionRecord, expected: Option<&str>) -> bool {
    expected.is_some_and(|expected| terminal_fence(tmux, record).as_deref() == Some(expected))
}

fn recover_retained(
    context: &CliContext,
    observed: &SessionRecord,
    target: &Target,
    tmux: &Path,
) -> Result<(), CliError> {
    let refused = || {
        CliError::runtime(
            "attempt-recovery-unproven",
            "recovery requires the original runtime and a proven unsubmitted input still in its composer",
            None,
        )
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    let lock =
        crate::acquire_session_record_lock_timed(context, &observed.id, Duration::from_secs(2))?;
    let mut manual_input = ManualInputSection::new(lock);
    let mut record = load_session_record(context, &observed.id)?;
    let mut claim = bound_claim(context, &record, target)?.ok_or_else(refused)?;
    let Evidence::NotSubmitted {
        composer:
            Composer::Pasted {
                input_sha256,
                fence: Some(fence),
                record_updated_at,
            },
    } = &claim.evidence
    else {
        return Err(refused());
    };
    if record.updated_at != *record_updated_at || !fence_matches(tmux, &record, Some(fence)) {
        return Err(refused());
    }
    let pane = pane_from_fence(fence).ok_or_else(refused)?.to_owned();
    let fence = fence.clone();
    let identity = record.clone();
    let guards = send_guard::GuardArgs {
        require_idle: true,
        ..Default::default()
    };
    let admission = send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record: &identity,
        args: &guards,
        tmux_bin: tmux,
        verify_composer: false,
        deadline,
    };
    crate::restart::ensure_not_fenced(context, &record)?;
    codex_app_server::ensure_manual_input_capability(context, &record)?;
    admission.check(None, "admission")?;
    let text = observe_input(&admission)
        .filter(|text| key::input_digest(text) == *input_sha256)
        .ok_or_else(refused)?;
    let keys = &[SpecialKey::Enter];
    crate::auth_incident::recovery::guard_input(context, &record, Some(&text), keys)?;
    if record.agent == "codex" {
        codex_account::ensure_terminal_input_allowed(&record)?;
    }
    let mut prepared = false;
    prepare_guarded_input(context, &mut record, true, &mut prepared)?;
    manual_input.arm(context, &record)?;
    let ingress = cli::permission::input_admission(context, &record, tmux, Some(&text), keys)?;
    let ingress = match ingress {
        Some(lock) => lock,
        None => activity::pending::probe_lock(context, &record.id)?.ok_or_else(refused)?,
    };
    let _ingress = activity::pending::hold(context, &record.id, ingress);
    admission.check(Some(&text), "recovery")?;
    // The old proof must be durably unavailable before Enter can reach tmux.
    claim.consume_recovery(context, &record)?;
    let mut command = ProcessCommand::new(tmux);
    command.args([
        "if-shell",
        "-F",
        "-t",
        &pane,
        &format!("#{{==:{FENCE_FORMAT},{fence}}}"),
        &format!("send-keys -t {pane} Enter"),
        "display-message -p recovery-refused",
    ]);
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = run_output_with_timeout_and_cap(command, remaining, 512).map_err(|_| refused())?;
    if !output.status.success() || !output.stdout.is_empty() {
        return Err(refused());
    }
    record.updated_at = Timestamp::now().to_string();
    write_session_record(context, &record)
}

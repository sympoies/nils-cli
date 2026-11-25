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
    /// Read the originally pasted text from stdin to check a wrapped attempt.
    #[arg(long)]
    text_stdin: bool,
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
    unsubmitted_attempt: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    recovery_enter_sent: bool,
}

#[derive(Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
enum Outcome {
    Submitted,
    NotSubmitted {
        composer: &'static str,
        #[serde(skip)]
        possibly_stranded: bool,
    },
    Unknown,
}

impl Outcome {
    fn holds_unsubmitted_attempt(&self) -> bool {
        matches!(
            self,
            Self::NotSubmitted {
                possibly_stranded: true,
                ..
            }
        )
    }

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
        let original = read_send_text(&None, target.text_stdin)?;
        let original = original
            .as_deref()
            .map(|text| text.trim_end_matches(['\r', '\n']));
        let record = load_session_record(context, &target.session)?;
        let tmux = resolve_tmux_bin(target.tmux_bin.as_deref());
        let outcome = if recover {
            recover_retained(context, &record, &target, &tmux, original)?;
            Outcome::Unknown
        } else {
            status(context, &record, &target, &tmux, original)?
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
                unsubmitted_attempt: outcome.holds_unsubmitted_attempt(),
                outcome,
                recovery_enter_sent: recover,
            },
            |result| {
                let detail = match result.outcome {
                    Outcome::NotSubmitted { composer, .. } => format!(" (composer {composer})"),
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
    original: Option<&str>,
) -> Result<Outcome, CliError> {
    let Some(claim) = bound_claim(context, record, target)? else {
        return Ok(Outcome::Unknown);
    };
    let outcome = match &claim.evidence {
        Evidence::Submitted => Outcome::Submitted,
        Evidence::Unknown => Outcome::Unknown,
        Evidence::NotSubmitted { composer } => {
            let composer = composer_status(context, record, tmux, composer, original);
            let composer = if composer == "occupied"
                && key::latest(context, record).as_ref() != Some(&claim)
            {
                "unknown"
            } else {
                composer
            };
            let possibly_stranded = matches!(composer, "exact" | "wrapped_match" | "occupied")
                || (composer == "unknown"
                    && matches!(
                        &claim.evidence,
                        Evidence::NotSubmitted {
                            composer: Composer::Pasted { .. }
                        }
                    )
                    && key::latest(context, record).as_ref() == Some(&claim));
            Outcome::NotSubmitted {
                composer,
                possibly_stranded,
            }
        }
        Evidence::NativePending {
            identity_sha256,
            client_id,
            input_sha256,
        } => {
            let deadline = Instant::now() + Duration::from_secs(3);
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
            if outcome.holds_unsubmitted_attempt() && current.updated_at != record.updated_at {
                return None;
            }
            bound_claim(context, &current, target).ok().flatten()
        })
        .as_ref()
        == Some(&claim);
    Ok(if unchanged { outcome } else { Outcome::Unknown })
}

/// A passive, keyless projection for a patrol. Occupancy after a retained
/// pre-Enter claim never extends the authority of the recovery operation.
pub(crate) fn unsubmitted_attempt(context: &CliContext, id: &str, tmux: &Path) -> bool {
    let observe = || {
        let record = load_session_record(context, id).ok()?;
        let claim = key::latest(context, &record)?;
        let Evidence::NotSubmitted { composer } = &claim.evidence else {
            return None;
        };
        let observed = composer_status(context, &record, tmux, composer, None);
        let status = Outcome::NotSubmitted {
            composer: observed,
            possibly_stranded: matches!(observed, "exact" | "wrapped_match" | "occupied")
                || (observed == "unknown" && matches!(composer, Composer::Pasted { .. })),
        };
        if !status.holds_unsubmitted_attempt() {
            return None;
        }
        let current = load_session_record(context, id).ok()?;
        (current.updated_at == record.updated_at
            && key::latest(context, &current).as_ref() == Some(&claim))
        .then_some(())
    };
    observe().is_some()
}

fn composer_status(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    composer: &Composer,
    original: Option<&str>,
) -> &'static str {
    let (input_sha256, fence, record_updated_at) = match composer {
        Composer::Pasted {
            input_sha256,
            fence,
            record_updated_at,
        } => (input_sha256, fence, record_updated_at),
        Composer::Absent => return "absent",
        Composer::Unknown => return "unknown",
    };
    let admission = send_guard::Admission {
        stopped_idle: &Default::default(),
        context,
        record,
        args: &send_guard::GuardArgs::default(),
        tmux_bin: tmux,
        verify_composer: false,
        deadline: Instant::now() + Duration::from_secs(3),
    };
    match observe_retained(&admission, input_sha256, original) {
        Some(Retained::Absent) => "absent",
        Some(Retained::Occupied) => "occupied",
        Some(retained)
            if record.updated_at == *record_updated_at
                && fence_matches(tmux, record, fence.as_deref()) =>
        {
            retained.label()
        }
        Some(_) if original.is_none() => "occupied",
        _ => "unknown",
    }
}

fn native_match(record: &SessionRecord, expected: &str) -> bool {
    crate::serve::prompt_receipts::native_identity(record).as_deref() == Some(expected)
}

enum Retained<'a> {
    Absent,
    Occupied,
    Exact(String),
    WrappedMatch(&'a str),
}

impl Retained<'_> {
    fn label(&self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Occupied => "occupied",
            Self::Exact(_) => "exact",
            Self::WrappedMatch(_) => "wrapped_match",
        }
    }
}

fn observe_retained<'a>(
    admission: &send_guard::Admission<'_>,
    digest: &str,
    original: Option<&'a str>,
) -> Option<Retained<'a>> {
    let original = match original {
        Some(text) if key::input_digest(text) != digest => return None,
        text => text,
    };
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
    let agent = &admission.record.agent;
    let retained = match send_submit::exact_composer_input(agent, &pane) {
        Some(input) if input.is_empty() => Retained::Absent,
        Some(input) if key::input_digest(&input) == digest => Retained::Exact(input),
        Some(_) if original.is_none() => Retained::Occupied,
        None if original.is_none() && send_submit::wrapped_composer_occupied(agent, &pane) => {
            Retained::Occupied
        }
        None => {
            let text = original?;
            let (input, _) = send_submit::wrapped_composer_layout(agent, &pane, text)?;
            if input != text {
                return None;
            }
            Retained::WrappedMatch(text)
        }
        _ => return None,
    };
    admission.identity("cleanup").ok()?;
    Some(retained)
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
    original: Option<&str>,
) -> Result<(), CliError> {
    let refused = || {
        CliError::runtime(
            "attempt-recovery-unproven",
            "recovery requires the original runtime and a proven unsubmitted input still in its composer",
            None,
        )
    };
    // Recovery uses the same terminal cursor proof as guarded send.
    let deadline = Instant::now() + super::guarded_hold_budget(observed, true);
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
    let text = match observe_retained(&admission, input_sha256, original) {
        Some(Retained::Exact(text)) => std::borrow::Cow::Owned(text),
        Some(Retained::WrappedMatch(text)) => std::borrow::Cow::Borrowed(text),
        _ => return Err(refused()),
    };
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
    let recovery_guards = send_guard::GuardArgs {
        // Claim creation bound the paste to an empty composer. Enable its
        // existing wrapped cursor proof, with no opaque-placeholder recovery.
        require_empty_composer: true,
        ..guards.clone()
    };
    send_guard::Admission {
        args: &recovery_guards,
        ..admission
    }
    .check(Some(&text), "recovery")?;
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

//! In-place restart of a managed session onto the installed provider binary.
//!
//! A restart keeps the session id, provider conversation, tmux runtime name,
//! mailbox and routing: it stops the provider at an idle safe point, retires
//! the stopped coordination incarnation, and resumes the exact conversation
//! or starts a failed pre-conversation Codex startup fresh under queued intent.

use std::path::{Path, PathBuf};
pub(crate) mod auth_recovery;
mod claude_exit;
mod fresh_startup;
#[cfg(not(all(test, target_os = "linux")))]
use crate::live_status_with_timeout;
pub(crate) use fresh_startup::{
    conversation_guards, eligible as fresh_startup, launch_plan, release_bootstrap,
};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;
#[cfg(not(all(test, target_os = "linux")))]
use std::{convert::identity as admission_deadline, thread::sleep, time::Instant};
#[cfg(all(test, target_os = "linux"))]
#[path = "restart_test_control.rs"]
pub(crate) mod test_control;
#[cfg(all(test, target_os = "linux"))]
use test_control::{Instant, admission_deadline, live_status_with_timeout, sleep};

use jiff::Timestamp;
use serde::Serialize;
use serde_json::json;

use crate::cli::{AgentKind, RestartArgs};
use crate::send_guard::{Admission, GuardArgs};
use crate::{CliContext, CliError, SessionRecord, coordination, load_session_record};

const COMMAND: &str = "restart";
const SCHEMA_VERSION: &str = "agent-session.restart.v1";
pub(crate) const DEFAULT_STOP_TIMEOUT_SECS: u64 = 30;
const POLL_INTERVAL: Duration = Duration::from_millis(200);
/// A stopped incarnation's last heartbeat stays fresh for up to 30 seconds.
const PRIOR_LEASE_WAIT: Duration = Duration::from_secs(35);
const VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// The provider version a restart observed for the runtime it launched.
const PROVIDER_VERSION_KEY: &str = "restart_provider_version";

#[derive(Debug, Default, Serialize)]
struct Incarnation {
    session_incarnation: Option<String>,
    generation: Option<u64>,
    pane_pid: Option<i64>,
    provider_version: Option<String>,
}

#[derive(Debug, Serialize)]
struct Receipt {
    schema_version: &'static str,
    id: String,
    agent: String,
    tmux_session: String,
    provider_session_id: Option<String>,
    outcome: &'static str,
    stop_method: Option<&'static str>,
    old: Incarnation,
    new: Incarnation,
    #[serde(skip_serializing_if = "Option::is_none")]
    native_notice: Option<crate::send::claude_notice::attach::Diagnostic>,
    started_at: String,
    stopped_at: Option<String>,
    relaunched_at: Option<String>,
    ready_at: Option<String>,
    finished_at: String,
}

pub(crate) fn run(context: &CliContext, args: RestartArgs) -> i32 {
    let format = args.format;
    match restart(context, &args) {
        Ok(receipt) => crate::render_single_success(COMMAND, format, &receipt, render_text),
        Err(err) => crate::render_error(COMMAND, format, err),
    }
}

fn render_text(receipt: &Receipt) -> String {
    let unknown = || "unknown".to_string();
    let mut text = format!(
        "{}: restarted {} -> {} (provider {})\n",
        receipt.id,
        receipt
            .old
            .session_incarnation
            .clone()
            .unwrap_or_else(unknown),
        receipt
            .new
            .session_incarnation
            .clone()
            .unwrap_or_else(unknown),
        receipt.new.provider_version.clone().unwrap_or_else(unknown),
    );
    if let Some(notice) = &receipt.native_notice {
        text.push_str(&notice.text());
    }
    text
}

pub(crate) enum ResumeStatus {
    Running,
    Stopped,
    Unknown { retryable: bool },
}

impl ResumeStatus {
    pub(crate) fn retry_details(self, record: &SessionRecord) -> serde_json::Value {
        let retryable = matches!(self, Self::Unknown { retryable: true });
        json!({
            "id": record.id,
            "retryable": retryable,
            "next_action": if retryable { "retry-resume" } else { "inspect-tmux" },
        })
    }
}

fn resume_probe(tmux: &Path, record: &SessionRecord, timeout: Duration) -> ResumeStatus {
    #[cfg(test)]
    if let Some(status) = crate::tmux_probe_fixture::read_status(tmux) {
        return if status == "running" {
            ResumeStatus::Running
        } else {
            ResumeStatus::Stopped
        };
    }
    let mut command = ProcessCommand::new(tmux);
    command.env("LC_ALL", "C").args([
        "has-session",
        "-t",
        &crate::exact_tmux_target(&record.tmux_session),
    ]);
    match crate::run_output_with_timeout(command, timeout) {
        Ok(output) if output.status.success() => ResumeStatus::Running,
        Ok(output) if crate::tmux_output_reports_absent(&output) => ResumeStatus::Stopped,
        Ok(output) => ResumeStatus::Unknown {
            retryable: output.status.code() == Some(1)
                && String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .any(|line| line.trim() == "server exited unexpectedly"),
        },
        Err(error) => ResumeStatus::Unknown {
            retryable: error.kind() == std::io::ErrorKind::TimedOut,
        },
    }
}

/// Resolve transient tmux shutdown uncertainty before deciding whether resume
/// is a no-op or may enter the existing stopped-runtime verification path.
/// The caller holds the session record lock throughout this bounded wait.
pub(crate) fn resume_status(tmux: &Path, record: &SessionRecord) -> ResumeStatus {
    if crate::recorded_runtime_is_from_prior_boot(record) {
        return ResumeStatus::Stopped;
    }
    let started = Instant::now();
    let budget = Duration::from_secs(2);
    loop {
        let remaining = budget.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return ResumeStatus::Unknown { retryable: true };
        }
        match resume_probe(tmux, record, remaining.min(Duration::from_secs(1))) {
            ResumeStatus::Unknown { retryable: true } => {}
            status => return status,
        }
        sleep(POLL_INTERVAL.min(budget.saturating_sub(started.elapsed())));
    }
}

/// The provider binary a resume of `record` launches.
pub(crate) fn resume_agent_bin(
    profile: Option<crate::DurableProfileResumeContext>,
    record: &SessionRecord,
    agent: AgentKind,
) -> PathBuf {
    profile
        .map(|profile| profile.agent_bin)
        .or_else(|| record.agent_bin.as_deref().map(PathBuf::from))
        .unwrap_or_else(|| crate::resolve_agent_bin(agent, None))
}

fn now() -> String {
    Timestamp::now().to_string()
}

fn launch_id(record: &SessionRecord) -> Option<String> {
    record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .filter(|launch_id| !launch_id.is_empty())
}

fn incarnation(record: &SessionRecord) -> Incarnation {
    let session_incarnation = launch_id(record);
    let provider_version = record
        .extra
        .get(PROVIDER_VERSION_KEY)
        .filter(|recorded| {
            session_incarnation.is_some()
                && recorded["launch_id"].as_str() == session_incarnation.as_deref()
        })
        .and_then(|recorded| recorded["version"].as_str())
        .map(str::to_string);
    Incarnation {
        session_incarnation,
        generation: record.runtime.as_ref().map(|runtime| runtime.generation),
        pane_pid: crate::persisted_tmux_runtime_identity(record)
            .ok()
            .flatten()
            .map(|identity| i64::from(identity.pane_pid)),
        provider_version,
    }
}

fn provider_version(agent_bin: &Path) -> Option<String> {
    let mut command = ProcessCommand::new(agent_bin);
    command.arg("--version").stdin(Stdio::null());
    let output = crate::run_output_with_timeout(command, VERSION_PROBE_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect())
}

fn refusal(code: &'static str, message: &'static str, record: &SessionRecord) -> CliError {
    CliError::runtime(code, message, Some(json!({ "id": record.id })))
}

/// A failure once the provider was asked to exit: the receipt says how far the
/// restart got and the printed command recovers or inspects the session.
fn failure(
    code: &'static str,
    message: &'static str,
    mut receipt: Receipt,
    outcome: &'static str,
    cause: Option<&str>,
    recovery: String,
) -> CliError {
    receipt.outcome = outcome;
    receipt.finished_at = now();
    let hint = format!("run `{recovery}` to recover the session");
    CliError::runtime(
        code,
        message,
        Some(json!({
            "id": receipt.id,
            "cause": cause,
            "recovery": recovery,
            "receipt": receipt,
        })),
    )
    .with_hint(hint)
}

fn fence_path(state_dir: &Path, session_id: &str) -> PathBuf {
    state_dir
        .join("coordination")
        .join(format!("restart-fence-{session_id}"))
}

/// Whether a live restart owns the exact incarnation's input. Notification
/// attempts check this while holding the registry lock (`registry_path`) that
/// the fence is written under.
pub(crate) fn fenced(registry_path: &Path, session_id: &str, incarnation: &str) -> bool {
    let Some(state_dir) = registry_path.parent().and_then(Path::parent) else {
        return false;
    };
    let Ok(fence) = std::fs::read_to_string(fence_path(state_dir, session_id)) else {
        return false;
    };
    fence.split_whitespace().next() == Some(incarnation) && owner_alive(&fence)
}

/// Whether the restart that wrote `fence` is still running.
fn owner_alive(fence: &str) -> bool {
    fence
        .split_whitespace()
        .nth(1)
        .and_then(|pid| pid.parse::<libc::pid_t>().ok())
        // SAFETY: signal 0 only probes whether the owning restart is alive.
        .is_some_and(|pid| unsafe { libc::kill(pid, 0) } == 0)
}

/// Refuse managed input during exclusive lifecycle input ownership.
pub(crate) fn ensure_not_fenced(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    ensure_not_restarting(context, record)?;
    crate::conversation::ensure_input_available(context, record)
}

/// The provider's native TUI must process the clear controller's own slash
/// command and first prompt. Those already pass through locked terminal input.
pub(crate) fn ensure_not_restarting(
    context: &CliContext,
    record: &SessionRecord,
) -> Result<(), CliError> {
    let registry = context.state_dir.join("coordination").join("registry.json");
    match launch_id(record) {
        Some(launch) if fenced(&registry, &record.id, &launch) => Err(refusal(
            "session-restarting",
            "a restart is stopping this session",
            record,
        )),
        _ => Ok(()),
    }
}

/// One restart's fence of an incarnation; dropping it releases the fence
/// while it still carries this restart's owner token.
pub(crate) struct RestartFence {
    context: CliContext,
    path: PathBuf,
    owner: String,
}

impl RestartFence {
    /// Under the registry lock notification attempts take, refuse a live
    /// restart of the session or an exact notification submission in
    /// progress, and fence any later attempt.
    pub(crate) fn acquire(
        context: &CliContext,
        record: &SessionRecord,
        launch: &str,
    ) -> Result<Self, CliError> {
        let locked = coordination::lock_registry_for_lifecycle(context)?;
        crate::conversation::ensure_input_available(context, record)?;
        coordination::ensure_notification_submission_not_in_progress(
            &locked.registry,
            &record.id,
            launch,
        )?;
        let path = fence_path(&context.state_dir, &record.id);
        if std::fs::read_to_string(&path).is_ok_and(|fence| owner_alive(&fence)) {
            return Err(refusal(
                "restart-in-progress",
                "another restart is stopping this session",
                record,
            ));
        }
        let owner = format!("{launch} {} {}\n", std::process::id(), uuid::Uuid::new_v4());
        crate::write_private_file(&path, owner.as_bytes())?;
        drop(locked);
        Ok(Self {
            context: context.clone(),
            path,
            owner,
        })
    }
}

impl Drop for RestartFence {
    fn drop(&mut self) {
        let Ok(_locked) = coordination::lock_registry_for_lifecycle(&self.context) else {
            return;
        };
        if std::fs::read_to_string(&self.path).is_ok_and(|fence| fence == self.owner) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A marker in a test's state directory that fails its next post-launch
/// persistence.
#[cfg(test)]
pub(super) const FAIL_POST_LAUNCH: &str = "restart-fail-post-launch";

fn restart(context: &CliContext, args: &RestartArgs) -> Result<Receipt, CliError> {
    #[cfg(all(test, target_os = "linux"))]
    let _clock = test_control::enter(context);
    let started_at = now();
    let tmux = crate::resolve_tmux_bin(args.tmux_bin.as_deref());
    let expected = load_session_record(context, &args.id)?;
    let plan = launch_plan(context, &expected)?;
    let agent = plan.agent;
    let provider_session_id = plan.conversation;
    let fresh = provider_session_id.is_none();
    // Stopping the runtime that hosts this very process would kill the
    // restart between its stop and its relaunch.
    if std::env::var("AGENT_SESSION_ID").is_ok_and(|id| id == expected.id) {
        return Err(refusal(
            "restart-own-session",
            "a session cannot restart the runtime that hosts this command",
            &expected,
        ));
    }
    let old = incarnation(&expected);
    let Some(old_launch) = old.session_incarnation.clone() else {
        return Err(refusal(
            "session-not-running",
            "restart requires a running session; use resume for a stopped one",
            &expected,
        ));
    };
    let (exit_requested, _fence) =
        request_provider_exit(context, &expected, agent, &old_launch, &tmux, args.force)?;
    let mut receipt = Receipt {
        schema_version: SCHEMA_VERSION,
        id: expected.id.clone(),
        agent: expected.agent.clone(),
        tmux_session: expected.tmux_session.clone(),
        provider_session_id,
        outcome: "ready",
        stop_method: None,
        old,
        new: Incarnation::default(),
        native_notice: None,
        started_at,
        stopped_at: None,
        relaunched_at: None,
        ready_at: None,
        finished_at: String::new(),
    };
    let glance = format!(
        "{} glance {}",
        crate::command_identity::executable(),
        expected.id
    );
    #[cfg(all(test, target_os = "linux"))]
    test_control::fire(context, test_control::Phase::ExitRequested, &old_launch);
    let stop_deadline = Instant::now() + Duration::from_secs(args.stop_timeout);
    let mut exit_dialogs = claude_exit::ExitDialogs::default();
    if exit_requested
        && wait_for_provider_exit(
            context,
            &expected,
            &tmux,
            stop_deadline,
            !args.force,
            &mut exit_dialogs,
        ) == ExitWait::Cancelled
    {
        receipt.outcome = "not-restarted";
        receipt.finished_at = now();
        return Err(CliError::runtime(
            "restart-exit-dialog",
            "not restarted: Claude's background-work exit dialog was cancelled",
            Some(json!({
                "id": receipt.id,
                "cause": claude_exit::Dialog::BackgroundWork.code(),
                "receipt": receipt,
            })),
        ));
    }

    let record_lock = match crate::acquire_session_record_lock_timed(
        context,
        &expected.id,
        Duration::from_secs(args.stop_timeout),
    ) {
        Ok(lock) => lock,
        Err(cause) => {
            return Err(failure(
                "restart-stop-failed",
                "the session record could not be locked to stop the runtime",
                receipt,
                "stop-failed",
                Some(cause.code()),
                glance,
            ));
        }
    };
    let mut record = match load_session_record(context, &expected.id).and_then(|record| {
        crate::ensure_same_session_identity(&expected, &record)?;
        Ok(record)
    }) {
        Ok(record)
            if launch_id(&record).as_deref() == Some(old_launch.as_str())
                && (!fresh || fresh_startup(context, &record)) =>
        {
            record
        }
        result => {
            // Another operation replaced the runtime during the exit wait;
            // that replacement is left exactly as it is.
            return Err(failure(
                "restart-superseded",
                "another operation replaced the session runtime during the restart",
                receipt,
                "superseded",
                result.err().as_ref().map(CliError::code),
                glance,
            ));
        }
    };
    let exited = crate::session_status(context, &tmux, &record) == "stopped";
    if !exited && !args.force && !fresh {
        if let Some(dialog) = exit_dialogs.last_dialog() {
            return Err(failure(
                "restart-exit-dialog-unresolved",
                dialog.unresolved_message(),
                receipt,
                "exit-dialog-unresolved",
                Some(dialog.code()),
                glance,
            ));
        }
        // Only --force may terminate a provider that did not leave on its own;
        // input admitted since the exit command must not be killed.
        return Err(failure(
            "restart-exit-timeout",
            "the provider did not exit within the stop timeout; the session was left running",
            receipt,
            "exit-timeout",
            None,
            glance,
        ));
    }
    let recovery = format!(
        "{} resume {}",
        crate::command_identity::executable(),
        record.id
    );
    if let Err(cause) =
        crate::auto_resume::cancel_for_manual_input_locked(context, &record.id, &now())
            .and_then(|()| crate::stop_session_runtime_locked(context, &mut record, &tmux))
    {
        return Err(failure(
            "restart-stop-failed",
            "the session runtime could not be verified stopped",
            receipt,
            "stop-failed",
            Some(cause.code()),
            glance,
        ));
    }
    receipt.stop_method = Some(if exit_requested && exited {
        "provider-exit"
    } else {
        "terminated"
    });
    receipt.stopped_at = Some(now());
    if let Err(cause) = coordination::retire_after_verified_stop(context, &record, &tmux)
        .and_then(|()| await_prior_lease(context, &record))
    {
        return Err(failure(
            "restart-prior-incarnation-live",
            "the session stopped but its prior coordination incarnation was not released",
            receipt,
            "relaunch-failed",
            Some(cause.code()),
            recovery,
        ));
    }
    receipt.relaunched_at = Some(now());
    let resumed = match crate::resume_session::resume_session_with_kind(
        context,
        record,
        &tmux,
        crate::provider_continuity::Kind::Restart,
    ) {
        Ok(resumed) => resumed,
        Err(cause) => {
            return Err(failure(
                "restart-relaunch-failed",
                "the session stopped but its provider could not be relaunched",
                receipt,
                "relaunch-failed",
                Some(cause.code()),
                recovery,
            ));
        }
    };
    let new_launch = resumed.session_incarnation.unwrap_or_default();
    receipt.new.session_incarnation = Some(new_launch.clone());
    receipt.new.generation = resumed.session_generation;
    let relaunched = match record_provider_version(context, &expected.id, agent, &new_launch) {
        Ok(relaunched) => relaunched,
        Err(cause) => {
            return Err(failure(
                "restart-post-launch-failed",
                "the provider was relaunched but the restart could not record it",
                receipt,
                "post-launch-failed",
                Some(cause.code()),
                glance,
            ));
        }
    };
    receipt.new = incarnation(&relaunched);
    receipt.native_notice = crate::send::claude_notice::attach::diagnostic(context, &relaunched);
    release_bootstrap(resumed.bootstrap, record_lock);

    let ready_deadline = Instant::now() + Duration::from_secs(args.ready_timeout);
    match await_ready(context, &relaunched, &new_launch, &tmux, ready_deadline) {
        Ok(()) => {
            if fresh {
                receipt.provider_session_id = load_session_record(context, &expected.id)?
                    .provider_resume
                    .map(|resume| resume.session_id);
            }
            receipt.ready_at = Some(now());
            receipt.finished_at = now();
            Ok(receipt)
        }
        Err(cause) => {
            let exited = cause == "provider-exited";
            Err(failure(
                "restart-not-ready",
                "the session was relaunched but its provider did not reach a ready prompt in time",
                receipt,
                if exited {
                    "relaunch-exited"
                } else {
                    "not-ready"
                },
                Some(&cause),
                if exited { recovery } else { glance },
            ))
        }
    }
}

/// Persist the installed provider version for the launched runtime.
fn record_provider_version(
    context: &CliContext,
    id: &str,
    agent: AgentKind,
    launch: &str,
) -> Result<SessionRecord, CliError> {
    #[cfg(test)]
    if context.state_dir.join(FAIL_POST_LAUNCH).exists() {
        return Err(CliError::runtime("session-write-failed", "injected", None));
    }
    let mut relaunched = load_session_record(context, id)?;
    let profile = crate::validate_durable_profile_resume_context(&relaunched)?;
    let version = provider_version(&resume_agent_bin(profile, &relaunched, agent));
    relaunched.extra.insert(
        PROVIDER_VERSION_KEY.to_string(),
        json!({ "launch_id": launch, "version": version }),
    );
    crate::write_session_record(context, &relaunched)?;
    Ok(relaunched)
}

/// A one-shot lifecycle command, constructed only after restart admission.
/// Its private field prevents general input paths from creating a quarantine
/// bypass from caller-supplied text.
pub(crate) struct ProviderExitCommand<'a>(&'static str, Option<&'a str>);

impl<'a> ProviderExitCommand<'a> {
    pub(crate) fn into_parts(self) -> (&'static str, Option<&'a str>) {
        (self.0, self.1)
    }
}

/// Only the lifecycle controllers, after idle admission and input fencing,
/// may submit the fixed provider exit command.
pub(crate) fn send_provider_exit(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    identity: Option<&crate::TmuxRuntimeIdentity>,
) -> Result<(), CliError> {
    let command = match record.agent.as_str() {
        "claude" => "/exit",
        "codex" => "/quit",
        _ => {
            return Err(refusal(
                "restart-unsupported",
                "provider cannot exit gracefully",
                record,
            ));
        }
    };
    crate::send::input::send_restart_exit_unlocked(
        context,
        record,
        ProviderExitCommand(command, identity.map(|identity| identity.pane_id.as_str())),
        tmux,
    )
}

pub(crate) fn check_exit_background_work(
    tmux: &Path,
    record: &SessionRecord,
) -> Result<(), CliError> {
    claude_exit::require_no_background_work(tmux, record, Instant::now() + Duration::from_secs(1))
}

#[derive(PartialEq, Eq)]
enum ExitWait {
    Inactive,
    Cancelled,
    TimedOut,
}

fn wait_for_provider_exit(
    context: &CliContext,
    expected: &SessionRecord,
    tmux: &Path,
    deadline: Instant,
    observe_dialogs: bool,
    dialogs: &mut claude_exit::ExitDialogs,
) -> ExitWait {
    while Instant::now() < deadline {
        let same_runtime = load_session_record(context, &expected.id)
            .is_ok_and(|current| crate::ensure_same_session_identity(expected, &current).is_ok());
        if !same_runtime || status_before(tmux, expected, deadline) == "stopped" {
            return ExitWait::Inactive;
        }
        if expected.agent == "claude"
            && observe_dialogs
            && dialogs.poll(context, expected, tmux, deadline)
        {
            return ExitWait::Cancelled;
        }
        sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    ExitWait::TimedOut
}

/// Wait without holding record, activity or provider locks so exit hooks
/// can finish. Inactivity here still needs fresh persisted process proof.
pub(crate) fn await_retained_exit(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Option<&'static str> {
    let deadline = Instant::now() + Duration::from_secs(DEFAULT_STOP_TIMEOUT_SECS);
    let mut dialogs = claude_exit::ExitDialogs::retaining_source();
    match wait_for_provider_exit(context, record, tmux, deadline, true, &mut dialogs) {
        ExitWait::Inactive => None,
        ExitWait::Cancelled => Some(claude_exit::Dialog::BackgroundWork.code()),
        ExitWait::TimedOut => Some(
            dialogs
                .last_dialog()
                .map_or("exit-timeout", |dialog| dialog.code()),
        ),
    }
}

/// Type the provider's own exit command into a verified idle, empty composer
/// once the incarnation's input is fenced. Returns whether it was sent and the
/// fence, held until the old runtime is stopped. Without `force` an unverified
/// session is refused before anything is typed or stopped.
fn request_provider_exit(
    context: &CliContext,
    expected: &SessionRecord,
    agent: AgentKind,
    launch: &str,
    tmux: &Path,
    force: bool,
) -> Result<(bool, RestartFence), CliError> {
    let _record_lock = crate::acquire_session_record_lock(context, &expected.id)?;
    let mut current = load_session_record(context, &expected.id)?;
    crate::ensure_same_session_identity(expected, &current)?;
    let _activity_lock = crate::activity::acquire_coordination_activity_lock(context, &current.id)?;
    let fresh = fresh_startup(context, &current);
    let status = crate::session_status(context, tmux, &current);
    if launch_id(&current).as_deref() != Some(launch)
        || (status != "running" && !(fresh && status == "stopped"))
    {
        return Err(refusal(
            "session-not-running",
            "restart requires a running session; use resume for a stopped one",
            &current,
        ));
    }
    if fresh && status == "stopped" {
        return Ok((false, RestartFence::acquire(context, &current, launch)?));
    }
    let guards = GuardArgs {
        expect_runtime: Some(launch.to_string()),
        require_idle: !fresh,
        require_empty_composer: true,
        ..GuardArgs::default()
    };
    let deadline = Instant::now() + crate::send_submit::HOLD_BUDGET;
    let admission = Admission {
        stopped_idle: &Default::default(),
        context,
        record: &current,
        args: &guards,
        tmux_bin: tmux,
        verify_composer: false,
        deadline: admission_deadline(deadline),
    };
    let admitted = match admission.check(None, "admission") {
        Ok(()) => true,
        // --force overrides idle quiescence only, never runtime identity.
        Err(cause) if cause.code().starts_with("send-identity-") => {
            return Err(CliError::runtime(
                "restart-identity-unverified",
                "the session pane does not prove this session's runtime identity",
                Some(json!({ "id": current.id, "cause": cause.code() })),
            ));
        }
        Err(_) if force => false,
        Err(cause) => {
            return Err(CliError::runtime(
                "restart-session-not-idle",
                "restart requires a verified idle session with an empty composer",
                Some(json!({ "id": current.id, "cause": cause.code() })),
            )
            .with_hint("retry once the session is idle, or pass --force to stop it now"));
        }
    };
    if agent == AgentKind::Claude && !force {
        claude_exit::require_no_background_work(tmux, &current, deadline)?;
    }
    let fence = RestartFence::acquire(context, &current, launch)?;
    if fresh {
        if crate::activity::pending::event_pending(context, &current.id)? {
            return Err(refusal(
                "restart-session-not-idle",
                "provider event ingestion is pending",
                &current,
            ));
        }
        crate::codex_app_server::verify_startup_quiescent(&current, admission.deadline).map_err(
            |cause| {
                CliError::runtime(
                    "restart-session-not-idle",
                    "failed startup is not verified quiescent",
                    Some(json!({"id": current.id, "cause": cause})),
                )
            },
        )?;
        crate::codex_account::authorize_restart_exit_locked(context, &mut current)?;
        return Ok((false, fence));
    }
    // Admission already refuses every provider without an observable prompt.
    if !admitted || !matches!(agent, AgentKind::Claude | AgentKind::Codex) {
        return Ok((false, fence));
    }
    crate::codex_account::authorize_restart_exit_locked(context, &mut current)?;
    crate::auto_resume::cancel_for_manual_input_locked(context, &current.id, &now())?;
    send_provider_exit(context, &current, tmux, None)?;
    Ok((true, fence))
}

#[cfg(test)]
pub(crate) fn test_request_provider_exit(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
) -> Result<bool, CliError> {
    let launch = record
        .runtime
        .as_ref()
        .expect("fixture runtime")
        .launch_id
        .as_str();
    request_provider_exit(context, record, AgentKind::Codex, launch, tmux, false)
        .map(|(sent, _fence)| sent)
}

/// Bounded wait for the stopped incarnation's lease. Retiring it normally
/// releases it at once; a heartbeat that cannot be proven stale is waited out
/// instead of refusing the relaunch as a live prior incarnation.
fn await_prior_lease(context: &CliContext, record: &SessionRecord) -> Result<(), CliError> {
    let incarnation = coordination::incarnation(record)?;
    let deadline = Instant::now() + PRIOR_LEASE_WAIT;
    while coordination::broker::heartbeat_fresh(context, &record.id, &incarnation, 0) {
        if Instant::now() >= deadline {
            return Err(refusal(
                "session-incarnation-conflict",
                "the prior coordination incarnation is still live",
                record,
            ));
        }
        sleep(POLL_INTERVAL);
    }
    Ok(())
}

/// Bounded wait until the relaunched provider reports an idle prompt through
/// the same admission a guarded idle send uses. Every observation and sleep is
/// capped by `deadline`. Returns the last refusal code.
fn await_ready(
    context: &CliContext,
    record: &SessionRecord,
    launch: &str,
    tmux: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let guards = GuardArgs {
        expect_runtime: Some(launch.to_string()),
        require_idle: true,
        ..GuardArgs::default()
    };
    #[cfg(all(test, target_os = "linux"))]
    test_control::fire(context, test_control::Phase::Ready, launch);
    let mut cause = "ready-timeout".to_string();
    while Instant::now() < deadline {
        if status_before(tmux, record, deadline) == "stopped" {
            return Err("provider-exited".to_string());
        }
        let current =
            load_session_record(context, &record.id).map_err(|error| error.code().to_string())?;
        crate::ensure_same_session_identity(record, &current)
            .map_err(|error| error.code().to_string())?;
        let admission = Admission {
            stopped_idle: &Default::default(),
            context,
            record: &current,
            args: &guards,
            tmux_bin: tmux,
            verify_composer: false,
            deadline: admission_deadline(
                deadline.min(Instant::now() + crate::send_submit::HOLD_BUDGET),
            ),
        };
        match crate::codex_account::ensure_input_allowed(&current)
            .and_then(|()| admission.check(None, "readiness"))
        {
            Ok(()) => return Ok(()),
            Err(refused) => cause = refused.code().to_string(),
        }
        sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())));
    }
    Err(cause)
}

/// `session_status` with its tmux probe capped by the time left before
/// `deadline`.
fn status_before(tmux: &Path, record: &SessionRecord, deadline: Instant) -> String {
    if crate::recorded_runtime_is_from_prior_boot(record) {
        return "stopped".to_string();
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    live_status_with_timeout(
        tmux,
        &record.tmux_session,
        crate::PANE_INPUT_COMMAND_TIMEOUT.min(remaining),
    )
}

#[cfg(test)]
#[path = "../tests/support/short_roots.rs"]
mod short_roots;

#[cfg(all(test, target_os = "linux"))]
#[path = "restart_tests.rs"]
mod tests;

#[cfg(test)]
mod codex_account_tests;

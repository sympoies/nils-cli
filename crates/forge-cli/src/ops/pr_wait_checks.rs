//! `pr wait-checks` atom — blocking poll on top of `pr.checks` until every
//! required check reaches a terminal state.
//!
//! Spec / ops: shares schema `cli.forge-cli.pr.checks.v1` with the snapshot
//! atom (the `data.duration_ms` field is populated here; the snapshot leaves
//! it `None`). Exit-code matrix per spec §"pr wait-checks":
//!
//! - all required `success` → `SUCCESS 0`
//! - any required `failure` / `cancelled` / `timed_out` → `RUNTIME 1`,
//!   `error.kind = "checks_failed"`
//! - deadline reached with checks still running → `UNAVAILABLE 69`,
//!   `error.kind = "checks_timeout"`
//! - deadline reached with nothing ever reported → `DATA 65`,
//!   `error.kind = "checks_not_registered"`
//!
//! The last case is why an empty snapshot is not terminal. "All required checks
//! passed" is vacuously true over an empty set, so treating it as terminal
//! reported success for a head no CI had touched — which is exactly what the
//! provider returns during its check-registration window after a push or
//! force-with-lease. Polling through that window is the point of this atom.
//! `--allow-no-checks`, or a repository `[checks] none = true` declaration in
//! `.forge-cli.toml`, opts a genuinely check-free repository out.
//!
//! A repository that can never register a check would otherwise look like a
//! slow one until the whole budget ran out. On GitHub, once
//! [`NO_CI_GRACE_PERIOD`] has passed with nothing reported for the head, the
//! loop asks whether the repository has any Actions workflows at all; when it
//! has none it stops early with `checks_not_registered`. External CI apps are
//! still covered, because any status or check run they post counts as
//! registered and ends the empty streak.
//!
//! Polling uses `std::thread::sleep`; the implementation owns the clock
//! through the [`Clock`] trait so tests can drive deterministic snapshot
//! sequences.

use std::time::{Duration, Instant};

use nils_common::cli_contract::{Envelope, EnvelopeError, OutputFormat, schema_version_for};
use serde_json::json;

use crate::backend::{BackendCall, BackendProgram, BackendRunner, DryRunPayload};
use crate::cli::{BINARY, GlobalFlags, PrChecksArgs, PrWaitChecksArgs};
use crate::config::ForgeConfig;
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::pr_checks::{self, PrChecksPayload, SCHEMA, SCHEMA_VERSION};
use crate::ops::pr_create::find_git_toplevel;
use crate::ops::required_check_gate::CheckPresence;
use crate::provider::{Provider, ProviderContext, detect, git_remote_url};
use crate::rate_limit::default_runner;

/// How long a GitHub head may report nothing at all before the loop checks
/// whether the repository could register a check in the first place. Long
/// enough for an external CI app to post its first status after a push.
pub const NO_CI_GRACE_PERIOD: Duration = Duration::from_secs(60);

/// Actionable next step attached to every `checks_not_registered` emission.
pub const NOT_REGISTERED_HINT: &str = "if this repository genuinely configures no CI, declare \
     `[checks] none = true` with a `none_reason` in .forge-cli.toml, or pass --allow-no-checks";

/// Trait abstracting `now()` and `sleep` so tests can step time without
/// `std::thread::sleep`.
pub trait Clock {
    fn now(&self) -> Instant;
    fn sleep(&self, dur: Duration);
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

/// Blanket impl so a shared reference to a clock is itself a clock, letting
/// wrappers borrow a clock without taking ownership.
impl<T: Clock + ?Sized> Clock for &T {
    fn now(&self) -> Instant {
        (**self).now()
    }
    fn sleep(&self, dur: Duration) {
        (**self).sleep(dur)
    }
}

pub fn run(
    global: &GlobalFlags,
    args: PrWaitChecksArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let runner = default_runner();
    let clock = SystemClock;
    let mut args = args;
    if !args.allow_no_checks
        && let Ok(workdir) = std::env::current_dir()
    {
        let cfg = ForgeConfig::load_layered(&workdir, find_git_toplevel(&workdir).as_deref());
        args.allow_no_checks = cfg.declared_no_checks_reason().is_some();
    }
    run_with(&runner, &clock, global, &args, format, git_remote_url)
}

pub fn run_with<R: BackendRunner, C: Clock, F: Fn(&str) -> Option<String>>(
    runner: &R,
    clock: &C,
    global: &GlobalFlags,
    args: &PrWaitChecksArgs,
    format: OutputFormat,
    remote_url_lookup: F,
) -> Result<i32, ForgeError> {
    let ctx = detect(
        global.provider_hint(),
        &global.remote,
        global.repo.as_deref(),
        remote_url_lookup,
    )?;
    let snapshot_args = PrChecksArgs {
        id: args.id.clone(),
        required_only: args.required_only,
    };
    if global.dry_run {
        return Ok(emit_dry_run(&ctx, args, &snapshot_args, format));
    }

    match poll_until_terminal_or_timeout(runner, clock, global, &ctx, args, &snapshot_args)? {
        WaitOutcome::Success(snapshot) => Ok(emit_success(
            schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
            snapshot,
            format,
            render_text,
        )),
        WaitOutcome::Failed(snapshot) => Ok(emit_failure(
            snapshot,
            "checks_failed",
            "required checks did not reach success",
            nils_common::cli_contract::exit::RUNTIME,
            format,
        )),
        WaitOutcome::TimedOut(snapshot) => Ok(emit_timeout(snapshot, format)),
        WaitOutcome::NotRegistered(snapshot, cause) => Ok(emit_failure_with_hint(
            snapshot,
            "checks_not_registered",
            match cause {
                NotRegisteredCause::BudgetExpired => {
                    "no checks registered for this head within the timeout"
                }
                NotRegisteredCause::NoWorkflows => {
                    "no checks registered for this head and the repository has no \
                     GitHub Actions workflows"
                }
            },
            Some(NOT_REGISTERED_HINT),
            nils_common::cli_contract::exit::DATA,
            format,
        )),
    }
}

/// Internal outcome enum for the polling loop; the macro consumes the same
/// data through [`compute`] without going through the envelope emitters.
pub enum WaitOutcome {
    Success(PrChecksPayload),
    Failed(PrChecksPayload),
    TimedOut(PrChecksPayload),
    /// No check ever registered for this head. Distinct from
    /// [`WaitOutcome::TimedOut`], where checks existed but had not finished:
    /// the two have different causes and different fixes, so they must not
    /// share an error kind.
    NotRegistered(PrChecksPayload, NotRegisteredCause),
}

/// Why a wait ended as [`WaitOutcome::NotRegistered`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotRegisteredCause {
    /// The whole budget expired with the gating set still empty.
    BudgetExpired,
    /// Nothing registered within [`NO_CI_GRACE_PERIOD`] and the GitHub
    /// repository has no Actions workflows, so nothing ever could.
    NoWorkflows,
}

/// Macro-facing entry point: poll until terminal or timeout and return the
/// final snapshot tagged with its outcome. Caller (e.g. `pr deliver`) decides
/// whether the macro continues, short-circuits with `checks_failed`, or
/// short-circuits with `checks_timeout`.
pub fn compute<R: BackendRunner, C: Clock>(
    runner: &R,
    clock: &C,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    args: &PrWaitChecksArgs,
) -> Result<WaitOutcome, ForgeError> {
    let snapshot_args = PrChecksArgs {
        id: args.id.clone(),
        required_only: args.required_only,
    };
    poll_until_terminal_or_timeout(runner, clock, global, ctx, args, &snapshot_args)
}

fn poll_until_terminal_or_timeout<R: BackendRunner, C: Clock>(
    runner: &R,
    clock: &C,
    global: &GlobalFlags,
    ctx: &ProviderContext,
    args: &PrWaitChecksArgs,
    snapshot_args: &PrChecksArgs,
) -> Result<WaitOutcome, ForgeError> {
    let timeout = args.timeout;
    let interval = args.interval;
    let presence = CheckPresence::from_allow_no_checks(args.allow_no_checks);
    let start = clock.now();
    let deadline = start + timeout;

    // "Never registered" is a property of the whole run, not of the last read.
    // `pr_checks` normalizes a transient "no checks reported" into an empty
    // successful snapshot, so classifying from the final poll alone would turn a
    // run that had been watching real pending checks into `checks_not_registered`
    // — flipping a retryable UNAVAILABLE 69 into a fatal DATA 65 on exactly the
    // slow-registration case this exists for.
    let mut ever_saw_a_check = false;
    // Probe for Actions workflows at most once per run.
    let mut workflows_probed = false;

    loop {
        let snapshot = pr_checks::snapshot(runner, global, ctx, snapshot_args)?;
        let snapshot = with_duration(snapshot, ms_between(start, clock.now()));
        ever_saw_a_check =
            ever_saw_a_check || !crate::ops::required_check_gate::nothing_was_checked(&snapshot);
        if is_terminal(&snapshot, presence) {
            if snapshot.state == "success" {
                return Ok(WaitOutcome::Success(snapshot));
            }
            return Ok(WaitOutcome::Failed(snapshot));
        }
        let now = clock.now();
        if !ever_saw_a_check
            && !workflows_probed
            && presence == CheckPresence::Required
            && ctx.provider == Provider::GitHub
            && now.saturating_duration_since(start) >= NO_CI_GRACE_PERIOD
        {
            workflows_probed = true;
            if github_repo_has_no_workflows(runner, ctx) {
                let snapshot = with_duration(snapshot, ms_between(start, now));
                return Ok(WaitOutcome::NotRegistered(
                    snapshot,
                    NotRegisteredCause::NoWorkflows,
                ));
            }
        }
        if now >= deadline {
            let expired = with_duration(snapshot, ms_between(start, now));
            // Report *why* the budget expired. Nothing ever registered is a
            // different problem, with a different fix, from checks that ran long.
            if !ever_saw_a_check {
                return Ok(WaitOutcome::NotRegistered(
                    expired,
                    NotRegisteredCause::BudgetExpired,
                ));
            }
            return Ok(WaitOutcome::TimedOut(expired));
        }
        let remaining = deadline.saturating_duration_since(now);
        let sleep_for = std::cmp::min(interval, remaining);
        if sleep_for.is_zero() {
            continue;
        }
        clock.sleep(sleep_for);
    }
}

/// True only when GitHub positively reports `total_count: 0` Actions workflows
/// for the repository. Any failure to ask — no repo slug, a permission error,
/// an unexpected body — answers `false`, which keeps the ordinary wait.
fn github_repo_has_no_workflows<R: BackendRunner>(runner: &R, ctx: &ProviderContext) -> bool {
    let Some(call) = build_github_workflows_call(ctx) else {
        return false;
    };
    let Ok(output) = runner.run(&call) else {
        return false;
    };
    serde_json::from_str::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|body| body.get("total_count").and_then(serde_json::Value::as_u64))
        == Some(0)
}

/// `gh api repos/{owner}/{repo}/actions/workflows?per_page=1`, or `None`
/// when the context carries no `owner/name` slug to address.
fn build_github_workflows_call(ctx: &ProviderContext) -> Option<BackendCall> {
    let repo = ctx.repo.as_deref()?;
    let (owner, name) = repo.split_once('/')?;
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return None;
    }
    let mut argv = vec![
        std::ffi::OsString::from("api"),
        std::ffi::OsString::from(format!("repos/{repo}/actions/workflows?per_page=1")),
    ];
    ctx.push_github_api_hostname(&mut argv);
    Some(BackendCall::new(BackendProgram::Gh, argv))
}

fn ms_between(start: Instant, end: Instant) -> u64 {
    end.duration_since(start).as_millis() as u64
}

fn with_duration(mut snapshot: PrChecksPayload, duration_ms: u64) -> PrChecksPayload {
    snapshot.duration_ms = Some(duration_ms);
    snapshot
}

/// Terminal iff nothing in the gating set is still pending **and** the gating
/// set exists at all.
///
/// An empty set is not a terminal pass. `gh pr checks --required` reports "no
/// required checks reported" during the provider's check-registration window
/// after a push or force-with-lease, which [`pr_checks`] normalizes to an empty
/// successful snapshot; polling through that window is the entire point of this
/// atom, so it keeps waiting instead of declaring the head green. A repository
/// that configures no checks passes [`CheckPresence::Optional`] so the loop
/// still terminates immediately.
fn is_terminal(snapshot: &PrChecksPayload, presence: CheckPresence) -> bool {
    if presence == CheckPresence::Required
        && crate::ops::required_check_gate::nothing_was_checked(snapshot)
    {
        return false;
    }
    snapshot.pending.is_empty()
}

fn emit_timeout(snapshot: PrChecksPayload, format: OutputFormat) -> i32 {
    emit_failure(
        snapshot,
        "checks_timeout",
        "deadline reached before required checks became terminal",
        nils_common::cli_contract::exit::UNAVAILABLE,
        format,
    )
}

/// Emit a failure envelope that *still carries* the snapshot payload so
/// callers can inspect failed/pending lists even though `ok = false`.
fn emit_failure(
    snapshot: PrChecksPayload,
    kind: &'static str,
    message: &str,
    exit_code: i32,
    format: OutputFormat,
) -> i32 {
    emit_failure_with_hint(snapshot, kind, message, None, exit_code, format)
}

fn emit_failure_with_hint(
    snapshot: PrChecksPayload,
    kind: &'static str,
    message: &str,
    hint: Option<&str>,
    exit_code: i32,
    format: OutputFormat,
) -> i32 {
    let schema = schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION);
    let envelope = Envelope {
        schema_version: schema,
        ok: false,
        data: Some(snapshot.clone()),
        warnings: Vec::new(),
        error: Some(EnvelopeError {
            code: kind.to_string(),
            message: message.to_string(),
            hint: hint.map(str::to_string),
            details: Some(json!({
                "state": snapshot.state,
                "required_count": snapshot.required_count,
                "success_count": snapshot.success_count,
                "duration_ms": snapshot.duration_ms,
            })),
        }),
    };
    match format {
        OutputFormat::Json => {
            let serialized =
                serde_json::to_string(&envelope).unwrap_or_else(|_| String::from("{\"ok\":false}"));
            println!("{serialized}");
        }
        OutputFormat::Text => {
            eprintln!("error: {kind}: {message}");
            if let Some(hint) = hint {
                eprintln!("hint: {hint}");
            }
            render_text(&snapshot);
        }
    }
    exit_code
}

fn emit_dry_run(
    ctx: &ProviderContext,
    wait_args: &PrWaitChecksArgs,
    snapshot_args: &PrChecksArgs,
    format: OutputFormat,
) -> i32 {
    let call = pr_checks::build_dry_run_call(ctx, snapshot_args);
    let payload = DryRunPlan {
        inner: DryRunPayload::new(ctx.provider, &call),
        timeout_ms: wait_args.timeout.as_millis() as u64,
        interval_ms: wait_args.interval.as_millis() as u64,
        required_only: snapshot_args.required_only,
    };
    emit_success(
        schema_version_for(BINARY, SCHEMA, SCHEMA_VERSION),
        payload,
        format,
        |p| {
            println!(
                "would run: {plan} (interval={interval}ms, timeout={timeout}ms, required_only={required})",
                plan = p.inner.plan.join(" "),
                interval = p.interval_ms,
                timeout = p.timeout_ms,
                required = p.required_only,
            );
        },
    )
}

#[derive(serde::Serialize)]
struct DryRunPlan {
    #[serde(flatten)]
    inner: DryRunPayload,
    timeout_ms: u64,
    interval_ms: u64,
    required_only: bool,
}

#[cfg(test)]
fn timed_out_placeholder(ctx: &ProviderContext) -> PrChecksPayload {
    PrChecksPayload {
        provider: ctx.provider.as_str(),
        state: "pending",
        required_count: 0,
        success_count: 0,
        failed: Vec::new(),
        pending: Vec::new(),
        checks: Vec::new(),
        duration_ms: None,
        warnings: Vec::new(),
    }
}

fn render_text(payload: &PrChecksPayload) {
    println!(
        "{state} [{provider}] required={required} success={success} failed={fcount} pending={pcount} elapsed_ms={elapsed}",
        state = payload.state,
        provider = payload.provider,
        required = payload.required_count,
        success = payload.success_count,
        fcount = payload.failed.len(),
        pcount = payload.pending.len(),
        elapsed = payload.duration_ms.unwrap_or(0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{BackendCall, BackendSuccess};
    use crate::provider::{DetectionSource, Provider};
    use std::cell::RefCell;
    use std::time::Duration;

    fn make_ctx(p: Provider) -> ProviderContext {
        ProviderContext {
            provider: p,
            host: "example.com".into(),
            source: DetectionSource::Flag,
            repo: None,
        }
    }

    fn make_global() -> GlobalFlags {
        GlobalFlags {
            format: None,
            remote: "origin".into(),
            provider: Some(crate::cli::ProviderFlag::Github),
            host: None,
            repo: None,
            store_root: None,
            dry_run: false,
        }
    }

    fn make_args(id: &str) -> PrWaitChecksArgs {
        PrWaitChecksArgs {
            id: id.into(),
            timeout: Duration::from_secs(5),
            interval: Duration::from_millis(1),
            required_only: true,
            allow_no_checks: false,
        }
    }

    struct StubRunner {
        outputs: RefCell<Vec<String>>,
    }

    impl BackendRunner for StubRunner {
        fn run(&self, _call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            let mut outs = self.outputs.borrow_mut();
            let next = if outs.is_empty() {
                String::new()
            } else {
                outs.remove(0)
            };
            Ok(BackendSuccess {
                stdout: next,
                stderr: String::new(),
            })
        }
    }

    /// Test clock with a manually-advanced now() and a sleep() that advances
    /// the clock instead of actually sleeping.
    struct StepClock {
        now: RefCell<Instant>,
    }

    impl StepClock {
        fn new() -> Self {
            Self {
                now: RefCell::new(Instant::now()),
            }
        }
    }

    impl Clock for StepClock {
        fn now(&self) -> Instant {
            *self.now.borrow()
        }
        fn sleep(&self, dur: Duration) {
            let mut cell = self.now.borrow_mut();
            *cell += dur;
        }
    }

    /// A required-only snapshot costs *two* backend calls — the general check
    /// list and the `--required` list that names which of them gate — so a stub
    /// must answer both. Queueing one output leaves the required list empty,
    /// which used to read as a vacuous "success" and is now correctly rejected
    /// as an unregistered head.
    const ONE_REQUIRED_PASS: &str =
        r#"[{"name":"build","bucket":"pass","conclusion":"success","isRequired":true}]"#;

    #[test]
    fn succeeds_when_first_snapshot_is_terminal_success() {
        let runner = StubRunner {
            outputs: RefCell::new(vec![
                ONE_REQUIRED_PASS.into(),
                // the `--required` list, naming `build` as gating
                ONE_REQUIRED_PASS.into(),
            ]),
        };
        let clock = StepClock::new();
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = make_args("1");
        // We can't easily call run_with without re-implementing the provider
        // detection plumb-through; exercise the pieces directly.
        let snapshot_args = PrChecksArgs {
            id: args.id.clone(),
            required_only: args.required_only,
        };
        let snapshot = pr_checks::snapshot(&runner, &global, &ctx, &snapshot_args).unwrap();
        assert_eq!(snapshot.state, "success");
        // Without this the test passes on a single-output stub, where the
        // `--required` list comes back empty, `build` is never marked required,
        // and "terminal success" is true only over an empty gating set. Assert
        // the gating set is real so the doubled fixture is load-bearing.
        assert_eq!(
            snapshot.required_count, 1,
            "the fixture must produce a real gating set, not a vacuous one"
        );
        assert_eq!(snapshot.success_count, 1);
        let snapshot = with_duration(snapshot, ms_between(clock.now(), clock.now()));
        assert!(is_terminal(&snapshot, CheckPresence::Required));
    }

    #[test]
    fn pending_then_success_succeeds_on_second_poll() {
        const ONE_REQUIRED_PENDING: &str =
            r#"[{"name":"build","bucket":"pending","isRequired":true}]"#;
        let runner = StubRunner {
            outputs: RefCell::new(vec![
                // poll 1: general list, then the `--required` list
                ONE_REQUIRED_PENDING.into(),
                ONE_REQUIRED_PENDING.into(),
                // poll 2: the same check, now passing
                ONE_REQUIRED_PASS.into(),
                ONE_REQUIRED_PASS.into(),
            ]),
        };
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = PrChecksArgs {
            id: "1".into(),
            required_only: true,
        };
        let s1 = pr_checks::snapshot(&runner, &global, &ctx, &args).unwrap();
        assert_eq!(s1.state, "pending");
        assert!(!is_terminal(&s1, CheckPresence::Required));
        let s2 = pr_checks::snapshot(&runner, &global, &ctx, &args).unwrap();
        assert_eq!(s2.state, "success");
        assert!(is_terminal(&s2, CheckPresence::Required));
    }

    /// An empty gating set is the provider saying "I have not registered any
    /// checks for this head yet", which is the opposite of "they all passed".
    /// Treating it as terminal is what let delivery report success against a
    /// head no CI had touched.
    #[test]
    fn an_empty_gating_set_is_not_terminal() {
        let runner = StubRunner {
            outputs: RefCell::new(vec!["[]".into()]),
        };
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = PrChecksArgs {
            id: "1".into(),
            required_only: true,
        };
        let snapshot = pr_checks::snapshot(&runner, &global, &ctx, &args).unwrap();
        // The read surface still reports "nothing is failing" — that stays true.
        assert_eq!(snapshot.state, "success");
        assert_eq!(snapshot.required_count, 0);
        // The wait loop must not accept it as a terminal pass.
        assert!(
            !is_terminal(&snapshot, CheckPresence::Required),
            "an unregistered head must keep the poll loop running"
        );
    }

    /// A repository that genuinely has no checks must still terminate, or
    /// `pr wait-checks` would hang for its whole timeout on every invocation.
    #[test]
    fn an_empty_gating_set_is_terminal_when_no_checks_are_allowed() {
        let runner = StubRunner {
            outputs: RefCell::new(vec!["[]".into()]),
        };
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = PrChecksArgs {
            id: "1".into(),
            required_only: true,
        };
        let snapshot = pr_checks::snapshot(&runner, &global, &ctx, &args).unwrap();
        assert!(is_terminal(&snapshot, CheckPresence::Optional));
    }

    /// Checks that never register must expire as `checks_not_registered`, not
    /// as `checks_timeout`: the two need different fixes, and "waited and
    /// nothing ever appeared" is the actionable one.
    #[test]
    fn checks_that_never_register_time_out_as_not_registered() {
        let runner = StubRunner {
            outputs: RefCell::new(vec!["[]".into(), "[]".into(), "[]".into()]),
        };
        let clock = StepClock::new();
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = PrWaitChecksArgs {
            timeout: Duration::from_millis(3),
            interval: Duration::from_millis(1),
            ..make_args("1")
        };
        let snapshot_args = PrChecksArgs {
            id: args.id.clone(),
            required_only: args.required_only,
        };
        let outcome =
            poll_until_terminal_or_timeout(&runner, &clock, &global, &ctx, &args, &snapshot_args)
                .expect("poll must not error");
        assert!(
            matches!(
                outcome,
                WaitOutcome::NotRegistered(_, NotRegisteredCause::BudgetExpired)
            ),
            "an always-empty snapshot must expire as not-registered"
        );
    }

    /// "Never registered" is a property of the run, not of the last poll.
    ///
    /// `pr_checks` normalizes a transient "no checks reported" into an empty
    /// successful snapshot, so a run that watched real pending checks and then
    /// hit one empty read at the deadline must still expire as `checks_timeout`
    /// (UNAVAILABLE 69, retryable) — not as `checks_not_registered` (DATA 65,
    /// fatal). Classifying from the final snapshot alone flips exactly the
    /// slow-registration case this feature exists for onto the wrong side of
    /// the retry boundary.
    #[test]
    fn a_transient_empty_read_at_the_deadline_is_still_a_timeout() {
        const ONE_REQUIRED_PENDING: &str =
            r#"[{"name":"build","bucket":"pending","isRequired":true}]"#;
        let runner = StubRunner {
            outputs: RefCell::new(vec![
                // poll 1: a real pending required check
                ONE_REQUIRED_PENDING.into(),
                ONE_REQUIRED_PENDING.into(),
                // poll 2, at the deadline: a transient empty read
                "[]".into(),
                "[]".into(),
            ]),
        };
        let clock = StepClock::new();
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = PrWaitChecksArgs {
            timeout: Duration::from_millis(2),
            interval: Duration::from_millis(2),
            ..make_args("1")
        };
        let snapshot_args = PrChecksArgs {
            id: args.id.clone(),
            required_only: args.required_only,
        };

        let outcome =
            poll_until_terminal_or_timeout(&runner, &clock, &global, &ctx, &args, &snapshot_args)
                .expect("poll must not error");

        assert!(
            matches!(outcome, WaitOutcome::TimedOut(_)),
            "a run that saw checks must expire as a timeout, not as not-registered"
        );
    }

    /// The registration window is the real scenario: nothing at first, then the
    /// provider registers the checks and they pass.
    #[test]
    fn checks_registering_late_still_succeed() {
        let runner = StubRunner {
            outputs: RefCell::new(vec![
                // poll 1: the registration window — nothing reported yet
                "[]".into(),
                "[]".into(),
                // poll 2: the checks have registered and passed
                ONE_REQUIRED_PASS.into(),
                ONE_REQUIRED_PASS.into(),
            ]),
        };
        let clock = StepClock::new();
        let ctx = make_ctx(Provider::GitHub);
        let global = make_global();
        let args = make_args("1");
        let snapshot_args = PrChecksArgs {
            id: args.id.clone(),
            required_only: args.required_only,
        };
        let outcome =
            poll_until_terminal_or_timeout(&runner, &clock, &global, &ctx, &args, &snapshot_args)
                .expect("poll must not error");
        match outcome {
            WaitOutcome::Success(snapshot) => assert_eq!(snapshot.required_count, 1),
            _ => panic!("late registration must still reach success"),
        }
    }

    /// Routes `gh api …/actions/workflows` to a fixed answer and every other
    /// call through a check-snapshot sequence that repeats its last entry.
    struct WorkflowProbeRunner {
        checks: RefCell<Vec<String>>,
        workflows: Result<String, ()>,
        probes: RefCell<usize>,
    }

    impl WorkflowProbeRunner {
        fn new(checks: &[&str], workflows: Result<&str, ()>) -> Self {
            Self {
                checks: RefCell::new(checks.iter().map(|c| c.to_string()).collect()),
                workflows: workflows.map(str::to_string),
                probes: RefCell::new(0),
            }
        }
    }

    impl BackendRunner for WorkflowProbeRunner {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            if call.argv.first().map(|a| a.as_os_str()) == Some("api".as_ref()) {
                *self.probes.borrow_mut() += 1;
                assert_eq!(
                    call.argv[1],
                    std::ffi::OsString::from("repos/acme/widgets/actions/workflows?per_page=1")
                );
                return match &self.workflows {
                    Ok(body) => Ok(BackendSuccess {
                        stdout: body.clone(),
                        stderr: String::new(),
                    }),
                    Err(()) => Err(ForgeError::validation("test", "stub", "api refused", None)),
                };
            }
            let mut checks = self.checks.borrow_mut();
            let next = if checks.len() > 1 {
                checks.remove(0)
            } else {
                checks[0].clone()
            };
            Ok(BackendSuccess {
                stdout: next,
                stderr: String::new(),
            })
        }
    }

    fn repo_ctx() -> ProviderContext {
        ProviderContext {
            repo: Some("acme/widgets".into()),
            ..make_ctx(Provider::GitHub)
        }
    }

    fn default_budget_args() -> PrWaitChecksArgs {
        PrWaitChecksArgs {
            timeout: Duration::from_secs(30 * 60),
            interval: Duration::from_secs(20),
            ..make_args("1")
        }
    }

    fn poll(runner: &WorkflowProbeRunner, args: &PrWaitChecksArgs) -> WaitOutcome {
        let snapshot_args = PrChecksArgs {
            id: args.id.clone(),
            required_only: args.required_only,
        };
        poll_until_terminal_or_timeout(
            runner,
            &StepClock::new(),
            &make_global(),
            &repo_ctx(),
            args,
            &snapshot_args,
        )
        .expect("poll must not error")
    }

    /// The #1801 case: no workflows and nothing reported. The wait stops once
    /// the grace period has passed instead of burning the 30-minute budget.
    #[test]
    fn a_repository_without_workflows_fails_after_the_grace_period() {
        let runner = WorkflowProbeRunner::new(&["[]"], Ok(r#"{"total_count":0,"workflows":[]}"#));
        match poll(&runner, &default_budget_args()) {
            WaitOutcome::NotRegistered(snapshot, NotRegisteredCause::NoWorkflows) => {
                let waited = Duration::from_millis(snapshot.duration_ms.expect("duration"));
                assert!(waited >= NO_CI_GRACE_PERIOD, "waited {waited:?}");
                assert!(waited < NO_CI_GRACE_PERIOD * 2, "waited {waited:?}");
            }
            _ => panic!("a repository with no workflows must fail fast as not-registered"),
        }
        assert_eq!(*runner.probes.borrow(), 1);
    }

    /// A repository that has workflows keeps today's wait: nothing is decided
    /// from the probe, and it is asked only once.
    #[test]
    fn a_repository_with_workflows_waits_out_the_budget() {
        let runner = WorkflowProbeRunner::new(&["[]"], Ok(r#"{"total_count":2,"workflows":[]}"#));
        match poll(&runner, &default_budget_args()) {
            WaitOutcome::NotRegistered(snapshot, NotRegisteredCause::BudgetExpired) => {
                assert_eq!(snapshot.duration_ms, Some(30 * 60 * 1000));
            }
            _ => panic!("a repository with workflows must wait for the whole budget"),
        }
        assert_eq!(*runner.probes.borrow(), 1);
    }

    /// Failing to ask is not evidence of no CI; the wait continues.
    #[test]
    fn a_failed_workflow_probe_keeps_waiting() {
        let runner = WorkflowProbeRunner::new(&["[]"], Err(()));
        assert!(matches!(
            poll(&runner, &default_budget_args()),
            WaitOutcome::NotRegistered(_, NotRegisteredCause::BudgetExpired)
        ));
    }

    /// Slow registration past the grace period still reaches success when the
    /// repository has workflows.
    #[test]
    fn checks_registering_after_the_grace_period_still_succeed() {
        let mut sequence = vec!["[]"; 2 * 5];
        sequence.extend([ONE_REQUIRED_PASS, ONE_REQUIRED_PASS]);
        let runner = WorkflowProbeRunner::new(&sequence, Ok(r#"{"total_count":1,"workflows":[]}"#));
        match poll(&runner, &default_budget_args()) {
            WaitOutcome::Success(snapshot) => {
                assert_eq!(snapshot.required_count, 1);
                assert!(
                    Duration::from_millis(snapshot.duration_ms.expect("duration"))
                        > NO_CI_GRACE_PERIOD
                );
            }
            _ => panic!("late registration must still reach success"),
        }
    }

    /// A declared opt-out never needs the probe.
    #[test]
    fn allowed_no_checks_never_probes_workflows() {
        let runner = WorkflowProbeRunner::new(&["[]"], Ok(r#"{"total_count":0}"#));
        let args = PrWaitChecksArgs {
            allow_no_checks: true,
            ..default_budget_args()
        };
        assert!(matches!(poll(&runner, &args), WaitOutcome::Success(_)));
        assert_eq!(*runner.probes.borrow(), 0);
    }

    #[test]
    fn workflow_probe_needs_an_owner_name_slug() {
        assert!(build_github_workflows_call(&make_ctx(Provider::GitHub)).is_none());
        let nested = ProviderContext {
            repo: Some("group/sub/repo".into()),
            ..make_ctx(Provider::GitHub)
        };
        assert!(build_github_workflows_call(&nested).is_none());
        assert!(build_github_workflows_call(&repo_ctx()).is_some());
    }

    #[test]
    fn step_clock_advances_on_sleep() {
        let clock = StepClock::new();
        let before = clock.now();
        clock.sleep(Duration::from_secs(2));
        let after = clock.now();
        assert_eq!(ms_between(before, after), 2000);
    }

    #[test]
    fn failure_state_marks_terminal_and_emit_failure_returns_runtime_code() {
        let snapshot = PrChecksPayload {
            provider: "github",
            state: "failure",
            required_count: 1,
            success_count: 0,
            failed: vec![pr_checks::FailedCheck {
                name: "test".into(),
                url: None,
                conclusion: Some("failure".into()),
            }],
            pending: Vec::new(),
            checks: Vec::new(),
            duration_ms: Some(123),
            warnings: Vec::new(),
        };
        assert!(is_terminal(&snapshot, CheckPresence::Required));
        let code = emit_failure(
            snapshot,
            "checks_failed",
            "required checks did not reach success",
            nils_common::cli_contract::exit::RUNTIME,
            OutputFormat::Json,
        );
        assert_eq!(code, nils_common::cli_contract::exit::RUNTIME);
    }

    #[test]
    fn timed_out_placeholder_has_pending_state() {
        let p = timed_out_placeholder(&make_ctx(Provider::GitHub));
        assert_eq!(p.state, "pending");
    }
}

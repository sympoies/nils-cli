//! Operator cleanup of orphaned Main Agent runs.
//!
//! A run whose controller session was deleted can never be closed through
//! `close` or `closeout`, because both authenticate as that controller. These
//! commands let the local operator, trusted at the same level as the state
//! directory itself, list such runs and terminalize their records. They only
//! ever change registry records: sessions, worktrees, packets, and receipts are
//! left in place.
//!
//! A run is orphaned when its controller session is definitely deleted and holds
//! no active claim, and every assignment passes the same live-owner rule the
//! pre-launch overlap check uses (`assignment_worker_may_be_live`), carries no
//! in-flight operation fence, and has no pending progress receipt.

use super::*;

use agent_session::internal::coordination::claims::ActiveClaimContext;

const ORPHANED_RUNS_SCHEMA: &str = "main-agent.orphaned-runs.v1";
const CLOSE_ORPHANED_RUNS_SCHEMA: &str = "main-agent.close-orphaned-runs.v1";
/// Receipt principal for operator-authorized closes. It is not a session id:
/// the operator is authorized by state-directory ownership, not a capability.
const OPERATOR_PRINCIPAL: &str = "local-operator";
const OPERATOR_INCARNATION: &str = "state-dir";
const CLOSE_OPERATION: &str = "close-orphaned-runs";
const RUN_REASON: &str = "orphaned";
const ASSIGNMENT_REASON: &str = "orphaned-run";
const TERMINAL_ASSIGNMENT_STATES: [&str; 2] = ["released", "cancelled"];

const CLOSE_ORPHANED_AFTER_HELP: &str = "SAFETY:\n  Without --apply this only prints the plan. --apply commits exactly the plan\n  whose plan_digest the dry run printed, under the registry lock, and refuses\n  when any run, assignment, or revision changed since. Only registry records\n  change: runs become closed (reason orphaned); non-terminal assignments become\n  cancelled, or released when already accepted (reason orphaned-run). Sessions,\n  worktrees, packets, and history are never deleted.\n\nREFUSED RUNS:\n  orphaned-run-controller-claim-active  controller still holds an active claim\n  orphaned-run-worker-live              a worker session exists, holds a claim,\n                                        or a launch has no bound worker yet\n  orphaned-run-operation-pending        an operation fence or progress receipt\n                                        is still in flight\n  orphaned-run-too-recent               last activity is newer than --older-than\n\nEXAMPLES:\n  main-agent runs close-orphaned --older-than 7d --format json\n  main-agent runs close-orphaned --older-than 7d --apply --if-plan-digest sha256:... --idempotency-key close-orphaned-001 --format json";

#[derive(Clone, Debug, Args)]
pub(super) struct RunsArgs {
    #[command(subcommand)]
    command: RunsCommand,
}

#[derive(Clone, Debug, Subcommand)]
enum RunsCommand {
    /// List active runs whose controller session is deleted and whose workers,
    /// claims, and operations are all gone. Read-only.
    Orphaned(OrphanedArgs),
    /// Close orphaned runs older than a bound. Prints the plan unless --apply.
    #[command(after_help = CLOSE_ORPHANED_AFTER_HELP)]
    CloseOrphaned(CloseOrphanedArgs),
}

#[derive(Clone, Debug, Args)]
struct OrphanedArgs {
    /// Report runs whose last run or assignment activity is newer than this
    /// (for example 7d or 12h) as too recent. The default 0 applies no bound.
    #[arg(long, value_name = "DURATION", default_value = "0")]
    older_than: String,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

#[derive(Clone, Debug, Args)]
struct CloseOrphanedArgs {
    /// Close only runs whose last run or assignment activity is at least this
    /// old, for example 7d or 12h.
    #[arg(long, value_name = "DURATION")]
    older_than: String,
    /// Print the plan without changing anything. This is the default.
    #[arg(long, conflicts_with = "apply")]
    dry_run: bool,
    /// Commit the plan named by --if-plan-digest.
    #[arg(long, requires_all = ["if_plan_digest", "idempotency_key"])]
    apply: bool,
    /// The plan_digest printed by the dry run being applied.
    #[arg(long, value_name = "DIGEST")]
    if_plan_digest: Option<String>,
    #[arg(long, help = IDEMPOTENCY_KEY_HELP)]
    idempotency_key: Option<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    format: OutputFormat,
}

pub(super) fn command_name(args: &RunsArgs) -> &'static str {
    match args.command {
        RunsCommand::Orphaned(_) => "runs-orphaned",
        RunsCommand::CloseOrphaned(_) => "runs-close-orphaned",
    }
}

pub(super) fn output_format(args: &RunsArgs) -> OutputFormat {
    match &args.command {
        RunsCommand::Orphaned(args) => args.format,
        RunsCommand::CloseOrphaned(args) => args.format,
    }
}

pub(super) fn run_runs(context: &CliContext, args: &RunsArgs) -> Result<Value, CliError> {
    match &args.command {
        RunsCommand::Orphaned(args) => list_orphaned(context, args),
        RunsCommand::CloseOrphaned(args) => close_orphaned(context, args),
    }
}

fn list_orphaned(context: &CliContext, args: &OrphanedArgs) -> Result<Value, CliError> {
    let older_than = parse_duration_seconds(&args.older_than)?;
    // Observe claims before reading the orchestration registry to keep the
    // coordination -> orchestration order the mutating path uses.
    let active_claims =
        agent_session::internal::coordination::claims::active_claim_contexts(context)?;
    let registry = orchestration::load_registry_readonly(context)?;
    let evaluation = evaluate(
        &registry,
        &StateDirLiveness {
            context,
            active_claims: &active_claims,
        },
        jiff::Timestamp::now().as_second(),
        older_than,
    );
    Ok(json!({
        "schema_version": ORPHANED_RUNS_SCHEMA,
        "older_than_seconds": older_than,
        "active_runs": evaluation.active_runs,
        "controller_live_runs": evaluation.controller_live_runs,
        "plan_digest": plan_digest(&evaluation.closable),
        "orphaned": evaluation.closable,
        "refused": evaluation.refused,
    }))
}

fn close_orphaned(context: &CliContext, args: &CloseOrphanedArgs) -> Result<Value, CliError> {
    let older_than = parse_duration_seconds(&args.older_than)?;
    if let Some(key) = args.idempotency_key.as_deref() {
        validate_idempotency_key(key)?;
    }
    if let Some(digest) = args.if_plan_digest.as_deref() {
        validate_plan_digest(digest)?;
    }
    let active_claims =
        agent_session::internal::coordination::claims::active_claim_contexts(context)?;
    let liveness = StateDirLiveness {
        context,
        active_claims: &active_claims,
    };
    let now = jiff::Timestamp::now().as_second();
    if !args.apply {
        let registry = orchestration::load_registry_readonly(context)?;
        let evaluation = evaluate(&registry, &liveness, now, older_than);
        return Ok(close_result(false, older_than, evaluation));
    }
    let (Some(expected_digest), Some(idempotency_key)) = (
        args.if_plan_digest.as_deref(),
        args.idempotency_key.as_deref(),
    ) else {
        return Err(CliError::usage(
            "orphaned-run-plan-required",
            "--apply requires --if-plan-digest and --idempotency-key",
            None,
        ));
    };
    let request_digest = agent_session::internal::coordination::request_digest(
        CLOSE_OPERATION,
        &json!({ "older_than_seconds": older_than, "plan_digest": expected_digest }),
    );
    let mut locked = orchestration::lock_registry(context)?;
    if let Some(outcome) = operator_replay(&locked.registry, idempotency_key, &request_digest)? {
        return Ok(outcome);
    }
    let evaluation = evaluate(&locked.registry, &liveness, now, older_than);
    let current_digest = plan_digest(&evaluation.closable);
    if current_digest != expected_digest {
        return Err(CliError::data(
            "orphaned-run-plan-conflict",
            "the orphaned-run plan changed since it was reviewed; rerun the dry run",
            Some(json!({ "current_plan_digest": current_digest })),
        ));
    }
    let updated_at = timestamp();
    for planned in &evaluation.closable {
        terminalize(&mut locked.registry, planned, &updated_at);
    }
    let outcome = close_result(true, older_than, evaluation);
    store_receipt_for_principal(
        &mut locked.registry,
        OPERATOR_PRINCIPAL,
        OPERATOR_INCARNATION,
        idempotency_key,
        CLOSE_OPERATION,
        &request_digest,
        outcome.clone(),
    )?;
    locked.save()?;
    Ok(outcome)
}

fn close_result(applied: bool, older_than: u64, evaluation: Evaluation) -> Value {
    json!({
        "schema_version": CLOSE_ORPHANED_RUNS_SCHEMA,
        "mode": if applied { "apply" } else { "dry-run" },
        "applied": applied,
        "older_than_seconds": older_than,
        "active_runs": evaluation.active_runs,
        "controller_live_runs": evaluation.controller_live_runs,
        "plan_digest": plan_digest(&evaluation.closable),
        "runs": evaluation.closable,
        "refused": evaluation.refused,
    })
}

fn validate_plan_digest(value: &str) -> Result<(), CliError> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if valid {
        Ok(())
    } else {
        Err(CliError::usage(
            "invalid-plan-digest",
            "--if-plan-digest must be the sha256:<64 lowercase hex> plan_digest a dry run printed",
            None,
        ))
    }
}

fn operator_replay(
    registry: &orchestration::Registry,
    idempotency_key: &str,
    request_digest: &str,
) -> Result<Option<Value>, CliError> {
    let key = receipt_key(OPERATOR_PRINCIPAL, OPERATOR_INCARNATION, idempotency_key);
    let Some(receipt) = registry.receipts.get(&key) else {
        return Ok(None);
    };
    if receipt.operation != CLOSE_OPERATION || receipt.request_digest != request_digest {
        return Err(CliError::data(
            "idempotency-conflict",
            "idempotency key was already used for a different request",
            None,
        ));
    }
    Ok(Some(receipt.outcome.clone()))
}

/// One run the plan closes. Serialized as-is into the command output.
#[derive(Debug, Serialize)]
struct PlannedRun {
    run_id: String,
    objective_summary: String,
    controller_session_id: String,
    created_at: String,
    last_activity_at: String,
    from_state: String,
    to_state: &'static str,
    from_revision: u64,
    to_revision: u64,
    reason: &'static str,
    assignments: Vec<PlannedAssignment>,
}

/// One non-terminal assignment of a planned run.
#[derive(Debug, Serialize)]
struct PlannedAssignment {
    assignment_id: String,
    from_state: String,
    to_state: &'static str,
    from_revision: u64,
    to_revision: u64,
    reason: &'static str,
}

/// The digest binds the apply to exactly the reviewed records: every closable
/// run and non-terminal assignment with the revision and state it had. Arrays
/// keep the encoding independent of JSON object key ordering.
fn plan_digest(closable: &[PlannedRun]) -> String {
    let identity = closable
        .iter()
        .map(|run| {
            json!([
                run.run_id,
                run.from_revision,
                run.assignments
                    .iter()
                    .map(|assignment| {
                        json!([
                            assignment.assignment_id,
                            assignment.from_revision,
                            assignment.from_state
                        ])
                    })
                    .collect::<Vec<_>>()
            ])
        })
        .collect::<Vec<_>>();
    format!(
        "sha256:{}",
        agent_session::internal::coordination::request_digest("orphaned-run-plan", &identity)
    )
}

/// Apply one planned run. The plan was evaluated from this same registry under
/// the held lock, so every record it names exists with its planned revision.
fn terminalize(registry: &mut orchestration::Registry, planned: &PlannedRun, updated_at: &str) {
    for change in &planned.assignments {
        if let Some(assignment) = registry.assignments.get_mut(&change.assignment_id) {
            assignment.state = change.to_state.to_string();
            assignment.revision = change.to_revision;
            assignment.updated_at = updated_at.to_string();
            if change.to_state == "cancelled" {
                assignment.blocker_summary = Some(format!(
                    "Terminalized by the local operator after its run was orphaned: {ASSIGNMENT_REASON}"
                ));
            }
        }
    }
    if let Some(run) = registry.runs.get_mut(&planned.run_id) {
        run.state = planned.to_state.to_string();
        run.revision = planned.to_revision;
        run.updated_at = updated_at.to_string();
    }
}

struct Evaluation {
    active_runs: usize,
    controller_live_runs: usize,
    closable: Vec<PlannedRun>,
    refused: Vec<Value>,
}

/// The host facts the orphan decision reads: session directories, active
/// claims observed before the registry lock, and assignment operation fences.
struct StateDirLiveness<'a> {
    context: &'a CliContext,
    active_claims: &'a [ActiveClaimContext],
}

impl StateDirLiveness<'_> {
    fn session_may_exist(&self, session_id: &str) -> bool {
        worker_session_may_exist(self.context, session_id)
    }

    /// Any incarnation counts: a claim the session still holds under an older
    /// or newer launch is still ownership this cleanup must not override.
    fn session_holds_active_claim(&self, session_id: &str) -> bool {
        self.active_claims
            .iter()
            .any(|claim| claim.session_id == session_id)
    }

    /// The code of an in-flight operation fence on the assignment, if any.
    fn assignment_operation(&self, assignment: &AssignmentRecord) -> Option<String> {
        if submit_recovery_in_flight(assignment) {
            return Some("submit-recovery-in-flight".to_string());
        }
        if assignment.worker_quarantine.is_some() {
            return Some("worker-quarantined".to_string());
        }
        // An unreadable fence is as uncertain as a present one.
        ensure_assignment_mutation_admitted(
            self.context,
            assignment,
            AssignmentMutationOwner::Ordinary,
        )
        .err()
        .map(|error| error.code().to_string())
    }
}

/// A controller- or worker-owned receipt that records an operation still in
/// progress. Its owner is gone, but its outcome was never resolved.
fn receipt_outcome_is_pending(outcome: &Value) -> bool {
    outcome["state"] == "in_progress"
        || worker_start_is_pending(outcome)
        || worker_start_readiness_is_pending(outcome)
        || worker_delete_is_pending(outcome)
        || worker_start_batch_lane_is_claim(outcome)
        || outcome["schema_version"] == "main-agent.quick-pending.v1"
        || (outcome["ok"] == false && outcome["resumable"] == true)
}

fn blocker(code: &str, details: Value) -> Value {
    let mut blocker = json!({ "code": code });
    if let (Some(target), Value::Object(details)) = (blocker.as_object_mut(), details) {
        target.extend(details);
    }
    blocker
}

fn activity_epoch(value: &str) -> Option<i64> {
    value
        .parse::<jiff::Timestamp>()
        .ok()
        .map(|timestamp| timestamp.as_second())
}

fn evaluate(
    registry: &orchestration::Registry,
    liveness: &StateDirLiveness<'_>,
    now: i64,
    older_than: u64,
) -> Evaluation {
    let mut pending_by_principal: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for receipt in registry.receipts.values() {
        if receipt_outcome_is_pending(&receipt.outcome) {
            pending_by_principal
                .entry(receipt.principal_session_id.as_str())
                .or_default()
                .push(receipt.operation.as_str());
        }
    }
    let mut evaluation = Evaluation {
        active_runs: 0,
        controller_live_runs: 0,
        closable: Vec::new(),
        refused: Vec::new(),
    };
    for run in registry.runs.values().filter(|run| run_is_live(run)) {
        evaluation.active_runs += 1;
        let controller = run.controller.session_id.as_str();
        if liveness.session_may_exist(controller) {
            evaluation.controller_live_runs += 1;
            continue;
        }
        let assignments = registry
            .assignments
            .values()
            .filter(|assignment| assignment.run_id == run.run_id)
            .collect::<Vec<_>>();
        let mut blockers = Vec::new();
        if liveness.session_holds_active_claim(controller) {
            blockers.push(blocker(
                "orphaned-run-controller-claim-active",
                json!({ "session_id": controller }),
            ));
        }
        let mut principals = vec![controller];
        for assignment in &assignments {
            let terminal = TERMINAL_ASSIGNMENT_STATES.contains(&assignment.state.as_str());
            let worker = assignment.worker.as_ref();
            let worker_claim = worker
                .is_some_and(|worker| liveness.session_holds_active_claim(&worker.session_id));
            let live = (!terminal || worker.is_some())
                && assignment_worker_may_be_live(worker, worker_claim, |worker| {
                    liveness.session_may_exist(&worker.session_id)
                });
            if live {
                let reason = match worker {
                    None => "worker-launch-pending",
                    Some(_) if worker_claim => "worker-claim-active",
                    Some(_) => "worker-session-present",
                };
                blockers.push(blocker(
                    "orphaned-run-worker-live",
                    json!({ "assignment_id": assignment.assignment_id, "reason": reason }),
                ));
            }
            if let Some(operation) = liveness.assignment_operation(assignment) {
                blockers.push(blocker(
                    "orphaned-run-operation-pending",
                    json!({ "assignment_id": assignment.assignment_id, "operation": operation }),
                ));
            }
            if let Some(worker) = worker {
                principals.push(worker.session_id.as_str());
            }
        }
        principals.sort_unstable();
        principals.dedup();
        for principal in principals {
            for operation in pending_by_principal.get(principal).into_iter().flatten() {
                blockers.push(blocker(
                    "orphaned-run-operation-pending",
                    json!({ "session_id": principal, "operation": operation }),
                ));
            }
        }
        let activity = std::iter::once(run.updated_at.as_str()).chain(
            assignments
                .iter()
                .map(|assignment| assignment.updated_at.as_str()),
        );
        let mut last_activity = run.updated_at.as_str();
        let mut last_epoch = Some(i64::MIN);
        for value in activity {
            let epoch = activity_epoch(value);
            // An unparseable time never ages in, whatever the bound, so it
            // wins and stays unparsed.
            if last_epoch.is_some() && epoch.is_none_or(|epoch| Some(epoch) > last_epoch) {
                last_activity = value;
                last_epoch = epoch;
            }
        }
        match last_epoch {
            None => blockers.push(blocker(
                "orphaned-run-too-recent",
                json!({
                    "last_activity_at": last_activity,
                    "reason": "activity-time-unparseable"
                }),
            )),
            Some(epoch) if u64::try_from(now.saturating_sub(epoch)).unwrap_or(0) < older_than => {
                blockers.push(blocker(
                    "orphaned-run-too-recent",
                    json!({ "last_activity_at": last_activity }),
                ));
            }
            Some(_) => {}
        }
        if let Some(first) = blockers.first() {
            evaluation.refused.push(json!({
                "run_id": run.run_id,
                "revision": run.revision,
                "controller_session_id": controller,
                "last_activity_at": last_activity,
                "code": first["code"],
                "blockers": blockers,
            }));
            continue;
        }
        let changes = assignments
            .iter()
            .filter(|assignment| !TERMINAL_ASSIGNMENT_STATES.contains(&assignment.state.as_str()))
            .map(|assignment| PlannedAssignment {
                assignment_id: assignment.assignment_id.clone(),
                from_state: assignment.state.clone(),
                to_state: if assignment.state == "accepted" {
                    "released"
                } else {
                    "cancelled"
                },
                from_revision: assignment.revision,
                to_revision: assignment.revision.saturating_add(1),
                reason: ASSIGNMENT_REASON,
            })
            .collect();
        evaluation.closable.push(PlannedRun {
            run_id: run.run_id.clone(),
            objective_summary: run.objective_summary.clone(),
            controller_session_id: controller.to_string(),
            created_at: run.created_at.clone(),
            last_activity_at: last_activity.to_string(),
            from_state: run.state.clone(),
            to_state: "closed",
            from_revision: run.revision,
            to_revision: run.revision.saturating_add(1),
            reason: RUN_REASON,
            assignments: changes,
        });
    }
    evaluation
}

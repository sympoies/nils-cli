//! Main Agent group cleanup and group archive engine.
//!
//! The serve daemon exposes group cleanup and archive for a Main Agent run, so
//! this engine lives in the agent-session library rather than behind the
//! `main-agent` facade. It owns plan construction, the durable execution lock,
//! replay receipts, and the staged assignment transitions.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::orchestration::{
    self, GroupCleanupProgressReceipt, IdempotencyReceipt, RunRecord, SessionRef,
};
use crate::{CliContext, CliError, SessionRegistryFence, load_session_record, session_dir};

use crate::orchestration_support::*;

pub(crate) const GROUP_CLEANUP_SCHEMA: &str = "agent-session.main-agent-group-cleanup.v1";
pub(crate) const GROUP_CLEANUP_REQUEST_SCHEMA: &str =
    "agent-session.main-agent-group-cleanup-request.v1";
pub(crate) const GROUP_CLEANUP_RESULT_SCHEMA: &str =
    "agent-session.main-agent-group-cleanup-result.v1";
pub(crate) const GROUP_ARCHIVE_REQUEST_SCHEMA: &str =
    "agent-session.main-agent-group-archive-request.v1";
pub(crate) const GROUP_ARCHIVE_RESULT_SCHEMA: &str =
    "agent-session.main-agent-group-archive-result.v1";
pub(crate) const GROUP_CLEANUP_MAX_ASSIGNMENTS: usize = 64;
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum GroupCleanupMode {
    Safe,
    Force,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupCleanupRequest {
    pub schema_version: String,
    pub expected_main_incarnation: String,
    pub expected_run_revision: u64,
    pub expected_plan_digest: String,
    pub mode: GroupCleanupMode,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GroupArchiveRequest {
    pub schema_version: String,
    pub expected_main_incarnation: String,
    pub expected_run_revision: u64,
    pub expected_plan_digest: String,
    pub mode: GroupCleanupMode,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GroupCleanupWorkerPlan {
    pub(crate) assignment_id: String,
    pub(crate) state: String,
    pub(crate) worker: Option<SessionRef>,
    pub(crate) force_required: bool,
    pub(crate) primary_managed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GroupCleanupPlan {
    pub(crate) schema_version: String,
    pub(crate) main: SessionRef,
    pub(crate) run_id: String,
    pub(crate) run_revision: u64,
    pub(crate) requires_force: bool,
    pub(crate) workers: Vec<GroupCleanupWorkerPlan>,
    pub(crate) plan_digest: String,
}

pub(crate) struct GroupCleanupExecution {
    pub value: Value,
    pub deleted_registry_fences: Vec<SessionRegistryFence>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct GroupCleanupResumeState {
    schema_version: String,
    plan: GroupCleanupPlan,
    #[serde(default)]
    authority_sealed: bool,
    worker_results: Vec<Value>,
    deleted_registry_fences: Vec<SessionRegistryFence>,
    #[serde(default)]
    pending_registry_fences: Vec<SessionRegistryFence>,
    run_closed: bool,
}

pub(crate) struct GroupCleanupReplay {
    value: Value,
    resume: Option<GroupCleanupResumeState>,
}

pub(crate) struct GroupCleanupProgressIdentity<'a> {
    requested_session_id: &'a str,
    principal_session_id: &'a str,
    incarnation: &'a str,
    operation: &'a str,
}

pub(crate) fn preview_group_cleanup(
    context: &CliContext,
    main_session_id: &str,
) -> Result<Value, CliError> {
    crate::validate_id(main_session_id)?;
    let record = load_session_record(context, main_session_id)?;
    let incarnation = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CliError::data(
                "main-session-incarnation-unavailable",
                "Main Agent session incarnation is unavailable",
                None,
            )
        })?;
    let main = session_ref(context, &record, incarnation);
    let registry = orchestration::load_registry_readonly(context)?;
    let run = registry
        .runs
        .values()
        .find(|run| run.state == "active" && run.controller == main)
        .ok_or_else(|| {
            not_found(
                "main-agent-run-not-found",
                "session is not the current controller of an active Main Agent run",
            )
        })?;
    serde_json::to_value(build_group_cleanup_plan(&registry, run, &main)?)
        .map_err(|_| invalid_input("group cleanup preview could not be serialized"))
}

pub(crate) fn execute_group_cleanup(
    context: &CliContext,
    main_session_id: &str,
    request: GroupCleanupRequest,
    tmux_bin: PathBuf,
) -> Result<GroupCleanupExecution, CliError> {
    execute_group_cleanup_operation(context, main_session_id, request, tmux_bin, false)
}

pub(crate) fn execute_group_archive(
    context: &CliContext,
    main_session_id: &str,
    request: GroupArchiveRequest,
    tmux_bin: PathBuf,
) -> Result<GroupCleanupExecution, CliError> {
    crate::validate_id(main_session_id)?;
    if request.schema_version != GROUP_ARCHIVE_REQUEST_SCHEMA {
        return Err(invalid_input("group archive request schema is unsupported"));
    }
    orchestration::validate_slug(
        "main session incarnation",
        &request.expected_main_incarnation,
        128,
    )?;
    orchestration::validate_digest(&request.expected_plan_digest)?;
    validate_idempotency_key(&request.idempotency_key)?;
    let cleanup_request = GroupCleanupRequest {
        schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
        expected_main_incarnation: request.expected_main_incarnation,
        expected_run_revision: request.expected_run_revision,
        expected_plan_digest: request.expected_plan_digest,
        mode: request.mode,
        idempotency_key: request.idempotency_key,
    };
    execute_group_cleanup_operation(context, main_session_id, cleanup_request, tmux_bin, true)
}

fn execute_group_cleanup_operation(
    context: &CliContext,
    main_session_id: &str,
    request: GroupCleanupRequest,
    tmux_bin: PathBuf,
    archive: bool,
) -> Result<GroupCleanupExecution, CliError> {
    let operation = if archive {
        "group-archive"
    } else {
        "group-cleanup"
    };
    let requested_main_session_id = main_session_id.to_string();
    validate_group_cleanup_request(&requested_main_session_id, &request)?;
    let request_digest = group_cleanup_operation_request_digest(&request, operation);
    let resolved_record = load_session_record(context, main_session_id);
    let canonical_main_session_id = match resolved_record.as_ref() {
        Ok(record) => record.id.clone(),
        Err(error) if error.code() == "session-not-found" => {
            let progress_principal = orchestration::recover_group_cleanup_progress_principal(
                context,
                &requested_main_session_id,
                &request.expected_main_incarnation,
                &request.idempotency_key,
                &request_digest,
            )?;
            if let Some(principal) = progress_principal {
                principal
            } else {
                let registry = orchestration::load_registry_readonly(context)?;
                recover_completed_group_cleanup_principal(
                    &registry,
                    &requested_main_session_id,
                    &request.expected_main_incarnation,
                    &request.idempotency_key,
                    &request_digest,
                    operation,
                )?
                .unwrap_or_else(|| requested_main_session_id.clone())
            }
        }
        Err(_) => requested_main_session_id.clone(),
    };
    let main_session_id = canonical_main_session_id.as_str();
    let legacy_alias = (requested_main_session_id != canonical_main_session_id)
        .then_some(requested_main_session_id.as_str());
    let execution_owner_digest = group_cleanup_execution_owner_digest(main_session_id);
    let _execution_lock = lock_group_cleanup_execution(context, &execution_owner_digest)?;

    let replay = {
        let locked = orchestration::lock_registry(context)?;
        group_cleanup_replay_with_legacy_alias(
            context,
            &locked.registry,
            main_session_id,
            legacy_alias,
            &request.expected_main_incarnation,
            &request.idempotency_key,
            &request_digest,
            operation,
        )?
    };
    let resume_state = if let Some(replay) = replay {
        if replay.value["completed"] == true {
            let deleted_registry_fences = replay
                .resume
                .map(|resume| resume.deleted_registry_fences)
                .unwrap_or_default();
            orchestration::remove_group_cleanup_progress(
                context,
                &group_cleanup_progress_key(
                    main_session_id,
                    &request.expected_main_incarnation,
                    &request.idempotency_key,
                ),
            )?;
            remove_legacy_group_cleanup_progress(
                context,
                legacy_alias,
                &request.expected_main_incarnation,
                &request.idempotency_key,
            )?;
            return Ok(GroupCleanupExecution {
                value: replay.value,
                deleted_registry_fences,
            });
        }
        Some(replay.resume.ok_or_else(|| {
            CliError::data(
                "group-cleanup-progress-invalid",
                "retryable group cleanup receipt has no resumable progress",
                None,
            )
        })?)
    } else {
        None
    };
    let closed_run_revision = request
        .expected_run_revision
        .checked_add(1)
        .ok_or_else(|| {
            CliError::data(
                "orchestration-revision-capacity",
                "Main Agent run revision reached its maximum value",
                Some(json!({ "run_revision": request.expected_run_revision })),
            )
        })?;

    let record = match resolved_record {
        Ok(record) => Some(record),
        Err(error)
            if error.code() == "session-not-found"
                && resume_state.as_ref().is_some_and(|resume| {
                    resume
                        .pending_registry_fences
                        .iter()
                        .any(|fence| fence.session_id == main_session_id)
                }) =>
        {
            None
        }
        Err(error) => return Err(error),
    };
    let incarnation = record
        .as_ref()
        .and_then(|record| record.runtime.as_ref())
        .map(|runtime| runtime.launch_id.as_str())
        .filter(|value| !value.is_empty())
        .unwrap_or(&request.expected_main_incarnation)
        .to_string();
    if incarnation != request.expected_main_incarnation {
        return Err(CliError::data(
            "main-session-incarnation-conflict",
            "Main Agent session incarnation changed after preview",
            Some(json!({
                "expected_main_incarnation": request.expected_main_incarnation,
                "actual_main_incarnation": incarnation,
            })),
        ));
    }
    let progress_identity = GroupCleanupProgressIdentity {
        requested_session_id: &requested_main_session_id,
        principal_session_id: main_session_id,
        incarnation: &incarnation,
        operation,
    };
    let main = record.as_ref().map_or_else(
        || {
            resume_state
                .as_ref()
                .expect("missing Main Agent record requires resumable progress")
                .plan
                .main
                .clone()
        },
        |record| session_ref(context, record, &incarnation),
    );

    let plan = if let Some(resume) = resume_state.as_ref() {
        if resume.schema_version != "agent-session.main-agent-group-cleanup-progress.v1"
            || resume.plan.main != main
            || resume.plan.run_id.is_empty()
            || resume.plan.run_revision != request.expected_run_revision
            || resume.plan.plan_digest != request.expected_plan_digest
        {
            return Err(CliError::data(
                "group-cleanup-progress-invalid",
                "durable group cleanup progress does not match the immutable request",
                None,
            ));
        }
        resume.plan.clone()
    } else {
        let locked = orchestration::lock_registry(context)?;
        let run = locked
            .registry
            .runs
            .values()
            .find(|run| run.state == "active" && run.controller == main)
            .cloned()
            .ok_or_else(|| {
                not_found(
                    "main-agent-run-not-found",
                    "session is not the current controller of an active Main Agent run",
                )
            })?;
        ensure_revision(request.expected_run_revision, run.revision, "run")?;
        let plan = build_group_cleanup_plan(&locked.registry, &run, &main)?;
        if plan.plan_digest != request.expected_plan_digest {
            return Err(CliError::data(
                "group-cleanup-plan-conflict",
                "Main Agent group cleanup plan changed after preview",
                Some(json!({
                    "expected_plan_digest": request.expected_plan_digest,
                    "current_plan_digest": plan.plan_digest,
                    "current_run_revision": run.revision,
                })),
            ));
        }
        plan
    };
    let mut worker_refs = plan
        .workers
        .iter()
        .filter_map(|worker| worker.worker.as_ref())
        .cloned()
        .collect::<Vec<_>>();
    worker_refs.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    let mut authority_refs = worker_refs.clone();
    authority_refs.push(main.clone());
    authority_refs.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    authority_refs.dedup_by(|left, right| left.session_id == right.session_id);
    let mut locked_session_authorities = Vec::with_capacity(authority_refs.len());
    for session in &authority_refs {
        let locked = crate::lock_exact_session_authority(context, &session.session_id)?;
        if let Some(locked) = locked.as_ref() {
            let session_incarnation = locked
                .record
                .runtime
                .as_ref()
                .map(|runtime| runtime.launch_id.as_str())
                .unwrap_or_default();
            if !orchestration::session_ref_matches(session, &locked.record, session_incarnation) {
                return Err(CliError::data(
                    "session-incarnation-conflict",
                    "session identity changed before group cleanup authority was fenced",
                    Some(json!({ "session_id": session.session_id })),
                ));
            }
        }
        locked_session_authorities.push((session.clone(), locked));
    }
    let plan = if resume_state
        .as_ref()
        .is_none_or(|resume| !resume.authority_sealed)
    {
        let worker_sessions = worker_refs
            .iter()
            .map(|worker| {
                (
                    worker.session_id.clone(),
                    worker.session_incarnation.clone(),
                )
            })
            .collect::<Vec<_>>();
        let cleanup_quiescence = crate::coordination::lock_group_cleanup_quiescence(
            context,
            &worker_sessions,
            request.mode == GroupCleanupMode::Force,
        )?;
        let mut locked = orchestration::lock_registry(context)?;
        if resume_state.is_none()
            && group_cleanup_replay_with_legacy_alias(
                context,
                &locked.registry,
                main_session_id,
                legacy_alias,
                &incarnation,
                &request.idempotency_key,
                &request_digest,
                operation,
            )?
            .is_some()
        {
            return Err(CliError::runtime(
                "group-cleanup-in-progress",
                "an identical group cleanup invocation is already making progress",
                None,
            ));
        }
        let run = locked
            .registry
            .runs
            .values()
            .find(|run| run.state == "active" && run.controller == main)
            .cloned()
            .ok_or_else(|| {
                not_found(
                    "main-agent-run-not-found",
                    "session is not the current controller of an active Main Agent run",
                )
            })?;
        ensure_revision(request.expected_run_revision, run.revision, "run")?;
        let current_plan = build_group_cleanup_plan(&locked.registry, &run, &main)?;
        if current_plan.plan_digest != request.expected_plan_digest {
            return Err(CliError::data(
                "group-cleanup-plan-conflict",
                "Main Agent group cleanup plan changed before authority was fenced",
                Some(json!({
                    "expected_plan_digest": request.expected_plan_digest,
                    "current_plan_digest": current_plan.plan_digest,
                    "current_run_revision": run.revision,
                })),
            ));
        }
        group_cleanup_assignment_transitions(context, &locked.registry, &run, &main, request.mode)?;
        let prior_results = resume_state
            .as_ref()
            .map(|resume| resume.worker_results.clone())
            .unwrap_or_default();
        let prior_fences = resume_state
            .as_ref()
            .map(|resume| resume.deleted_registry_fences.clone())
            .unwrap_or_default();
        let prior_pending_fences = resume_state
            .as_ref()
            .map(|resume| resume.pending_registry_fences.clone())
            .unwrap_or_default();
        if resume_state.is_none() {
            let initial_resume = GroupCleanupResumeState {
                schema_version: "agent-session.main-agent-group-cleanup-progress.v1".to_string(),
                plan: current_plan.clone(),
                authority_sealed: false,
                worker_results: prior_results.clone(),
                deleted_registry_fences: prior_fences.clone(),
                pending_registry_fences: prior_pending_fences.clone(),
                run_closed: false,
            };
            let initial_value = group_cleanup_progress_value(
                &current_plan,
                &prior_results,
                false,
                "authority_fence",
            );
            store_receipt_for_principal(
                &mut locked.registry,
                main_session_id,
                &incarnation,
                &request.idempotency_key,
                operation,
                &request_digest,
                group_cleanup_stored_outcome(&initial_value, &initial_resume)?,
            )?;
            // The initial progress receipt is the first durable transition.
            // Every later external effect is therefore adoptable by exact retry.
            locked.save()?;
            interrupt_group_cleanup_for_test(context, "authority_fence")?;
        }
        // The exact session record locks above serialize this durable fence
        // against resume and broker reprovision. Persist it before sealing the
        // coordination broker and before making assignment terminalization
        // durable; an interrupted retry adopts the same fence.
        for (session, present) in &locked_session_authorities {
            if present.is_some() {
                orchestration::persist_session_group_cleanup_fence(
                    context,
                    session,
                    &main,
                    &current_plan.run_id,
                    &current_plan.plan_digest,
                )?;
            }
        }
        cleanup_quiescence.seal(context)?;
        // Coordination authority is sealed before assignment state changes
        // become durable. The sealed progress update and assignment transition
        // share the orchestration save.
        prepare_group_cleanup_assignments(
            context,
            &mut locked.registry,
            &run,
            &main,
            request.mode,
        )?;
        let sealed_resume = group_cleanup_resume_state(
            &current_plan,
            &prior_results,
            &prior_fences,
            &prior_pending_fences,
            false,
        );
        let sealed_value =
            group_cleanup_progress_value(&current_plan, &prior_results, false, "authority_sealed");
        store_receipt_for_principal(
            &mut locked.registry,
            main_session_id,
            &incarnation,
            &request.idempotency_key,
            operation,
            &request_digest,
            group_cleanup_stored_outcome(&sealed_value, &sealed_resume)?,
        )?;
        locked.save()?;
        interrupt_group_cleanup_for_test(context, "authority_sealed")?;
        current_plan
    } else {
        for (session, present) in &locked_session_authorities {
            if present.is_some() {
                orchestration::persist_session_group_cleanup_fence(
                    context,
                    session,
                    &main,
                    &plan.run_id,
                    &plan.plan_digest,
                )?;
            }
        }
        plan
    };
    drop(locked_session_authorities);

    let mut deleted_registry_fences = resume_state
        .as_ref()
        .map(|resume| resume.deleted_registry_fences.clone())
        .unwrap_or_default();
    let mut pending_registry_fences = resume_state
        .as_ref()
        .map(|resume| resume.pending_registry_fences.clone())
        .unwrap_or_default();
    let mut worker_results = resume_state
        .as_ref()
        .map(|resume| {
            resume
                .worker_results
                .iter()
                .filter(|result| {
                    !matches!(
                        result["outcome"].as_str(),
                        Some("failed" | "delete_pending")
                    )
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for worker_plan in &plan.workers {
        if worker_results.iter().any(|result| {
            result["assignment_id"] == worker_plan.assignment_id
                && matches!(
                    result["outcome"].as_str(),
                    Some("deleted" | "absent" | "not_started")
                )
        }) {
            continue;
        }
        let Some(worker) = worker_plan.worker.as_ref() else {
            worker_results.push(json!({
                "assignment_id": worker_plan.assignment_id,
                "session_id": null,
                "outcome": "not_started",
                "cleanup_pending": false,
            }));
            let progress =
                group_cleanup_progress_value(&plan, &worker_results, false, "worker_checkpoint");
            store_group_cleanup_receipt(
                context,
                &progress_identity,
                &request,
                &request_digest,
                progress,
                group_cleanup_resume_state(
                    &plan,
                    &worker_results,
                    &deleted_registry_fences,
                    &pending_registry_fences,
                    false,
                ),
            )?;
            interrupt_group_cleanup_for_test(context, "worker_checkpoint")?;
            continue;
        };
        let worker_path = session_dir(context, &worker.session_id);
        match fs::symlink_metadata(&worker_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if let Some(index) = pending_registry_fences
                    .iter()
                    .position(|fence| fence.session_id == worker.session_id)
                {
                    let fence = pending_registry_fences.remove(index);
                    if !deleted_registry_fences.contains(&fence) {
                        deleted_registry_fences.push(fence);
                    }
                }
                worker_results.push(json!({
                    "assignment_id": worker_plan.assignment_id,
                    "session_id": worker.session_id,
                    "outcome": "absent",
                    "cleanup_pending": false,
                }));
                let progress = group_cleanup_progress_value(
                    &plan,
                    &worker_results,
                    false,
                    "worker_checkpoint",
                );
                store_group_cleanup_receipt(
                    context,
                    &progress_identity,
                    &request,
                    &request_digest,
                    progress,
                    group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?;
                interrupt_group_cleanup_for_test(context, "worker_checkpoint")?;
                continue;
            }
            Err(_) => {
                let error = CliError::runtime(
                    "worker-session-unavailable",
                    "worker session state is unavailable",
                    None,
                );
                let value = group_cleanup_failure(
                    &plan,
                    &worker_results,
                    Some(worker_plan),
                    "worker_cleanup",
                    &error,
                    false,
                );
                store_group_cleanup_receipt(
                    context,
                    &progress_identity,
                    &request,
                    &request_digest,
                    value.clone(),
                    group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?;
                return Ok(GroupCleanupExecution {
                    value,
                    deleted_registry_fences,
                });
            }
            Ok(_) => {}
        }
        let worker_record = match load_session_record(context, &worker.session_id) {
            Ok(record) => record,
            Err(error) => {
                let value = group_cleanup_failure(
                    &plan,
                    &worker_results,
                    Some(worker_plan),
                    "worker_cleanup",
                    &error,
                    false,
                );
                store_group_cleanup_receipt(
                    context,
                    &progress_identity,
                    &request,
                    &request_digest,
                    value.clone(),
                    group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?;
                return Ok(GroupCleanupExecution {
                    value,
                    deleted_registry_fences,
                });
            }
        };
        let worker_incarnation = worker_record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str())
            .unwrap_or_default();
        if !orchestration::session_ref_matches(worker, &worker_record, worker_incarnation) {
            let error = CliError::data(
                "session-incarnation-conflict",
                "worker session identity changed before group cleanup",
                Some(json!({ "assignment_id": worker_plan.assignment_id })),
            );
            let value = group_cleanup_failure(
                &plan,
                &worker_results,
                Some(worker_plan),
                "worker_cleanup",
                &error,
                false,
            );
            store_group_cleanup_receipt(
                context,
                &progress_identity,
                &request,
                &request_digest,
                value.clone(),
                group_cleanup_resume_state(
                    &plan,
                    &worker_results,
                    &deleted_registry_fences,
                    &pending_registry_fences,
                    false,
                ),
            )?;
            return Ok(GroupCleanupExecution {
                value,
                deleted_registry_fences,
            });
        }
        let pending_fence = SessionRegistryFence::from_record(&worker_record);
        if !pending_registry_fences.contains(&pending_fence) {
            pending_registry_fences.push(pending_fence.clone());
        }
        worker_results.push(json!({
            "assignment_id": worker_plan.assignment_id,
            "session_id": worker.session_id,
            "outcome": "delete_pending",
            "cleanup_pending": false,
        }));
        let pending_value =
            group_cleanup_progress_value(&plan, &worker_results, false, "worker_delete_pending");
        store_group_cleanup_receipt(
            context,
            &progress_identity,
            &request,
            &request_digest,
            pending_value,
            group_cleanup_resume_state(
                &plan,
                &worker_results,
                &deleted_registry_fences,
                &pending_registry_fences,
                false,
            ),
        )?;
        interrupt_group_cleanup_for_test(
            context,
            &format!("worker_delete_pending:{}", worker_plan.assignment_id),
        )?;
        let deletion = if archive {
            crate::archive_session_for_group_cleanup_with_expected_incarnation(
                context,
                &worker.session_id,
                tmux_bin.clone(),
                &worker.session_incarnation,
            )
            .map(|(_, deleted)| deleted)
        } else {
            crate::delete_session_for_group_cleanup(context, &worker.session_id, tmux_bin.clone())
        };
        match deletion {
            Ok(deleted) => {
                interrupt_group_cleanup_for_test(
                    context,
                    &format!(
                        "worker_deleted_uncheckpointed:{}",
                        worker_plan.assignment_id
                    ),
                )?;
                worker_results.retain(|result| {
                    result["assignment_id"] != worker_plan.assignment_id
                        || result["outcome"] != "delete_pending"
                });
                pending_registry_fences.retain(|fence| fence != &pending_fence);
                worker_results.push(json!({
                    "assignment_id": worker_plan.assignment_id,
                    "session_id": worker.session_id,
                    "outcome": "deleted",
                    "cleanup_pending": deleted.cleanup_pending,
                }));
                deleted_registry_fences.push(deleted.registry_fence);
                let progress =
                    group_cleanup_progress_value(&plan, &worker_results, false, "worker_deleted");
                store_group_cleanup_receipt(
                    context,
                    &progress_identity,
                    &request,
                    &request_digest,
                    progress,
                    group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?;
                interrupt_group_cleanup_for_test(
                    context,
                    &format!("worker_deleted:{}", worker_plan.assignment_id),
                )?;
            }
            Err(error) => {
                worker_results.retain(|result| {
                    result["assignment_id"] != worker_plan.assignment_id
                        || result["outcome"] != "delete_pending"
                });
                let value = group_cleanup_failure(
                    &plan,
                    &worker_results,
                    Some(worker_plan),
                    "worker_cleanup",
                    &error,
                    false,
                );
                store_group_cleanup_receipt(
                    context,
                    &progress_identity,
                    &request,
                    &request_digest,
                    value.clone(),
                    group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?;
                return Ok(GroupCleanupExecution {
                    value,
                    deleted_registry_fences,
                });
            }
        }
    }

    {
        let mut locked = orchestration::lock_registry(context)?;
        let run = locked
            .registry
            .runs
            .get(&plan.run_id)
            .cloned()
            .ok_or_else(|| {
                CliError::data(
                    "group-cleanup-run-conflict",
                    "Main Agent run changed while workers were being cleaned up",
                    None,
                )
            })?;
        if run.controller != main
            || !matches!(
                (run.state.as_str(), run.revision),
                ("active", revision) if revision == request.expected_run_revision
            ) && !matches!(
                (run.state.as_str(), run.revision),
                ("closed", revision) if revision == closed_run_revision
            )
        {
            let error = CliError::data(
                "group-cleanup-run-conflict",
                "Main Agent run changed while workers were being cleaned up",
                Some(json!({
                    "current_run_revision": run.revision,
                    "current_run_state": run.state
                })),
            );
            let value =
                group_cleanup_failure(&plan, &worker_results, None, "run_close", &error, false);
            store_receipt_for_principal(
                &mut locked.registry,
                main_session_id,
                &incarnation,
                &request.idempotency_key,
                operation,
                &request_digest,
                group_cleanup_stored_outcome(
                    &value,
                    &group_cleanup_resume_state(
                        &plan,
                        &worker_results,
                        &deleted_registry_fences,
                        &pending_registry_fences,
                        false,
                    ),
                )?,
            )?;
            locked.save()?;
            return Ok(GroupCleanupExecution {
                value,
                deleted_registry_fences,
            });
        }
        if run.state == "active" {
            let run = locked
                .registry
                .runs
                .get_mut(&plan.run_id)
                .expect("run checked above");
            run.state = "closed".to_string();
            run.revision = closed_run_revision;
            run.updated_at = timestamp();
        }
        let run_closed_resume = group_cleanup_resume_state(
            &plan,
            &worker_results,
            &deleted_registry_fences,
            &pending_registry_fences,
            true,
        );
        let run_closed_value =
            group_cleanup_progress_value(&plan, &worker_results, true, "run_closed");
        store_receipt_for_principal(
            &mut locked.registry,
            main_session_id,
            &incarnation,
            &request.idempotency_key,
            operation,
            &request_digest,
            group_cleanup_stored_outcome(&run_closed_value, &run_closed_resume)?,
        )?;
        locked.save()?;
    }
    interrupt_group_cleanup_for_test(context, "run_closed")?;

    let current_main = match load_session_record(context, main_session_id) {
        Ok(record) => record,
        Err(error) if error.code() == "session-not-found" => {
            let Some(index) = pending_registry_fences
                .iter()
                .position(|fence| fence.session_id == main_session_id)
            else {
                return Err(error);
            };
            let fence = pending_registry_fences.remove(index);
            if !deleted_registry_fences.contains(&fence) {
                deleted_registry_fences.push(fence);
            }
            let value = json!({
                "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                "run_id": plan.run_id,
                "completed": true,
                "run_closed": true,
                "main_deleted": true,
                "workers": worker_results,
            });
            let completed_resume = group_cleanup_resume_state(
                &plan,
                &worker_results,
                &deleted_registry_fences,
                &pending_registry_fences,
                true,
            );
            store_completed_group_cleanup_receipt(
                context,
                main_session_id,
                legacy_alias,
                &incarnation,
                &request,
                &request_digest,
                operation,
                group_cleanup_stored_outcome(&value, &completed_resume)?,
            )?;
            interrupt_group_cleanup_for_test(context, "main_deleted")?;
            return Ok(GroupCleanupExecution {
                value,
                deleted_registry_fences,
            });
        }
        Err(error) => return Err(error),
    };
    let current_main_incarnation = current_main
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.as_str())
        .unwrap_or_default();
    if !orchestration::session_ref_matches(&main, &current_main, current_main_incarnation) {
        let error = CliError::data(
            "main-session-incarnation-conflict",
            "Main Agent session identity changed before final deletion",
            None,
        );
        let value =
            group_cleanup_failure(&plan, &worker_results, None, "main_delete", &error, true);
        store_group_cleanup_receipt(
            context,
            &progress_identity,
            &request,
            &request_digest,
            value.clone(),
            group_cleanup_resume_state(
                &plan,
                &worker_results,
                &deleted_registry_fences,
                &pending_registry_fences,
                true,
            ),
        )?;
        return Ok(GroupCleanupExecution {
            value,
            deleted_registry_fences,
        });
    }
    let pending_main_fence = SessionRegistryFence::from_record(&current_main);
    if !pending_registry_fences.contains(&pending_main_fence) {
        pending_registry_fences.push(pending_main_fence.clone());
    }
    let pending_value =
        group_cleanup_progress_value(&plan, &worker_results, true, "main_delete_pending");
    store_group_cleanup_receipt(
        context,
        &progress_identity,
        &request,
        &request_digest,
        pending_value,
        group_cleanup_resume_state(
            &plan,
            &worker_results,
            &deleted_registry_fences,
            &pending_registry_fences,
            true,
        ),
    )?;
    interrupt_group_cleanup_for_test(context, "main_delete_pending")?;
    let main_deletion = if archive {
        crate::archive_session_for_group_cleanup_with_expected_incarnation(
            context,
            main_session_id,
            tmux_bin,
            &incarnation,
        )
        .map(|(_, deleted)| deleted)
    } else {
        crate::delete_session_for_group_cleanup(context, main_session_id, tmux_bin)
    };
    let main_deleted = match main_deletion {
        Ok(deleted) => {
            interrupt_group_cleanup_for_test(context, "main_deleted_uncheckpointed")?;
            deleted
        }
        Err(error) => {
            let value =
                group_cleanup_failure(&plan, &worker_results, None, "main_delete", &error, true);
            store_group_cleanup_receipt(
                context,
                &progress_identity,
                &request,
                &request_digest,
                value.clone(),
                group_cleanup_resume_state(
                    &plan,
                    &worker_results,
                    &deleted_registry_fences,
                    &pending_registry_fences,
                    true,
                ),
            )?;
            return Ok(GroupCleanupExecution {
                value,
                deleted_registry_fences,
            });
        }
    };
    pending_registry_fences.retain(|fence| fence != &pending_main_fence);
    if !deleted_registry_fences.contains(&main_deleted.registry_fence) {
        deleted_registry_fences.push(main_deleted.registry_fence);
    }
    let value = json!({
        "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
        "run_id": plan.run_id,
        "completed": true,
        "run_closed": true,
        "main_deleted": true,
        "workers": worker_results,
    });
    let completed_resume = group_cleanup_resume_state(
        &plan,
        &worker_results,
        &deleted_registry_fences,
        &pending_registry_fences,
        true,
    );
    store_completed_group_cleanup_receipt(
        context,
        main_session_id,
        legacy_alias,
        &incarnation,
        &request,
        &request_digest,
        operation,
        group_cleanup_stored_outcome(&value, &completed_resume)?,
    )?;
    interrupt_group_cleanup_for_test(context, "main_deleted")?;
    Ok(GroupCleanupExecution {
        value,
        deleted_registry_fences,
    })
}

fn validate_group_cleanup_request(
    main_session_id: &str,
    request: &GroupCleanupRequest,
) -> Result<(), CliError> {
    crate::validate_id(main_session_id)?;
    if request.schema_version != GROUP_CLEANUP_REQUEST_SCHEMA {
        return Err(invalid_input("group cleanup request schema is unsupported"));
    }
    orchestration::validate_slug(
        "main session incarnation",
        &request.expected_main_incarnation,
        128,
    )?;
    orchestration::validate_digest(&request.expected_plan_digest)?;
    validate_idempotency_key(&request.idempotency_key)
}

#[cfg(test)]
pub(crate) fn group_cleanup_request_digest(request: &GroupCleanupRequest) -> String {
    group_cleanup_operation_request_digest(request, "group-cleanup")
}

fn group_cleanup_operation_request_digest(
    request: &GroupCleanupRequest,
    operation: &str,
) -> String {
    let namespace = if operation == "group-archive" {
        "main-agent-group-archive"
    } else {
        "main-agent-group-cleanup"
    };
    crate::coordination::request_digest(
        namespace,
        &json!({
            "expected_main_incarnation": request.expected_main_incarnation,
            "expected_run_revision": request.expected_run_revision,
            "expected_plan_digest": request.expected_plan_digest,
            "mode": request.mode,
        }),
    )
}

pub(crate) fn group_cleanup_execution_owner_digest(main_session_id: &str) -> String {
    crate::coordination::request_digest(
        "main-agent-group-cleanup-owner",
        &json!({ "main_session_id": main_session_id }),
    )
}

#[derive(Debug)]
pub(crate) struct GroupCleanupExecutionLock {
    pub(crate) _file: fs::File,
}

pub(crate) fn lock_group_cleanup_execution(
    context: &CliContext,
    request_digest: &str,
) -> Result<GroupCleanupExecutionLock, CliError> {
    fs::create_dir_all(&context.state_dir).map_err(|_| {
        CliError::runtime(
            "orchestration-store-unavailable",
            "orchestration store is unavailable",
            None,
        )
    })?;
    let directory_path = orchestration::ensure_orchestration_root(context)?;
    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory_path)
        .map_err(|_| {
            CliError::runtime(
                "orchestration-store-unavailable",
                "orchestration store is unavailable",
                None,
            )
        })?;
    let directory_metadata = directory.metadata().map_err(|_| {
        CliError::runtime(
            "orchestration-store-unavailable",
            "orchestration store is unavailable",
            None,
        )
    })?;
    if !directory_metadata.is_dir()
        || directory_metadata.uid() != unsafe { libc::geteuid() }
        || directory_metadata.mode() & 0o077 != 0
    {
        return Err(CliError::data(
            "orchestration-store-invalid",
            "orchestration store root is unsafe",
            None,
        ));
    }
    let name = CString::new(format!("group-cleanup-{request_digest}.lock"))
        .map_err(|_| invalid_input("group cleanup request digest is invalid"))?;
    // SAFETY: the directory descriptor is a validated, non-symlinked private
    // orchestration root, and the returned descriptor is owned by `lock`.
    let descriptor = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            nils_common::fs::SECRET_FILE_MODE,
        )
    };
    if descriptor < 0 {
        return Err(CliError::runtime(
            "orchestration-store-unavailable",
            "orchestration store is unavailable",
            None,
        ));
    }
    // SAFETY: `openat` returned a newly owned descriptor.
    let lock = unsafe { fs::File::from_raw_fd(descriptor) };
    let lock_metadata = lock.metadata().map_err(|_| {
        CliError::runtime(
            "orchestration-store-unavailable",
            "orchestration store is unavailable",
            None,
        )
    })?;
    if !lock_metadata.is_file()
        || lock_metadata.uid() != unsafe { libc::geteuid() }
        || lock_metadata.mode() & 0o077 != 0
    {
        return Err(CliError::data(
            "orchestration-store-invalid",
            "orchestration cleanup lock is unsafe",
            None,
        ));
    }
    // SAFETY: the descriptor remains open for the duration of the execution.
    let result = unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        return Err(CliError::runtime(
            "group-cleanup-in-progress",
            "an identical group cleanup invocation is already making progress",
            None,
        ));
    }
    Ok(GroupCleanupExecutionLock { _file: lock })
}

struct GroupCleanupReplaySelector<'a> {
    progress_principal_session_id: &'a str,
    requested_session_id: &'a str,
    include_registry_outcome: bool,
}

fn group_cleanup_replay(
    context: &CliContext,
    registry: &orchestration::Registry,
    selector: GroupCleanupReplaySelector<'_>,
    incarnation: &str,
    idempotency_key: &str,
    request_digest: &str,
    operation: &str,
) -> Result<Option<GroupCleanupReplay>, CliError> {
    let registry_outcome = if selector.include_registry_outcome
        && let Some(receipt) = registry.receipts.get(&receipt_key(
            selector.progress_principal_session_id,
            incarnation,
            idempotency_key,
        )) {
        if receipt.operation != operation || receipt.request_digest != request_digest {
            return Err(CliError::data(
                "idempotency-conflict",
                "idempotency key was already used for a different request",
                None,
            ));
        }
        let replay = decode_group_cleanup_replay(receipt.outcome.clone())?;
        if replay.value["completed"] == true {
            return Ok(Some(replay));
        }
        Some(replay)
    } else {
        None
    };
    let key = group_cleanup_progress_key(
        selector.progress_principal_session_id,
        incarnation,
        idempotency_key,
    );
    let Some(bytes) = orchestration::read_group_cleanup_progress(context, &key)? else {
        return Ok(registry_outcome);
    };
    let receipt = orchestration::decode_group_cleanup_progress_receipt(&bytes).map_err(|_| {
        CliError::data(
            "group-cleanup-progress-invalid",
            "durable group cleanup progress is invalid",
            None,
        )
    })?;
    let (Some(canonical_session_id), Some(canonical_incarnation)) = (
        receipt.outcome["_resume"]["plan"]["main"]["session_id"].as_str(),
        receipt.outcome["_resume"]["plan"]["main"]["session_incarnation"].as_str(),
    ) else {
        return Err(CliError::data(
            "group-cleanup-progress-invalid",
            "durable group cleanup progress identity is invalid",
            None,
        ));
    };
    if receipt.principal_session_id != selector.progress_principal_session_id
        || receipt.idempotency_key != idempotency_key
        || !(orchestration::GroupCleanupSelectorBinding {
            schema_version: &receipt.schema_version,
            requested_session_id: receipt.requested_session_id.as_deref(),
            stored_principal_session_id: &receipt.principal_session_id,
            canonical_session_id,
            stored_incarnation: &receipt.principal_incarnation,
            canonical_incarnation,
            expected_session_id: selector.requested_session_id,
            expected_incarnation: incarnation,
        })
        .is_exact()
    {
        return Err(CliError::data(
            "group-cleanup-progress-invalid",
            "durable group cleanup progress identity is invalid",
            None,
        ));
    }
    if receipt.request_digest != request_digest {
        return Err(CliError::data(
            "idempotency-conflict",
            "idempotency key was already used for a different request",
            None,
        ));
    }
    decode_group_cleanup_replay(receipt.outcome).map(Some)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn group_cleanup_replay_with_legacy_alias(
    context: &CliContext,
    registry: &orchestration::Registry,
    canonical_main_session_id: &str,
    legacy_alias: Option<&str>,
    incarnation: &str,
    idempotency_key: &str,
    request_digest: &str,
    operation: &str,
) -> Result<Option<GroupCleanupReplay>, CliError> {
    let Some(legacy_alias) = legacy_alias else {
        return group_cleanup_replay(
            context,
            registry,
            GroupCleanupReplaySelector {
                progress_principal_session_id: canonical_main_session_id,
                requested_session_id: canonical_main_session_id,
                include_registry_outcome: true,
            },
            incarnation,
            idempotency_key,
            request_digest,
            operation,
        );
    };
    let validate_alias_replay = |replay: &GroupCleanupReplay| {
        replay.resume.as_ref().is_none_or(|resume| {
            resume.plan.main.session_id != canonical_main_session_id
                || resume.plan.main.session_incarnation != incarnation
        })
    };
    let legacy_replay = group_cleanup_replay(
        context,
        registry,
        GroupCleanupReplaySelector {
            progress_principal_session_id: legacy_alias,
            requested_session_id: legacy_alias,
            include_registry_outcome: true,
        },
        incarnation,
        idempotency_key,
        request_digest,
        operation,
    )?;
    if let Some(replay) = legacy_replay.as_ref()
        && validate_alias_replay(replay)
    {
        return Err(CliError::data(
            "main-session-incarnation-conflict",
            "session alias resolved to a different Main Agent cleanup principal",
            None,
        ));
    }
    if legacy_replay.is_some() {
        return Ok(legacy_replay);
    }
    let canonical_progress = group_cleanup_replay(
        context,
        registry,
        GroupCleanupReplaySelector {
            progress_principal_session_id: canonical_main_session_id,
            requested_session_id: legacy_alias,
            include_registry_outcome: false,
        },
        incarnation,
        idempotency_key,
        request_digest,
        operation,
    )?;
    if let Some(replay) = canonical_progress.as_ref()
        && validate_alias_replay(replay)
    {
        return Err(CliError::data(
            "main-session-incarnation-conflict",
            "session alias resolved to a different Main Agent cleanup principal",
            None,
        ));
    }
    Ok(canonical_progress)
}

pub(crate) fn recover_completed_group_cleanup_principal(
    registry: &orchestration::Registry,
    requested_session_id: &str,
    incarnation: &str,
    idempotency_key: &str,
    request_digest: &str,
    operation: &str,
) -> Result<Option<String>, CliError> {
    let mut recovered = None;
    for (key, receipt) in &registry.receipts {
        if receipt.operation != operation
            || receipt.request_digest != request_digest
            || receipt.outcome["completed"] != true
            || key != &receipt_key(&receipt.principal_session_id, incarnation, idempotency_key)
        {
            continue;
        }
        let Some(plan_main) = receipt.outcome["_resume"]["plan"]["main"].as_object() else {
            continue;
        };
        let Some(canonical_session_id) = plan_main["session_id"].as_str() else {
            continue;
        };
        let Some(canonical_incarnation) = plan_main["session_incarnation"].as_str() else {
            continue;
        };
        if crate::validate_id(canonical_session_id).is_err()
            || !(orchestration::GroupCleanupSelectorBinding {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA,
                requested_session_id: None,
                stored_principal_session_id: &receipt.principal_session_id,
                canonical_session_id,
                stored_incarnation: &receipt.principal_incarnation,
                canonical_incarnation,
                expected_session_id: requested_session_id,
                expected_incarnation: incarnation,
            })
            .is_exact()
        {
            continue;
        }
        if recovered
            .as_deref()
            .is_some_and(|existing| existing != canonical_session_id)
        {
            return Err(CliError::data(
                "group-cleanup-progress-conflict",
                "multiple completed cleanup principals matched the requested session alias",
                None,
            ));
        }
        recovered = Some(canonical_session_id.to_string());
    }
    Ok(recovered)
}

fn remove_legacy_group_cleanup_progress(
    context: &CliContext,
    legacy_alias: Option<&str>,
    incarnation: &str,
    idempotency_key: &str,
) -> Result<(), CliError> {
    let Some(legacy_alias) = legacy_alias else {
        return Ok(());
    };
    orchestration::remove_group_cleanup_progress(
        context,
        &group_cleanup_progress_key(legacy_alias, incarnation, idempotency_key),
    )
}

fn decode_group_cleanup_replay(mut value: Value) -> Result<GroupCleanupReplay, CliError> {
    let resume = value
        .as_object_mut()
        .and_then(|object| object.remove("_resume"))
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| {
            CliError::data(
                "group-cleanup-progress-invalid",
                "durable group cleanup progress is invalid",
                None,
            )
        })?;
    Ok(GroupCleanupReplay { value, resume })
}

pub(crate) fn group_cleanup_progress_key(
    main_session_id: &str,
    incarnation: &str,
    idempotency_key: &str,
) -> String {
    crate::coordination::request_digest(
        "main-agent-group-cleanup-progress-key",
        &json!({
            "main_session_id": main_session_id,
            "incarnation": incarnation,
            "idempotency_key": idempotency_key,
        }),
    )
}

pub(crate) fn group_cleanup_stored_outcome(
    value: &Value,
    resume: &GroupCleanupResumeState,
) -> Result<Value, CliError> {
    let mut stored = value.clone();
    let object = stored.as_object_mut().ok_or_else(|| {
        CliError::runtime(
            "group-cleanup-progress-invalid",
            "group cleanup result is not an object",
            None,
        )
    })?;
    object.insert(
        "_resume".to_string(),
        serde_json::to_value(resume).map_err(|_| {
            CliError::runtime(
                "group-cleanup-progress-invalid",
                "group cleanup progress could not be serialized",
                None,
            )
        })?,
    );
    Ok(stored)
}

pub(crate) fn group_cleanup_resume_state(
    plan: &GroupCleanupPlan,
    worker_results: &[Value],
    deleted_registry_fences: &[SessionRegistryFence],
    pending_registry_fences: &[SessionRegistryFence],
    run_closed: bool,
) -> GroupCleanupResumeState {
    GroupCleanupResumeState {
        schema_version: "agent-session.main-agent-group-cleanup-progress.v1".to_string(),
        plan: plan.clone(),
        authority_sealed: true,
        worker_results: worker_results.to_vec(),
        deleted_registry_fences: deleted_registry_fences.to_vec(),
        pending_registry_fences: pending_registry_fences.to_vec(),
        run_closed,
    }
}

pub(crate) fn group_cleanup_progress_value(
    plan: &GroupCleanupPlan,
    worker_results: &[Value],
    run_closed: bool,
    stage: &str,
) -> Value {
    json!({
        "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
        "run_id": plan.run_id,
        "completed": false,
        "run_closed": run_closed,
        "main_deleted": false,
        "workers": worker_results,
        "progress": { "stage": stage },
    })
}

fn interrupt_group_cleanup_for_test(context: &CliContext, stage: &str) -> Result<(), CliError> {
    #[cfg(debug_assertions)]
    if fs::read_to_string(context.state_dir.join("group-cleanup-interrupt-test"))
        .ok()
        .as_deref()
        == Some(stage)
    {
        return Err(CliError::runtime(
            "group-cleanup-test-interrupted",
            "group cleanup interrupted after its durable stage checkpoint",
            Some(json!({ "stage": stage })),
        ));
    }
    let _ = context;
    Ok(())
}

pub(crate) fn store_group_cleanup_receipt(
    context: &CliContext,
    identity: &GroupCleanupProgressIdentity<'_>,
    request: &GroupCleanupRequest,
    request_digest: &str,
    value: Value,
    resume: GroupCleanupResumeState,
) -> Result<(), CliError> {
    let outcome = group_cleanup_stored_outcome(&value, &resume)?;
    let progress_key = group_cleanup_progress_key(
        identity.principal_session_id,
        identity.incarnation,
        &request.idempotency_key,
    );
    if value["completed"] == true {
        return store_completed_group_cleanup_receipt(
            context,
            identity.principal_session_id,
            (identity.requested_session_id != identity.principal_session_id)
                .then_some(identity.requested_session_id),
            identity.incarnation,
            request,
            request_digest,
            identity.operation,
            outcome,
        );
    }
    let progress = GroupCleanupProgressReceipt {
        schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_SCHEMA.to_string(),
        requested_session_id: Some(identity.requested_session_id.to_string()),
        principal_session_id: identity.principal_session_id.to_string(),
        principal_incarnation: identity.incarnation.to_string(),
        idempotency_key: request.idempotency_key.clone(),
        request_digest: request_digest.to_string(),
        outcome,
    };
    let bytes = serde_json::to_vec(&progress).map_err(|_| {
        CliError::runtime(
            "group-cleanup-progress-invalid",
            "group cleanup progress could not be serialized",
            None,
        )
    })?;
    orchestration::store_group_cleanup_progress(context, &progress_key, &bytes)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn store_completed_group_cleanup_receipt(
    context: &CliContext,
    principal_session_id: &str,
    legacy_alias: Option<&str>,
    incarnation: &str,
    request: &GroupCleanupRequest,
    request_digest: &str,
    operation: &str,
    outcome: Value,
) -> Result<(), CliError> {
    let mut locked = orchestration::lock_registry(context)?;
    let canonical_key = receipt_key(principal_session_id, incarnation, &request.idempotency_key);
    let legacy_key =
        legacy_alias.map(|alias| receipt_key(alias, incarnation, &request.idempotency_key));
    let destinations = legacy_key
        .iter()
        .chain(std::iter::once(&canonical_key))
        .cloned()
        .collect::<Vec<_>>();
    for (key, expected_principal) in legacy_key
        .as_ref()
        .zip(legacy_alias)
        .into_iter()
        .chain(std::iter::once((&canonical_key, principal_session_id)))
    {
        if let Some(existing) = locked.registry.receipts.get(key)
            && (existing.principal_session_id != expected_principal
                || existing.principal_incarnation != incarnation
                || existing.operation != operation
                || existing.request_digest != request_digest)
        {
            return Err(CliError::data(
                "idempotency-conflict",
                "cleanup receipt conflicts with the completed canonical request",
                None,
            ));
        }
    }
    let new_destinations = destinations
        .iter()
        .filter(|key| !locked.registry.receipts.contains_key(*key))
        .count();
    while locked
        .registry
        .receipts
        .len()
        .saturating_add(new_destinations)
        > idempotency_receipt_capacity()
    {
        let victim = locked
            .registry
            .receipts
            .iter()
            .filter(|(key, _)| !destinations.contains(key))
            .min_by_key(|(_, receipt)| receipt.created_at_epoch)
            .map(|(key, _)| key.clone())
            .ok_or_else(|| {
                CliError::unavailable(
                    "orchestration-store-capacity",
                    "orchestration receipt capacity is exhausted",
                    None,
                )
            })?;
        locked.registry.receipts.remove(&victim);
    }
    let created_at_epoch = crate::coordination::now_epoch();
    if let (Some(legacy_alias), Some(legacy_key)) = (legacy_alias, legacy_key) {
        locked.registry.receipts.insert(
            legacy_key,
            IdempotencyReceipt {
                principal_session_id: legacy_alias.to_string(),
                principal_incarnation: incarnation.to_string(),
                operation: operation.to_string(),
                request_digest: request_digest.to_string(),
                outcome: outcome.clone(),
                created_at_epoch,
            },
        );
    }
    locked.registry.receipts.insert(
        canonical_key,
        IdempotencyReceipt {
            principal_session_id: principal_session_id.to_string(),
            principal_incarnation: incarnation.to_string(),
            operation: operation.to_string(),
            request_digest: request_digest.to_string(),
            outcome,
            created_at_epoch,
        },
    );
    locked.save()?;
    drop(locked);
    orchestration::remove_group_cleanup_progress(
        context,
        &group_cleanup_progress_key(principal_session_id, incarnation, &request.idempotency_key),
    )?;
    remove_legacy_group_cleanup_progress(
        context,
        legacy_alias,
        incarnation,
        &request.idempotency_key,
    )
}

fn group_cleanup_failure(
    plan: &GroupCleanupPlan,
    prior_results: &[Value],
    failed_worker: Option<&GroupCleanupWorkerPlan>,
    stage: &str,
    error: &CliError,
    run_closed: bool,
) -> Value {
    let mut workers = prior_results.to_vec();
    if let Some(worker) = failed_worker {
        workers.push(json!({
            "assignment_id": worker.assignment_id,
            "session_id": worker.worker.as_ref().map(|item| item.session_id.as_str()),
            "outcome": "failed",
            "cleanup_pending": false,
            "error_code": error.code(),
        }));
    }
    json!({
        "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
        "run_id": plan.run_id,
        "completed": false,
        "run_closed": run_closed,
        "main_deleted": false,
        "workers": workers,
        "failure": {
            "stage": stage,
            "code": error.code(),
            "message": error.message(),
        },
    })
}

pub(crate) fn build_group_cleanup_plan(
    registry: &orchestration::Registry,
    run: &RunRecord,
    main: &SessionRef,
) -> Result<GroupCleanupPlan, CliError> {
    if run.state != "active" || run.controller != *main {
        return Err(CliError::data(
            "main-agent-run-conflict",
            "Main Agent run does not match the requested controller",
            None,
        ));
    }
    let mut workers = registry
        .assignments
        .values()
        .filter(|assignment| assignment.run_id == run.run_id && assignment.primary_manager == *main)
        .map(|assignment| GroupCleanupWorkerPlan {
            assignment_id: assignment.assignment_id.clone(),
            state: assignment.state.clone(),
            worker: assignment.worker.clone(),
            force_required: !matches!(
                assignment.state.as_str(),
                "accepted" | "released" | "cancelled"
            ),
            primary_managed: true,
        })
        .collect::<Vec<_>>();
    if workers.len() > GROUP_CLEANUP_MAX_ASSIGNMENTS {
        return Err(CliError::data(
            "group-cleanup-batch-too-large",
            "group cleanup is bounded to 64 primary assignments so resumable per-worker checkpoints and lock hold times remain bounded",
            Some(json!({
                "assignment_count": workers.len(),
                "maximum_assignment_count": GROUP_CLEANUP_MAX_ASSIGNMENTS
            })),
        ));
    }
    workers.sort_by(|left, right| left.assignment_id.cmp(&right.assignment_id));
    let requires_force = workers.iter().any(|worker| worker.force_required);
    let digest = format!(
        "sha256:{}",
        crate::coordination::request_digest(
            "main-agent-group-cleanup-plan",
            &json!({
                "schema_version": GROUP_CLEANUP_SCHEMA,
                "main": main,
                "run_id": run.run_id,
                "run_revision": run.revision,
                "requires_force": requires_force,
                "workers": workers,
            }),
        )
    );
    Ok(GroupCleanupPlan {
        schema_version: GROUP_CLEANUP_SCHEMA.to_string(),
        main: main.clone(),
        run_id: run.run_id.clone(),
        run_revision: run.revision,
        requires_force,
        workers,
        plan_digest: digest,
    })
}

pub(crate) fn prepare_group_cleanup_assignments(
    context: &CliContext,
    registry: &mut orchestration::Registry,
    run: &RunRecord,
    main: &SessionRef,
    mode: GroupCleanupMode,
) -> Result<(), CliError> {
    let transitions = group_cleanup_assignment_transitions(context, registry, run, main, mode)?;
    for (assignment_id, next_state, next_revision) in transitions {
        let assignment = registry
            .assignments
            .get_mut(&assignment_id)
            .expect("group cleanup assignment transition was preflighted");
        assignment.state = next_state;
        assignment.revision = next_revision;
        assignment.updated_at = timestamp();
    }
    Ok(())
}

fn group_cleanup_assignment_transitions(
    context: &CliContext,
    registry: &orchestration::Registry,
    run: &RunRecord,
    main: &SessionRef,
    mode: GroupCleanupMode,
) -> Result<Vec<(String, String, u64)>, CliError> {
    let force_required = registry
        .assignments
        .values()
        .filter(|assignment| assignment.run_id == run.run_id && assignment.primary_manager == *main)
        .filter(|assignment| {
            !matches!(
                assignment.state.as_str(),
                "accepted" | "released" | "cancelled"
            )
        })
        .map(|assignment| assignment.assignment_id.clone())
        .collect::<Vec<_>>();
    if mode == GroupCleanupMode::Safe && !force_required.is_empty() {
        return Err(CliError::data(
            "group-cleanup-force-required",
            "group cleanup includes nonterminal assignments and requires explicit force",
            Some(json!({ "assignment_ids": force_required })),
        ));
    }
    for assignment in registry
        .assignments
        .values()
        .filter(|assignment| assignment.run_id == run.run_id && assignment.primary_manager == *main)
    {
        ensure_submit_recovery_not_in_flight(assignment)?;
        ensure_assignment_mutation_admitted(
            context,
            assignment,
            AssignmentMutationOwner::Ordinary,
        )?;
    }
    registry
        .assignments
        .values()
        .filter(|assignment| assignment.run_id == run.run_id && assignment.primary_manager == *main)
        .filter_map(|assignment| {
            let next = match assignment.state.as_str() {
                "accepted" => Some("released"),
                "released" | "cancelled" => None,
                _ if mode == GroupCleanupMode::Force => Some("cancelled"),
                _ => None,
            }?;
            Some(
                assignment
                    .revision
                    .checked_add(1)
                    .map(|revision| (assignment.assignment_id.clone(), next.to_string(), revision))
                    .ok_or_else(|| {
                        CliError::data(
                            "orchestration-revision-capacity",
                            "assignment revision reached its maximum value",
                            Some(json!({
                                "assignment_id": assignment.assignment_id,
                                "current_revision": assignment.revision
                            })),
                        )
                    }),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nils_test_support::GlobalStateLock;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::SessionRecord;
    use crate::cli::CoordinationMode;
    use crate::orchestration::{
        ASSIGNMENT_SCHEMA, AssignmentRecord, SUBMIT_RECOVERY_SCHEMA, SubmitRecoveryRecord,
    };

    fn dep_assignment(id: &str, run_id: &str, state: &str) -> AssignmentRecord {
        AssignmentRecord {
            schema_version: ASSIGNMENT_SCHEMA.to_string(),
            assignment_id: id.to_string(),
            run_id: run_id.to_string(),
            revision: 1,
            state: state.to_string(),
            task_summary: "dependency".to_string(),
            private_packet_digest: format!("sha256:{}", "0".repeat(64)),
            primary_manager: SessionRef {
                machine: None,
                session_id: "main".to_string(),
                session_incarnation: "inc".to_string(),
                session_created_at: "2030-01-01T00:00:00Z".to_string(),
            },
            worker: None,
            previous_worker: None,
            collaborators: Vec::new(),
            borrowed_by: Vec::new(),
            repository: None,
            worktree: None,
            base_ref: None,
            scopes: Vec::new(),
            durable_refs: Vec::new(),
            depends_on: Vec::new(),
            checkpoint: None,
            result_summary: None,
            blocker_summary: None,
            submit_recovery: None,
            worker_quarantine: None,
            account_handoff: None,
            runtime_stop: None,
            claim_revocation: None,
            readiness_stop_proof: None,
            created_at: "2030-01-01T00:00:00Z".to_string(),
            updated_at: "2030-01-01T00:00:00Z".to_string(),
        }
    }

    fn run_record(id: &str, ephemeral: bool) -> RunRecord {
        RunRecord {
            schema_version: orchestration::RUN_SCHEMA.to_string(),
            run_id: id.to_string(),
            revision: 1,
            state: "active".to_string(),
            tier: "direct".to_string(),
            objective_summary: "summary".to_string(),
            objective_packet_digest: format!("sha256:{}", "a".repeat(64)),
            controller: SessionRef {
                machine: None,
                session_id: "main".to_string(),
                session_incarnation: "inc".to_string(),
                session_created_at: "2030-01-01T00:00:00Z".to_string(),
            },
            durable_refs: Vec::new(),
            ephemeral,
            checkpoint: None,
            created_at: "2030-01-01T00:00:00Z".to_string(),
            updated_at: "2030-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn group_cleanup_plan_is_exactly_scoped_and_requires_force_for_live_work() {
        let mut registry = orchestration::Registry::default();
        let run = run_record("run-one", false);
        let controller = run.controller.clone();
        registry.runs.insert(run.run_id.clone(), run.clone());

        let mut accepted = dep_assignment("accepted", "run-one", "accepted");
        accepted.worker = Some(SessionRef {
            machine: None,
            session_id: "worker-accepted".to_string(),
            session_incarnation: "worker-inc-accepted".to_string(),
            session_created_at: "2030-01-01T00:01:00Z".to_string(),
        });
        registry
            .assignments
            .insert(accepted.assignment_id.clone(), accepted);

        let mut submitted = dep_assignment("submitted", "run-one", "submitted");
        submitted.worker = Some(SessionRef {
            machine: None,
            session_id: "worker-submitted".to_string(),
            session_incarnation: "worker-inc-submitted".to_string(),
            session_created_at: "2030-01-01T00:02:00Z".to_string(),
        });
        registry
            .assignments
            .insert(submitted.assignment_id.clone(), submitted);

        let mut collaborator_owned = dep_assignment("borrowed", "run-one", "working");
        collaborator_owned.primary_manager = SessionRef {
            machine: None,
            session_id: "other-main".to_string(),
            session_incarnation: "other-inc".to_string(),
            session_created_at: "2030-01-01T00:00:00Z".to_string(),
        };
        registry
            .assignments
            .insert(collaborator_owned.assignment_id.clone(), collaborator_owned);

        let plan = build_group_cleanup_plan(&registry, &run, &controller).unwrap();

        assert!(plan.requires_force);
        assert_eq!(plan.run_id, "run-one");
        assert_eq!(plan.run_revision, 1);
        assert_eq!(
            plan.workers
                .iter()
                .map(|worker| worker.assignment_id.as_str())
                .collect::<Vec<_>>(),
            vec!["accepted", "submitted"],
        );
        assert!(plan.workers.iter().all(|worker| worker.primary_managed),);
        assert!(plan.plan_digest.starts_with("sha256:"));
    }

    #[test]
    fn group_cleanup_force_terminalizes_primary_assignments_before_deletion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        let mut registry = orchestration::Registry::default();
        let run = run_record("run-one", false);
        let controller = run.controller.clone();
        registry.runs.insert(run.run_id.clone(), run.clone());
        for state in ["working", "submitted", "accepted", "released", "cancelled"] {
            let id = format!("assignment-{state}");
            registry
                .assignments
                .insert(id.clone(), dep_assignment(&id, "run-one", state));
        }

        let safe_error = prepare_group_cleanup_assignments(
            &context,
            &mut registry,
            &run,
            &controller,
            GroupCleanupMode::Safe,
        )
        .unwrap_err();
        assert_eq!(safe_error.code(), "group-cleanup-force-required");
        assert_eq!(registry.assignments["assignment-working"].state, "working");

        registry
            .assignments
            .get_mut("assignment-working")
            .expect("working assignment")
            .submit_recovery = Some(SubmitRecoveryRecord {
            schema_version: SUBMIT_RECOVERY_SCHEMA.to_string(),
            attempt_id: "cleanup-recovery".to_string(),
            origin: "automatic".to_string(),
            run_id: Some("run-one".to_string()),
            controller: Some(controller.clone()),
            session_incarnation: "worker-inc".to_string(),
            reserved_revision: 1,
            state: "attempting".to_string(),
            attempt_count: 1,
            result: "recovery reserved".to_string(),
            attempted_at: "2030-01-01T00:00:01Z".to_string(),
            updated_at: "2030-01-01T00:00:01Z".to_string(),
        });
        let recovery_error = prepare_group_cleanup_assignments(
            &context,
            &mut registry,
            &run,
            &controller,
            GroupCleanupMode::Force,
        )
        .unwrap_err();
        assert_eq!(recovery_error.code(), "submit-recovery-in-flight");
        assert_eq!(registry.assignments["assignment-working"].state, "working");
        assert_eq!(
            registry.assignments["assignment-submitted"].state,
            "submitted"
        );
        assert_eq!(
            registry.assignments["assignment-accepted"].state,
            "accepted"
        );
        registry
            .assignments
            .get_mut("assignment-working")
            .expect("working assignment")
            .submit_recovery
            .as_mut()
            .expect("submit recovery")
            .state = "failed".to_string();

        prepare_group_cleanup_assignments(
            &context,
            &mut registry,
            &run,
            &controller,
            GroupCleanupMode::Force,
        )
        .unwrap();
        assert_eq!(
            registry.assignments["assignment-working"].state,
            "cancelled"
        );
        assert_eq!(
            registry.assignments["assignment-submitted"].state,
            "cancelled"
        );
        assert_eq!(
            registry.assignments["assignment-accepted"].state,
            "released"
        );
        assert_eq!(
            registry.assignments["assignment-released"].state,
            "released"
        );
        assert_eq!(
            registry.assignments["assignment-cancelled"].state,
            "cancelled"
        );
    }

    #[test]
    fn group_cleanup_assignment_revision_overflow_fails_before_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        let mut registry = orchestration::Registry::default();
        let run = run_record("run-one", false);
        let controller = run.controller.clone();
        registry.runs.insert(run.run_id.clone(), run.clone());
        let mut assignment = dep_assignment("assignment-max", "run-one", "accepted");
        assignment.revision = u64::MAX;
        registry
            .assignments
            .insert(assignment.assignment_id.clone(), assignment);
        let before = serde_json::to_vec(&registry).unwrap();

        let error = prepare_group_cleanup_assignments(
            &context,
            &mut registry,
            &run,
            &controller,
            GroupCleanupMode::Safe,
        )
        .expect_err("an exhausted assignment revision must fail closed");
        assert_eq!(error.code(), "orchestration-revision-capacity");
        assert_eq!(
            serde_json::to_vec(&registry).unwrap(),
            before,
            "revision overflow must not partially terminalize assignments"
        );
    }

    #[test]
    fn group_cleanup_plan_enforces_the_bounded_checkpoint_batch() {
        let run = run_record("run-one", false);
        let controller = run.controller.clone();
        let registry_with = |count: usize| {
            let mut registry = orchestration::Registry::default();
            registry.runs.insert(run.run_id.clone(), run.clone());
            for index in 0..count {
                let id = format!("assignment-{index:03}");
                registry
                    .assignments
                    .insert(id.clone(), dep_assignment(&id, "run-one", "accepted"));
            }
            registry
        };

        let bounded = registry_with(GROUP_CLEANUP_MAX_ASSIGNMENTS);
        assert_eq!(
            build_group_cleanup_plan(&bounded, &run, &controller)
                .expect("maximum bounded cleanup plan")
                .workers
                .len(),
            GROUP_CLEANUP_MAX_ASSIGNMENTS
        );
        let oversized = registry_with(GROUP_CLEANUP_MAX_ASSIGNMENTS + 1);
        let error = build_group_cleanup_plan(&oversized, &run, &controller)
            .expect_err("oversized cleanup plan must fail before acquiring worker locks");
        assert_eq!(error.code(), "group-cleanup-batch-too-large");
    }

    fn cleanup_test_session(id: &str, incarnation: &str) -> SessionRecord {
        SessionRecord {
            schema_version: crate::SESSION_DOCUMENT_VERSION.to_string(),
            id: id.to_string(),
            agent: "codex".to_string(),
            mode: "interactive".to_string(),
            coordination_mode: CoordinationMode::Advisory,
            title: None,
            title_state: None,
            title_revision: 0,
            cwd: "/tmp".to_string(),
            tmux_session: format!("agent-{id}"),
            prompt_file: None,
            log_file: None,
            created_at: "2030-01-01T00:00:00Z".to_string(),
            updated_at: "2030-01-01T00:00:00Z".to_string(),
            provider_resume: None,
            runtime: Some(crate::RuntimeInfo {
                kind: "tmux".to_string(),
                tmux_session: format!("agent-{id}"),
                generation: 1,
                started_at: "2030-01-01T00:00:00Z".to_string(),
                launch_id: incarnation.to_string(),
                extra: std::collections::BTreeMap::new(),
            }),
            public_metadata: None,
            agent_args: Vec::new(),
            agent_bin: None,
            extra: std::collections::BTreeMap::new(),
            lineage: None,
            work: None,
            lineage_adoption: None,
            role: None,
            resume_sidecar_extra: std::collections::BTreeMap::new(),
        }
    }

    fn group_cleanup_progress_race_fixture(
        context: &CliContext,
        idempotency_key: &str,
    ) -> (PathBuf, PathBuf, Vec<u8>) {
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: idempotency_key.to_string(),
                request_digest: "6".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let source_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            context,
            &source_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let source_path = progress_dir.join(&source_key);
        fs::File::options()
            .write(true)
            .open(&source_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-race-main-{index}");
            let incarnation = format!("live-race-incarnation-{index}");
            crate::write_session_record(context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        (
            progress_dir,
            source_path,
            progress_bytes("incoming-main", "incoming-incarnation"),
        )
    }

    fn group_cleanup_progress_byte_pressure_fixture(
        context: &CliContext,
        idempotency_key: &str,
    ) -> (PathBuf, String, Vec<u8>) {
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, padding: usize| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: "byte-pressure-incarnation".to_string(),
                idempotency_key: idempotency_key.to_string(),
                request_digest: "d".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "padding": "x".repeat(padding)
                }),
            })
            .unwrap()
        };
        let current_key = "f".repeat(64);
        orchestration::store_group_cleanup_progress(
            context,
            &current_key,
            &progress_bytes("current-main", 0),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let mut source_path = None;
        for index in 0..4 {
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(
                &path,
                progress_bytes(&format!("abandoned-byte-main-{index}"), 3_200_000),
            )
            .unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            if index == 0 {
                fs::File::options()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_times(
                        fs::FileTimes::new()
                            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                    )
                    .unwrap();
                source_path = Some(path);
            }
        }
        (
            source_path.unwrap(),
            current_key,
            progress_bytes("current-main", 4_000_000),
        )
    }

    #[test]
    fn group_cleanup_worker_failure_preserves_the_main_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let main = cleanup_test_session("main", "inc");
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut worker_a = cleanup_test_session("worker-a", "worker-a-inc");
        worker_a.created_at = "2030-01-01T00:00:30Z".to_string();
        crate::mark_tmux_runtime_never_launched(&mut worker_a);
        fs::create_dir_all(session_dir(&context, &worker_a.id)).unwrap();
        crate::write_session_record(&context, &worker_a).unwrap();
        fs::create_dir_all(session_dir(&context, "worker-broken")).unwrap();

        let mut run = run_record("run-one", false);
        run.controller = session_ref(&context, &main, "inc");
        let mut assignment_a = dep_assignment("assignment-a", "run-one", "submitted");
        assignment_a.primary_manager = run.controller.clone();
        assignment_a.worker = Some(session_ref(&context, &worker_a, "worker-a-inc"));
        let mut assignment = dep_assignment("assignment-broken", "run-one", "submitted");
        assignment.primary_manager = run.controller.clone();
        assignment.worker = Some(SessionRef {
            machine: None,
            session_id: "worker-broken".to_string(),
            session_incarnation: "worker-inc".to_string(),
            session_created_at: "2030-01-01T00:01:00Z".to_string(),
        });
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked
                .registry
                .assignments
                .insert(assignment_a.assignment_id.clone(), assignment_a);
            locked
                .registry
                .assignments
                .insert(assignment.assignment_id.clone(), assignment);
            locked.save().unwrap();
        }

        let preview = preview_group_cleanup(&context, "main").unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "inc".to_string(),
            expected_run_revision: preview["run_revision"].as_u64().unwrap(),
            expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
            mode: GroupCleanupMode::Force,
            idempotency_key: "cleanup-001".to_string(),
        };
        let repaired_tmux = tmp.path().join("tmux-missing-session");
        fs::write(
            &repaired_tmux,
            "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&repaired_tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let execution =
            execute_group_cleanup(&context, "main", request.clone(), repaired_tmux.clone())
                .unwrap();

        assert_eq!(execution.value["completed"], false);
        assert_eq!(execution.value["main_deleted"], false);
        assert_eq!(execution.value["failure"]["stage"], "worker_cleanup");
        assert_eq!(
            execution.value["workers"][0]["assignment_id"],
            "assignment-a"
        );
        assert_eq!(execution.value["workers"][0]["outcome"], "deleted");
        assert_eq!(
            execution.value["workers"][1]["assignment_id"],
            "assignment-broken"
        );
        let first_fences = execution.deleted_registry_fences.clone();
        assert_eq!(first_fences.len(), 1);
        assert!(!session_dir(&context, "worker-a").exists());
        assert!(
            session_dir(&context, "main").join("session.json").exists(),
            "worker cleanup failure must preserve the Main Agent record"
        );

        let mut repaired_worker = cleanup_test_session("worker-broken", "worker-inc");
        repaired_worker.created_at = "2030-01-01T00:01:00Z".to_string();
        crate::mark_tmux_runtime_never_launched(&mut repaired_worker);
        crate::write_session_record(&context, &repaired_worker).unwrap();
        let mut repaired_main = load_session_record(&context, "main").unwrap();
        crate::mark_tmux_runtime_never_launched(&mut repaired_main);
        crate::write_session_record(&context, &repaired_main).unwrap();
        let resumed =
            execute_group_cleanup(&context, "main", request.clone(), repaired_tmux.clone())
                .unwrap();
        assert_eq!(
            resumed.value["completed"], true,
            "identical retry must resume from durable progress"
        );
        assert_eq!(resumed.value["run_closed"], true);
        assert_eq!(resumed.value["main_deleted"], true);
        assert_eq!(resumed.value["workers"][0]["outcome"], "deleted");
        assert_eq!(resumed.value["workers"][0]["session_id"], "worker-a");
        assert_eq!(resumed.value["workers"][1]["outcome"], "deleted");
        assert_eq!(
            &resumed.deleted_registry_fences[..first_fences.len()],
            first_fences.as_slice(),
            "retry must carry forward the exact first-worker registry fence"
        );
        assert_eq!(
            resumed.value["workers"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|worker| worker["session_id"] == "worker-a")
                .count(),
            1,
            "retry must skip the already deleted first worker"
        );
        assert!(!session_dir(&context, "worker-broken").exists());
        assert!(!session_dir(&context, "main").exists());
        assert_eq!(
            orchestration::load_registry_readonly(&context)
                .unwrap()
                .runs["run-one"]
                .state,
            "closed"
        );

        let replayed = execute_group_cleanup(&context, "main", request, repaired_tmux).unwrap();
        assert_eq!(replayed.value, resumed.value);
        assert_eq!(
            replayed.deleted_registry_fences, resumed.deleted_registry_fences,
            "successful replay must retain every daemon registry fence"
        );
    }

    #[test]
    fn group_cleanup_deletes_workers_closes_the_run_and_deletes_main_last() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let tmux = tmp.path().join("tmux-missing-session");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let mut main = cleanup_test_session("main", "inc");
        crate::mark_tmux_runtime_never_launched(&mut main);
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut worker = cleanup_test_session("worker", "worker-inc");
        crate::mark_tmux_runtime_never_launched(&mut worker);
        fs::create_dir_all(session_dir(&context, &worker.id)).unwrap();
        crate::write_session_record(&context, &worker).unwrap();

        let mut run = run_record("run-one", false);
        run.controller = session_ref(&context, &main, "inc");
        let mut assignment = dep_assignment("assignment", "run-one", "accepted");
        assignment.primary_manager = run.controller.clone();
        assignment.worker = Some(session_ref(&context, &worker, "worker-inc"));
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked
                .registry
                .assignments
                .insert(assignment.assignment_id.clone(), assignment);
            locked.save().unwrap();
        }

        let preview = preview_group_cleanup(&context, "main").unwrap();
        let execution = execute_group_cleanup(
            &context,
            "main",
            GroupCleanupRequest {
                schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
                expected_main_incarnation: "inc".to_string(),
                expected_run_revision: preview["run_revision"].as_u64().unwrap(),
                expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
                mode: GroupCleanupMode::Safe,
                idempotency_key: "cleanup-success-001".to_string(),
            },
            tmux,
        )
        .unwrap();

        assert_eq!(
            execution.value["completed"], true,
            "unexpected cleanup result"
        );
        assert_eq!(execution.value["run_closed"], true);
        assert_eq!(execution.value["main_deleted"], true);
        assert_eq!(execution.value["workers"][0]["outcome"], "deleted");
        assert!(!session_dir(&context, "worker").exists());
        assert!(!session_dir(&context, "main").exists());
        let registry = orchestration::load_registry_readonly(&context).unwrap();
        assert_eq!(registry.runs["run-one"].state, "closed");
        assert_eq!(
            crate::board::closed_reasons_for_test(&context),
            vec![
                ("worker".to_string(), "deleted".to_string()),
                ("main".to_string(), "deleted".to_string()),
            ]
        );
    }

    #[test]
    fn group_cleanup_exact_retry_adopts_every_durable_interruption_stage() {
        for (stage, worker_started) in [
            ("authority_fence", true),
            ("authority_sealed", true),
            ("worker_checkpoint", false),
            ("worker_delete_pending:assignment", true),
            ("worker_deleted_uncheckpointed:assignment", true),
            ("worker_deleted:assignment", true),
            ("run_closed", true),
            ("main_delete_pending", true),
            ("main_deleted_uncheckpointed", true),
            ("main_deleted", true),
        ] {
            let tmp = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            let tmux = tmp.path().join("tmux-missing-session");
            fs::write(
                &tmux,
                "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
            let mut main = cleanup_test_session("main", "inc");
            crate::mark_tmux_runtime_never_launched(&mut main);
            fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
            crate::write_session_record(&context, &main).unwrap();
            let mut run = run_record("run-one", false);
            run.controller = session_ref(&context, &main, "inc");
            let mut assignment = dep_assignment("assignment", "run-one", "accepted");
            assignment.primary_manager = run.controller.clone();
            if worker_started {
                let mut worker = cleanup_test_session("worker", "worker-inc");
                crate::mark_tmux_runtime_never_launched(&mut worker);
                fs::create_dir_all(session_dir(&context, &worker.id)).unwrap();
                crate::write_session_record(&context, &worker).unwrap();
                assignment.worker = Some(session_ref(&context, &worker, "worker-inc"));
            } else {
                assignment.worker = None;
            }
            {
                let mut locked = orchestration::lock_registry(&context).unwrap();
                locked.registry.runs.insert(run.run_id.clone(), run);
                locked
                    .registry
                    .assignments
                    .insert(assignment.assignment_id.clone(), assignment);
                locked.save().unwrap();
            }
            let preview = preview_group_cleanup(&context, "main").unwrap();
            let request = GroupCleanupRequest {
                schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
                expected_main_incarnation: "inc".to_string(),
                expected_run_revision: preview["run_revision"].as_u64().unwrap(),
                expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
                mode: GroupCleanupMode::Safe,
                idempotency_key: format!("cleanup-interrupt-{stage}"),
            };
            fs::write(
                context.state_dir.join("group-cleanup-interrupt-test"),
                stage,
            )
            .unwrap();
            let interrupted =
                match execute_group_cleanup(&context, "main", request.clone(), tmux.clone()) {
                    Ok(_) => panic!("expected stage interruption"),
                    Err(error) => error,
                };
            assert_eq!(interrupted.code(), "group-cleanup-test-interrupted");
            fs::remove_file(context.state_dir.join("group-cleanup-interrupt-test")).unwrap();
            if stage == "main_deleted_uncheckpointed" {
                let progress_key =
                    group_cleanup_progress_key("main", "inc", &request.idempotency_key);
                let progress_path = context
                    .state_dir
                    .join("orchestration/group-cleanup-progress")
                    .join(&progress_key);
                fs::File::options()
                    .write(true)
                    .open(&progress_path)
                    .unwrap()
                    .set_times(
                        fs::FileTimes::new()
                            .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                    )
                    .unwrap();
                for index in 0..128 {
                    let competing = GroupCleanupProgressReceipt {
                        schema_version: "agent-session.main-agent-group-cleanup-receipt.v1"
                            .to_string(),
                        requested_session_id: None,
                        principal_session_id: format!("abandoned-main-{index}"),
                        principal_incarnation: "abandoned-incarnation".to_string(),
                        idempotency_key: "retention-pressure".to_string(),
                        request_digest: "e".repeat(64),
                        outcome: json!({
                            "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                            "completed": false,
                            "workers": []
                        }),
                    };
                    orchestration::store_group_cleanup_progress(
                        &context,
                        &format!("{index:064x}"),
                        &serde_json::to_vec(&competing).unwrap(),
                    )
                    .unwrap();
                }
                assert!(
                    orchestration::read_group_cleanup_progress(&context, &progress_key)
                        .unwrap()
                        .is_some(),
                    "post-delete resume progress must survive retention pressure"
                );
            }
            let resumed =
                execute_group_cleanup(&context, "main", request.clone(), tmux.clone()).unwrap();
            assert_eq!(
                resumed.value["completed"], true,
                "cleanup retry must resume from durable progress"
            );
            let replayed = execute_group_cleanup(&context, "main", request, tmux).unwrap();
            assert_eq!(replayed.value, resumed.value, "stage={stage}");
            assert_eq!(
                replayed.deleted_registry_fences, resumed.deleted_registry_fences,
                "stage={stage}"
            );
        }
    }

    #[test]
    fn group_cleanup_safe_and_force_requests_have_one_execution_owner() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let safe_request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "incarnation".to_string(),
            expected_run_revision: 1,
            expected_plan_digest: format!("sha256:{}", "a".repeat(64)),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "safe-cleanup".to_string(),
        };
        let mut force_request = safe_request.clone();
        force_request.mode = GroupCleanupMode::Force;
        force_request.idempotency_key = "force-cleanup".to_string();
        assert_ne!(
            group_cleanup_request_digest(&safe_request),
            group_cleanup_request_digest(&force_request),
            "safe and force remain distinct idempotent requests"
        );
        let owner_digest = group_cleanup_execution_owner_digest("main");
        let first = lock_group_cleanup_execution(&context, &owner_digest).unwrap();
        let competing = lock_group_cleanup_execution(&context, &owner_digest).unwrap_err();
        assert_eq!(competing.code(), "group-cleanup-in-progress");
        drop(first);
        lock_group_cleanup_execution(&context, &owner_digest).unwrap();
    }

    #[test]
    fn group_cleanup_execution_lock_is_not_inherited_by_provider_children() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let owner_digest = group_cleanup_execution_owner_digest("main");
        let lock = lock_group_cleanup_execution(&context, &owner_digest).unwrap();
        let descriptor_flags = unsafe { libc::fcntl(lock._file.as_raw_fd(), libc::F_GETFD) };
        assert_ne!(descriptor_flags, -1);
        assert_ne!(
            descriptor_flags & libc::FD_CLOEXEC,
            0,
            "provider and tmux children must not inherit the cleanup execution flock"
        );
    }

    #[test]
    fn group_cleanup_execution_lock_rejects_a_symlinked_orchestration_root() {
        use std::os::unix::fs::symlink;

        let tmp = tempfile::TempDir::new().unwrap();
        let outside = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir_all(&context.state_dir).unwrap();
        symlink(outside.path(), context.state_dir.join("orchestration")).unwrap();

        let owner_digest = group_cleanup_execution_owner_digest("main");
        let error = lock_group_cleanup_execution(&context, &owner_digest).unwrap_err();
        assert_eq!(error.code(), "orchestration-store-invalid");
        assert!(
            !outside
                .path()
                .join(format!("group-cleanup-{owner_digest}.lock"))
                .exists(),
            "cleanup locking must not create files through an unsafe parent symlink"
        );
    }

    #[test]
    fn group_cleanup_session_aliases_share_lock_but_not_exact_receipt_selector() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let main = cleanup_test_session("main-controller-unique", "main-incarnation");
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: 1,
            expected_plan_digest: format!("sha256:{}", "a".repeat(64)),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "alias-cleanup".to_string(),
        };
        let owner_digest = group_cleanup_execution_owner_digest(&main.id);
        let lock = lock_group_cleanup_execution(&context, &owner_digest).unwrap();
        for alias in ["main-c", "main-controller"] {
            let error = execute_group_cleanup(
                &context,
                alias,
                request.clone(),
                PathBuf::from("/bin/false"),
            )
            .err()
            .expect("an exact cleanup lock must reject every session alias");
            assert_eq!(
                error.code(),
                "group-cleanup-in-progress",
                "{alias} must collide with the exact session cleanup lock"
            );
        }
        drop(lock);

        let request_digest = group_cleanup_request_digest(&request);
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            store_receipt_for_principal(
                &mut locked.registry,
                &main.id,
                "main-incarnation",
                &request.idempotency_key,
                "group-cleanup",
                &request_digest,
                json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": true,
                    "run_closed": true,
                    "main_deleted": true,
                    "workers": [],
                }),
            )
            .unwrap();
            locked.save().unwrap();
        }
        for alias in ["main-c", "main-controller"] {
            let error = match execute_group_cleanup(
                &context,
                alias,
                request.clone(),
                PathBuf::from("/bin/false"),
            ) {
                Ok(_) => panic!("a canonical-only receipt must not authorize an alias replay"),
                Err(error) => error,
            };
            assert_eq!(
                error.code(),
                "main-agent-run-not-found",
                "{alias} must not adopt the canonical receipt selector"
            );
        }
        let replay = execute_group_cleanup(
            &context,
            "main-controller-unique",
            request,
            PathBuf::from("/bin/false"),
        )
        .unwrap();
        assert_eq!(replay.value["completed"], true);
    }

    #[test]
    fn group_cleanup_alias_retry_adopts_legacy_incomplete_receipt_namespace() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut main = cleanup_test_session("main-controller-unique", "main-incarnation");
        crate::mark_tmux_runtime_never_launched(&mut main);
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut run = run_record("run-one", false);
        run.controller = session_ref(&context, &main, "main-incarnation");
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked.save().unwrap();
        }
        let preview = preview_group_cleanup(&context, "main-c").unwrap();
        let plan: GroupCleanupPlan = serde_json::from_value(preview.clone()).unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: preview["run_revision"].as_u64().unwrap(),
            expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "prior-alias-progress".to_string(),
        };
        let request_digest = group_cleanup_request_digest(&request);
        let legacy_results = vec![json!({
            "assignment_id": "prior-progress-canary",
            "outcome": "prior-alias-adopted"
        })];
        store_group_cleanup_receipt(
            &context,
            &GroupCleanupProgressIdentity {
                requested_session_id: "main-c",
                principal_session_id: "main-c",
                incarnation: "main-incarnation",
                operation: "group-cleanup",
            },
            &request,
            &request_digest,
            group_cleanup_progress_value(&plan, &legacy_results, false, "authority_sealed"),
            group_cleanup_resume_state(&plan, &legacy_results, &[], &[], false),
        )
        .unwrap();
        let legacy_progress_key =
            group_cleanup_progress_key("main-c", "main-incarnation", &request.idempotency_key);
        let legacy_progress_path = context
            .state_dir
            .join("orchestration/group-cleanup-progress")
            .join(&legacy_progress_key);
        let mut prior_progress: GroupCleanupProgressReceipt = serde_json::from_slice(
            &orchestration::read_group_cleanup_progress(&context, &legacy_progress_key)
                .unwrap()
                .expect("alias progress"),
        )
        .unwrap();
        prior_progress.schema_version =
            orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string();
        prior_progress.requested_session_id = None;
        orchestration::store_group_cleanup_progress(
            &context,
            &legacy_progress_key,
            &serde_json::to_vec(&prior_progress).unwrap(),
        )
        .unwrap();
        fs::File::options()
            .write(true)
            .open(&legacy_progress_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 0..128 {
            let competing = GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: format!("abandoned-alias-main-{index}"),
                principal_incarnation: "abandoned-incarnation".to_string(),
                idempotency_key: "prior-alias-retention-pressure".to_string(),
                request_digest: "f".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            };
            orchestration::store_group_cleanup_progress(
                &context,
                &format!("{index:064x}"),
                &serde_json::to_vec(&competing).unwrap(),
            )
            .unwrap();
        }
        assert!(
            orchestration::read_group_cleanup_progress(&context, &legacy_progress_key)
                .unwrap()
                .is_some(),
            "retention pressure must preserve live prior-version alias progress"
        );

        let tmux = tmp.path().join("tmux-missing-session");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        let execution =
            execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone()).unwrap();
        assert_eq!(
            execution.value["completed"], true,
            "prior-version replay must complete"
        );
        assert_eq!(
            execution.value["workers"][0]["assignment_id"], "prior-progress-canary",
            "the canonical retry must adopt the prior alias-keyed progress"
        );
        assert!(
            orchestration::read_group_cleanup_progress(&context, &legacy_progress_key)
                .unwrap()
                .is_none(),
            "canonical completion must remove the adopted prior-version alias sidecar"
        );
        let replayed = execute_group_cleanup(&context, "main-c", request, tmux).unwrap();
        assert_eq!(replayed.value, execution.value);
        assert_eq!(
            replayed.deleted_registry_fences,
            execution.deleted_registry_fences
        );
    }

    #[test]
    fn group_cleanup_alias_retry_recovers_after_main_deletion_and_rejects_alias_reuse() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut main = cleanup_test_session("main-controller-unique", "main-incarnation");
        crate::mark_tmux_runtime_never_launched(&mut main);
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut run = run_record("run-one", false);
        run.controller = session_ref(&context, &main, "main-incarnation");
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked.save().unwrap();
        }
        let preview = preview_group_cleanup(&context, "main-c").unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: preview["run_revision"].as_u64().unwrap(),
            expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "alias-post-delete-retry".to_string(),
        };
        let tmux = tmp.path().join("tmux-missing-session");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            context.state_dir.join("group-cleanup-interrupt-test"),
            "main_deleted_uncheckpointed",
        )
        .unwrap();
        let interrupted = execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone())
            .err()
            .expect("cleanup must stop after deleting the canonical Main Agent");
        assert_eq!(interrupted.code(), "group-cleanup-test-interrupted");
        assert!(!session_dir(&context, "main-controller-unique").exists());
        fs::remove_file(context.state_dir.join("group-cleanup-interrupt-test")).unwrap();
        let canonical_progress_key = group_cleanup_progress_key(
            "main-controller-unique",
            "main-incarnation",
            &request.idempotency_key,
        );
        let pending_progress: GroupCleanupProgressReceipt = serde_json::from_slice(
            &orchestration::read_group_cleanup_progress(&context, &canonical_progress_key)
                .unwrap()
                .expect("canonical pending progress"),
        )
        .unwrap();
        assert_eq!(
            pending_progress.schema_version,
            orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_SCHEMA
        );
        assert_eq!(
            pending_progress.requested_session_id.as_deref(),
            Some("main-c"),
            "new progress must retain the exact original selector"
        );
        assert_eq!(
            pending_progress.principal_session_id,
            "main-controller-unique"
        );

        let resumed =
            execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone()).unwrap();
        assert_eq!(resumed.value["completed"], true);
        assert!(
            resumed
                .deleted_registry_fences
                .iter()
                .any(|fence| fence.session_id == "main-controller-unique"),
            "alias retry must recover the deleted canonical session fence"
        );
        orchestration::store_group_cleanup_progress(
            &context,
            &canonical_progress_key,
            br#"{"ambiguous_terminal_cleanup":true}"#,
        )
        .unwrap();
        let replayed =
            execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone()).unwrap();
        assert_eq!(replayed.value, resumed.value);
        assert_eq!(
            replayed.deleted_registry_fences,
            resumed.deleted_registry_fences
        );
        assert!(
            orchestration::read_group_cleanup_progress(&context, &canonical_progress_key)
                .unwrap()
                .is_none(),
            "completed replay must reconcile a canonical sidecar left by ambiguous cleanup"
        );

        let mut replacement = cleanup_test_session("main-c-replacement", "replacement-incarnation");
        crate::mark_tmux_runtime_never_launched(&mut replacement);
        fs::create_dir_all(session_dir(&context, &replacement.id)).unwrap();
        crate::write_session_record(&context, &replacement).unwrap();
        let before_registry =
            fs::read(context.state_dir.join("orchestration/registry.json")).unwrap();
        let conflict = execute_group_cleanup(&context, "main-c", request, tmux)
            .err()
            .expect("a reused alias must not adopt the prior canonical cleanup");
        assert_eq!(conflict.code(), "main-session-incarnation-conflict");
        assert_eq!(
            fs::read(context.state_dir.join("orchestration/registry.json")).unwrap(),
            before_registry,
            "alias reuse must not mutate either cleanup namespace"
        );
    }

    #[test]
    fn group_cleanup_legacy_alias_retry_recovers_after_main_deletion_and_reconciles_receipt() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut main = cleanup_test_session("main-controller-unique", "main-incarnation");
        crate::mark_tmux_runtime_never_launched(&mut main);
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut run = run_record("run-one", false);
        run.controller = session_ref(&context, &main, "main-incarnation");
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked.save().unwrap();
        }
        let preview = preview_group_cleanup(&context, "main-c").unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: preview["run_revision"].as_u64().unwrap(),
            expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "prior-alias-post-delete".to_string(),
        };
        let tmux = tmp.path().join("tmux-missing-session");
        fs::write(
            &tmux,
            "#!/bin/sh\nprintf \"%s\\n\" \"can't find session: test\" >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(
            context.state_dir.join("group-cleanup-interrupt-test"),
            "main_deleted_uncheckpointed",
        )
        .unwrap();
        let interrupted = execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone())
            .err()
            .expect("cleanup must stop after deleting the canonical Main Agent");
        assert_eq!(interrupted.code(), "group-cleanup-test-interrupted");
        fs::remove_file(context.state_dir.join("group-cleanup-interrupt-test")).unwrap();

        let canonical_progress_key = group_cleanup_progress_key(
            "main-controller-unique",
            "main-incarnation",
            &request.idempotency_key,
        );
        let legacy_progress_key =
            group_cleanup_progress_key("main-c", "main-incarnation", &request.idempotency_key);
        let mut progress: GroupCleanupProgressReceipt = serde_json::from_slice(
            &orchestration::read_group_cleanup_progress(&context, &canonical_progress_key)
                .unwrap()
                .expect("canonical pending progress"),
        )
        .unwrap();
        progress.schema_version =
            orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string();
        progress.requested_session_id = None;
        progress.principal_session_id = "main-c".to_string();
        orchestration::store_group_cleanup_progress(
            &context,
            &legacy_progress_key,
            &serde_json::to_vec(&progress).unwrap(),
        )
        .unwrap();
        orchestration::remove_group_cleanup_progress(&context, &canonical_progress_key).unwrap();
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            let canonical_receipt_key = receipt_key(
                "main-controller-unique",
                "main-incarnation",
                &request.idempotency_key,
            );
            let legacy_receipt_key =
                receipt_key("main-c", "main-incarnation", &request.idempotency_key);
            let mut receipt = locked
                .registry
                .receipts
                .remove(&canonical_receipt_key)
                .expect("canonical run-closed receipt");
            receipt.principal_session_id = "main-c".to_string();
            locked.registry.receipts.insert(legacy_receipt_key, receipt);
            locked.save().unwrap();
        }

        let resumed =
            execute_group_cleanup(&context, "main-c", request.clone(), tmux.clone()).unwrap();
        assert_eq!(resumed.value["completed"], true);
        assert!(
            orchestration::read_group_cleanup_progress(&context, &legacy_progress_key)
                .unwrap()
                .is_none(),
            "prior-version sidecar must be removed after canonical completion"
        );
        let registry = orchestration::load_registry_readonly(&context).unwrap();
        let legacy_receipt = &registry.receipts
            [&receipt_key("main-c", "main-incarnation", &request.idempotency_key)];
        assert_eq!(
            legacy_receipt.outcome["completed"], true,
            "prior-version registry readers must observe the reconciled terminal outcome"
        );
        let replayed = execute_group_cleanup(&context, "main-c", request, tmux).unwrap();
        assert_eq!(replayed.value, resumed.value);
        assert_eq!(
            replayed.deleted_registry_fences,
            resumed.deleted_registry_fences
        );
    }

    #[test]
    fn group_cleanup_progress_does_not_rewrite_near_limit_registry_per_worker() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.receipts.insert(
                "near-limit-filler".to_string(),
                orchestration::IdempotencyReceipt {
                    principal_session_id: "filler".to_string(),
                    principal_incarnation: "filler-incarnation".to_string(),
                    operation: "filler".to_string(),
                    request_digest: "f".repeat(64),
                    outcome: json!({ "padding": "x".repeat(3 * 1024 * 1024) }),
                    created_at_epoch: 0,
                },
            );
            locked.save().unwrap();
        }
        orchestration::reset_registry_save_bytes_for_test();
        let main = SessionRef {
            machine: None,
            session_id: "main-controller".to_string(),
            session_incarnation: "main-incarnation".to_string(),
            session_created_at: "2030-01-01T00:00:00Z".to_string(),
        };
        let plan = GroupCleanupPlan {
            schema_version: GROUP_CLEANUP_SCHEMA.to_string(),
            main,
            run_id: "run-one".to_string(),
            run_revision: 1,
            requires_force: false,
            workers: Vec::new(),
            plan_digest: format!("sha256:{}", "a".repeat(64)),
        };
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: 1,
            expected_plan_digest: plan.plan_digest.clone(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "bounded-progress".to_string(),
        };
        let request_digest = group_cleanup_request_digest(&request);
        let mut worker_results = Vec::new();
        for index in 0..64 {
            worker_results.push(json!({
                "assignment_id": format!("worker-{index:02}"),
                "outcome": "deleted",
            }));
            let value =
                group_cleanup_progress_value(&plan, &worker_results, false, "worker_deleted");
            store_group_cleanup_receipt(
                &context,
                &GroupCleanupProgressIdentity {
                    requested_session_id: "main-controller",
                    principal_session_id: "main-controller",
                    incarnation: "main-incarnation",
                    operation: "group-cleanup",
                },
                &request,
                &request_digest,
                value,
                group_cleanup_resume_state(&plan, &worker_results, &[], &[], false),
            )
            .unwrap();
        }
        assert!(
            orchestration::registry_save_bytes_for_test() <= 8 * 1024 * 1024,
            "worker progress must not serialize a near-limit registry once per checkpoint"
        );
    }

    #[test]
    fn group_cleanup_progress_retention_bounds_abandoned_records_without_evicting_active() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let active_main = cleanup_test_session("active-main", "active-incarnation");
        crate::write_session_record(&context, &active_main).unwrap();

        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "bounded-retention".to_string(),
                request_digest: "a".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let active_key = "a".repeat(64);
        orchestration::store_group_cleanup_progress(
            &context,
            &active_key,
            &progress_bytes("active-main", "active-incarnation"),
        )
        .unwrap();
        for index in 0..160 {
            orchestration::store_group_cleanup_progress(
                &context,
                &format!("{index:064x}"),
                &progress_bytes(&format!("abandoned-main-{index}"), "stale-incarnation"),
            )
            .unwrap();
        }

        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let retained = fs::read_dir(&progress_dir).unwrap().count();
        assert!(
            retained <= 128,
            "abandoned progress must stay under the aggregate file-count bound"
        );
        assert!(
            orchestration::read_group_cleanup_progress(&context, &active_key)
                .unwrap()
                .is_some(),
            "an exact live principal's resumable progress must be retained"
        );
    }

    #[test]
    fn group_cleanup_progress_retention_reads_bodies_only_under_pressure_and_never_evicts_live() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-read-bound".to_string(),
                request_digest: "b".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };

        for index in 0..10 {
            orchestration::store_group_cleanup_progress(
                &context,
                &format!("{index:064x}"),
                &progress_bytes(&format!("missing-{index}"), "missing-incarnation"),
            )
            .unwrap();
        }
        orchestration::reset_group_cleanup_progress_body_reads_for_test();
        orchestration::store_group_cleanup_progress(
            &context,
            &format!("{:064x}", 10),
            &progress_bytes("missing-10", "missing-incarnation"),
        )
        .unwrap();
        assert_eq!(
            orchestration::group_cleanup_progress_body_reads_for_test(),
            0,
            "an in-capacity checkpoint must scan metadata without reading progress bodies"
        );

        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        for index in 10..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            let record = cleanup_test_session(&session_id, &incarnation);
            crate::write_session_record(&context, &record).unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new()
                        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                )
                .unwrap();
        }
        for index in 0..10 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            let record = cleanup_test_session(&session_id, &incarnation);
            crate::write_session_record(&context, &record).unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new()
                        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                )
                .unwrap();
        }

        let error = orchestration::store_group_cleanup_progress(
            &context,
            &"f".repeat(64),
            &progress_bytes("new-main", "new-incarnation"),
        )
        .expect_err("all-live capacity must fail closed");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
        assert!(
            orchestration::read_group_cleanup_progress(&context, &format!("{:064x}", 0))
                .unwrap()
                .is_some(),
            "old progress for an exact live incarnation must never be evicted"
        );
    }

    #[test]
    fn group_cleanup_progress_removal_serializes_with_capacity_admission() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-removal-race".to_string(),
                request_digest: "c".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let abandoned_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &abandoned_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        fs::File::options()
            .write(true)
            .open(progress_dir.join(&abandoned_key))
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let (scanned, resume) =
            orchestration::install_group_cleanup_progress_scan_hook_for_test(&progress_dir);
        let writer_context = context.clone();
        let incoming = progress_bytes("new-main", "new-incarnation");
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        scanned.wait();

        let remover_context = context.clone();
        let remover_key = abandoned_key.clone();
        let (removed_tx, removed_rx) = std::sync::mpsc::channel();
        let remover = thread::spawn(move || {
            let result =
                orchestration::remove_group_cleanup_progress(&remover_context, &remover_key);
            removed_tx.send(()).unwrap();
            result
        });
        let removal_completed_while_store_locked =
            removed_rx.recv_timeout(Duration::from_millis(50)).is_ok();
        resume.wait();

        let writer_result = writer.join().unwrap();
        let remover_result = remover.join().unwrap();
        assert!(
            !removal_completed_while_store_locked,
            "removal must wait for the aggregate capacity snapshot and admission"
        );
        writer_result.expect("capacity admission must use a stable projection");
        remover_result.expect("idempotent removal must succeed after admission");
        assert!(
            fs::read_dir(&progress_dir).unwrap().count() <= 128,
            "the serialized race must preserve the aggregate file-count bound"
        );
    }

    #[test]
    fn completed_group_cleanup_receipt_does_not_hold_registry_while_waiting_for_progress_lock() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let main = SessionRef {
            machine: None,
            session_id: "main-controller".to_string(),
            session_incarnation: "main-incarnation".to_string(),
            session_created_at: "2030-01-01T00:00:00Z".to_string(),
        };
        let plan = GroupCleanupPlan {
            schema_version: GROUP_CLEANUP_SCHEMA.to_string(),
            main,
            run_id: "run-one".to_string(),
            run_revision: 1,
            requires_force: false,
            workers: Vec::new(),
            plan_digest: format!("sha256:{}", "a".repeat(64)),
        };
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: 1,
            expected_plan_digest: plan.plan_digest.clone(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "completed-lock-order".to_string(),
        };
        let request_digest = group_cleanup_request_digest(&request);
        store_group_cleanup_receipt(
            &context,
            &GroupCleanupProgressIdentity {
                requested_session_id: "main-controller",
                principal_session_id: "main-controller",
                incarnation: "main-incarnation",
                operation: "group-cleanup",
            },
            &request,
            &request_digest,
            group_cleanup_progress_value(&plan, &[], false, "worker_checkpoint"),
            group_cleanup_resume_state(&plan, &[], &[], &[], false),
        )
        .unwrap();

        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let (scanned, resume) =
            orchestration::install_group_cleanup_progress_scan_hook_for_test(&progress_dir);
        let holder_context = context.clone();
        let holder = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(
                &holder_context,
                &"f".repeat(64),
                br#"{"holder":true}"#,
            )
        });
        scanned.wait();

        let finalizer_context = context.clone();
        let finalizer_request = request.clone();
        let finalizer_digest = request_digest.clone();
        let finalizer_plan = plan.clone();
        let finalizer = thread::spawn(move || {
            store_group_cleanup_receipt(
                &finalizer_context,
                &GroupCleanupProgressIdentity {
                    requested_session_id: "main-controller",
                    principal_session_id: "main-controller",
                    incarnation: "main-incarnation",
                    operation: "group-cleanup",
                },
                &finalizer_request,
                &finalizer_digest,
                json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": true,
                    "run_closed": true,
                    "main_deleted": true,
                    "workers": []
                }),
                group_cleanup_resume_state(&finalizer_plan, &[], &[], &[], true),
            )
        });
        let receipt_key = receipt_key(
            "main-controller",
            "main-incarnation",
            &request.idempotency_key,
        );
        let receipt_deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let saved = orchestration::load_registry_readonly(&context)
                .ok()
                .is_some_and(|registry| registry.receipts.contains_key(&receipt_key));
            if saved {
                break;
            }
            assert!(
                Instant::now() < receipt_deadline,
                "completed receipt was not saved before the test deadline"
            );
            thread::sleep(Duration::from_millis(5));
        }

        let probe_context = context.clone();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let probe = thread::spawn(move || {
            let locked = orchestration::lock_registry(&probe_context);
            acquired_tx.send(locked.is_ok()).unwrap();
            locked.map(drop)
        });
        let registry_acquired_while_progress_locked = acquired_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok_and(|value| value);
        resume.wait();

        holder.join().unwrap().unwrap();
        finalizer.join().unwrap().unwrap();
        probe.join().unwrap().unwrap();
        assert!(
            registry_acquired_while_progress_locked,
            "completed finalization must release the global registry lock before progress removal waits"
        );
    }

    #[test]
    fn completed_group_cleanup_preserves_alias_and_canonical_receipts_at_capacity() {
        let _fixture_ownership = GlobalStateLock::new();
        IDEMPOTENCY_RECEIPT_CAPACITY_FOR_TEST.store(4, Ordering::Release);
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let principal = "main-controller";
        let alias = "main-c";
        let incarnation = "main-incarnation";
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: incarnation.to_string(),
            expected_run_revision: 1,
            expected_plan_digest: format!("sha256:{}", "a".repeat(64)),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "dual-receipt-capacity".to_string(),
        };
        let request_digest = group_cleanup_request_digest(&request);
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            store_receipt_for_principal(
                &mut locked.registry,
                principal,
                incarnation,
                &request.idempotency_key,
                "group-cleanup",
                &request_digest,
                json!({ "completed": false }),
            )
            .unwrap();
            for index in 0..3 {
                let filler_principal = format!("filler-{index}");
                store_receipt_for_principal(
                    &mut locked.registry,
                    &filler_principal,
                    "filler-incarnation",
                    &format!("filler-capacity-{index}"),
                    "filler",
                    &"b".repeat(64),
                    json!({ "completed": true }),
                )
                .unwrap();
            }
            for receipt in locked.registry.receipts.values_mut() {
                receipt.created_at_epoch = i64::MAX;
            }
            let oldest_filler = receipt_key("filler-0", "filler-incarnation", "filler-capacity-0");
            locked
                .registry
                .receipts
                .get_mut(&oldest_filler)
                .unwrap()
                .created_at_epoch = i64::MIN;
            locked.save().unwrap();
        }
        let plan = GroupCleanupPlan {
            schema_version: GROUP_CLEANUP_SCHEMA.to_string(),
            main: SessionRef {
                machine: None,
                session_id: principal.to_string(),
                session_incarnation: incarnation.to_string(),
                session_created_at: "2030-01-01T00:00:00Z".to_string(),
            },
            run_id: "run-one".to_string(),
            run_revision: 1,
            requires_force: false,
            workers: Vec::new(),
            plan_digest: request.expected_plan_digest.clone(),
        };
        let completed = json!({
            "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
            "completed": true,
            "run_closed": true,
            "main_deleted": true,
            "workers": []
        });
        let outcome = group_cleanup_stored_outcome(
            &completed,
            &group_cleanup_resume_state(&plan, &[], &[], &[], true),
        )
        .unwrap();
        let stored = store_completed_group_cleanup_receipt(
            &context,
            principal,
            Some(alias),
            incarnation,
            &request,
            &request_digest,
            "group-cleanup",
            outcome,
        );
        IDEMPOTENCY_RECEIPT_CAPACITY_FOR_TEST.store(MAX_IDEMPOTENCY_RECEIPTS, Ordering::Release);
        stored.unwrap();

        let registry = orchestration::load_registry_readonly(&context).unwrap();
        for receipt_principal in [principal, alias] {
            let key = receipt_key(receipt_principal, incarnation, &request.idempotency_key);
            assert_eq!(
                registry.receipts[&key].outcome["completed"], true,
                "{receipt_principal} terminal receipt must survive capacity admission"
            );
        }
        assert_eq!(registry.receipts.len(), 4);
    }

    #[test]
    fn group_cleanup_progress_retention_protects_unverifiable_entries() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-unverifiable".to_string(),
                request_digest: "d".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let oldest_key = format!("{:064x}", 0);
        crate::write_session_record(
            &context,
            &cleanup_test_session("live-main-0", "live-incarnation-0"),
        )
        .unwrap();
        orchestration::store_group_cleanup_progress(
            &context,
            &oldest_key,
            &progress_bytes("live-main-0", "live-incarnation-0"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let oldest_path = progress_dir.join(&oldest_key);
        fs::File::options()
            .write(true)
            .open(&oldest_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let (scanned, resume) =
            orchestration::install_group_cleanup_progress_scan_hook_for_test(&progress_dir);
        let writer_context = context.clone();
        let incoming = progress_bytes("new-main", "new-incarnation");
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        scanned.wait();
        fs::write(&oldest_path, b"{").unwrap();
        fs::set_permissions(&oldest_path, fs::Permissions::from_mode(0o600)).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("unverifiable progress must fail capacity admission closed");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert!(
            oldest_path.exists(),
            "unverifiable exact-live progress must remain protected"
        );
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
    }

    #[test]
    fn group_cleanup_progress_retention_does_not_unlink_a_replacement_after_identity_check() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: "agent-session.main-agent-group-cleanup-receipt.v1".to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-replacement-race".to_string(),
                request_digest: "e".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let abandoned_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &abandoned_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let abandoned_path = progress_dir.join(&abandoned_key);
        fs::File::options()
            .write(true)
            .open(&abandoned_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let (classified, resume) =
            orchestration::install_group_cleanup_progress_eviction_hook_for_test(&abandoned_path);
        let writer_context = context.clone();
        let incoming = progress_bytes("new-main", "new-incarnation");
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        classified.wait();

        let replacement_session_id = "replacement-live-main";
        let replacement_incarnation = "replacement-live-incarnation";
        crate::write_session_record(
            &context,
            &cleanup_test_session(replacement_session_id, replacement_incarnation),
        )
        .unwrap();
        fs::remove_file(&abandoned_path).unwrap();
        let replacement_bytes = progress_bytes(replacement_session_id, replacement_incarnation);
        fs::write(&abandoned_path, &replacement_bytes).unwrap();
        fs::set_permissions(&abandoned_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&abandoned_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("a replacement must not be admitted by deleting the new path identity");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(
            fs::read(&abandoned_path).unwrap(),
            replacement_bytes,
            "eviction must remain bound to the stale descriptor snapshot"
        );
        let replacement_after = fs::symlink_metadata(&abandoned_path).unwrap();
        assert_eq!(
            (replacement_after.dev(), replacement_after.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino()),
            "the exact raced replacement inode must remain at its original key"
        );
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
        assert!(
            orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context),
            "a restored replacement must leave the bounded recycle slot retired"
        );
    }

    #[test]
    fn group_cleanup_progress_reconcile_preserves_replacement_after_exchange() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (progress_dir, source_path, incoming) =
            group_cleanup_progress_race_fixture(&context, "post-exchange-replacement");
        let (swapped, resume, _) =
            orchestration::install_group_cleanup_progress_recycle_hook_for_test(&context);
        let writer_context = context.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        swapped.wait();

        let replacement_session_id = "post-exchange-live-main";
        let replacement_incarnation = "post-exchange-live-incarnation";
        crate::write_session_record(
            &context,
            &cleanup_test_session(replacement_session_id, replacement_incarnation),
        )
        .unwrap();
        let mut replacement =
            serde_json::from_slice::<Value>(&fs::read(&source_path).unwrap()).unwrap();
        replacement["principal_session_id"] = json!(replacement_session_id);
        replacement["principal_incarnation"] = json!(replacement_incarnation);
        let replacement_bytes = serde_json::to_vec(&replacement).unwrap();
        fs::remove_file(&source_path).unwrap();
        fs::write(&source_path, &replacement_bytes).unwrap();
        fs::set_permissions(&source_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&source_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("post-exchange replacement must abort admission without displacement");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(fs::read(&source_path).unwrap(), replacement_bytes);
        let replacement_after = fs::symlink_metadata(&source_path).unwrap();
        assert_eq!(
            (replacement_after.dev(), replacement_after.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino())
        );
        assert!(!progress_dir.join("f".repeat(64)).exists());
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_post_verify_race_restores_source_replacement() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (progress_dir, source_path, incoming) =
            group_cleanup_progress_race_fixture(&context, "post-verify-replacement");
        let current_path = progress_dir.join("f".repeat(64));
        let (ready, resume) =
            orchestration::install_group_cleanup_progress_post_verify_hook_for_test(&source_path);
        let writer_context = context.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        ready.wait();

        let replacement_session_id = "post-verify-live-main";
        let replacement_incarnation = "post-verify-live-incarnation";
        crate::write_session_record(
            &context,
            &cleanup_test_session(replacement_session_id, replacement_incarnation),
        )
        .unwrap();
        let mut replacement =
            serde_json::from_slice::<Value>(&fs::read(&source_path).unwrap()).unwrap();
        replacement["principal_session_id"] = json!(replacement_session_id);
        replacement["principal_incarnation"] = json!(replacement_incarnation);
        let replacement_bytes = serde_json::to_vec(&replacement).unwrap();
        fs::remove_file(&source_path).unwrap();
        fs::write(&source_path, &replacement_bytes).unwrap();
        fs::set_permissions(&source_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&source_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("a source replacement must not be moved to the incoming key");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(fs::read(&source_path).unwrap(), replacement_bytes);
        let replacement_after = fs::symlink_metadata(&source_path).unwrap();
        assert_eq!(
            (replacement_after.dev(), replacement_after.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino()),
            "compensation must restore the exact post-verify replacement inode to its source key"
        );
        assert!(
            !current_path.exists(),
            "the replacement must not be installed at the incoming key"
        );
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_final_rename_preserves_replacement_before_verification() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (progress_dir, source_path, incoming) =
            group_cleanup_progress_race_fixture(&context, "post-final-rename-replacement");
        let current_path = progress_dir.join("f".repeat(64));
        let (renamed, resume) =
            orchestration::install_group_cleanup_progress_final_rename_hook_for_test(&current_path);
        let writer_context = context.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        renamed.wait();

        let replacement_session_id = "post-rename-live-main";
        let replacement_incarnation = "post-rename-live-incarnation";
        crate::write_session_record(
            &context,
            &cleanup_test_session(replacement_session_id, replacement_incarnation),
        )
        .unwrap();
        let mut replacement =
            serde_json::from_slice::<Value>(&fs::read(&current_path).unwrap()).unwrap();
        replacement["principal_session_id"] = json!(replacement_session_id);
        replacement["principal_incarnation"] = json!(replacement_incarnation);
        let replacement_bytes = serde_json::to_vec(&replacement).unwrap();
        fs::remove_file(&current_path).unwrap();
        fs::write(&current_path, &replacement_bytes).unwrap();
        fs::set_permissions(&current_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&current_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("post-rename replacement must abort admission without displacement");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert!(
            !current_path.exists(),
            "an unproven destination identity must be compensated back to the source key"
        );
        assert_eq!(fs::read(&source_path).unwrap(), replacement_bytes);
        let replacement_after = fs::symlink_metadata(&source_path).unwrap();
        assert_eq!(
            (replacement_after.dev(), replacement_after.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino())
        );
        assert!(
            orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context),
            "successful exact compensation must retire the transaction"
        );
    }

    #[test]
    fn group_cleanup_progress_recovery_uses_durable_final_install_proof() {
        let _fixture_ownership = GlobalStateLock::new();
        for destination_state in ["replaced", "removed"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            let (progress_dir, _, incoming) = group_cleanup_progress_race_fixture(
                &context,
                &format!("post-install-{destination_state}"),
            );
            let current_path = progress_dir.join("f".repeat(64));
            let (installed, resume) =
                orchestration::install_group_cleanup_progress_post_install_hook_for_test(
                    &current_path,
                );
            orchestration::fail_group_cleanup_progress_after_final_install_for_test(&current_path);
            let writer_context = context.clone();
            let writer = thread::spawn(move || {
                orchestration::store_group_cleanup_progress(
                    &writer_context,
                    &"f".repeat(64),
                    &incoming,
                )
            });
            installed.wait();

            let replacement = if destination_state == "replaced" {
                let replacement_session_id = "post-install-live-main";
                let replacement_incarnation = "post-install-live-incarnation";
                crate::write_session_record(
                    &context,
                    &cleanup_test_session(replacement_session_id, replacement_incarnation),
                )
                .unwrap();
                let mut replacement =
                    serde_json::from_slice::<Value>(&fs::read(&current_path).unwrap()).unwrap();
                replacement["principal_session_id"] = json!(replacement_session_id);
                replacement["principal_incarnation"] = json!(replacement_incarnation);
                let bytes = serde_json::to_vec(&replacement).unwrap();
                fs::remove_file(&current_path).unwrap();
                fs::write(&current_path, &bytes).unwrap();
                fs::set_permissions(&current_path, fs::Permissions::from_mode(0o600)).unwrap();
                let metadata = fs::symlink_metadata(&current_path).unwrap();
                Some((bytes, metadata.dev(), metadata.ino()))
            } else {
                fs::remove_file(&current_path).unwrap();
                None
            };
            resume.wait();

            let error = writer
                .join()
                .unwrap()
                .expect_err("the post-install crash must leave durable recovery work");
            assert_eq!(error.code(), "orchestration-store-unavailable");
            assert_eq!(
                orchestration::recover_group_cleanup_progress_principal(
                    &context,
                    "unrelated-selector",
                    "unrelated-incarnation",
                    "unrelated-idempotency",
                    &"a".repeat(64),
                )
                .unwrap(),
                None,
                "durable installed-phase evidence must let recovery converge"
            );
            if let Some((bytes, device, inode)) = replacement {
                assert_eq!(fs::read(&current_path).unwrap(), bytes);
                let metadata = fs::symlink_metadata(&current_path).unwrap();
                assert_eq!(
                    (metadata.dev(), metadata.ino()),
                    (device, inode),
                    "post-install recovery must retain the exact destination replacement"
                );
            } else {
                assert!(
                    !current_path.exists(),
                    "post-install recovery must respect destination removal"
                );
            }
            assert!(
                orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context)
            );
        }
    }

    #[test]
    fn group_cleanup_progress_recovery_compensates_unproven_final_rename() {
        let _fixture_ownership = GlobalStateLock::new();
        for destination_state in ["replaced", "removed"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            let (progress_dir, source_path, incoming) = group_cleanup_progress_race_fixture(
                &context,
                &format!("unproven-final-{destination_state}"),
            );
            let current_path = progress_dir.join("f".repeat(64));
            let (renamed, resume) =
                orchestration::install_group_cleanup_progress_final_rename_hook_for_test(
                    &current_path,
                );
            orchestration::fail_group_cleanup_progress_after_final_rename_for_test(&current_path);
            let writer_context = context.clone();
            let writer = thread::spawn(move || {
                orchestration::store_group_cleanup_progress(
                    &writer_context,
                    &"f".repeat(64),
                    &incoming,
                )
            });
            renamed.wait();

            let replacement = if destination_state == "replaced" {
                let replacement_session_id = "unproven-final-live-main";
                let replacement_incarnation = "unproven-final-live-incarnation";
                crate::write_session_record(
                    &context,
                    &cleanup_test_session(replacement_session_id, replacement_incarnation),
                )
                .unwrap();
                let mut replacement =
                    serde_json::from_slice::<Value>(&fs::read(&current_path).unwrap()).unwrap();
                replacement["principal_session_id"] = json!(replacement_session_id);
                replacement["principal_incarnation"] = json!(replacement_incarnation);
                let bytes = serde_json::to_vec(&replacement).unwrap();
                fs::remove_file(&current_path).unwrap();
                fs::write(&current_path, &bytes).unwrap();
                fs::set_permissions(&current_path, fs::Permissions::from_mode(0o600)).unwrap();
                let metadata = fs::symlink_metadata(&current_path).unwrap();
                Some((bytes, metadata.dev(), metadata.ino()))
            } else {
                fs::remove_file(&current_path).unwrap();
                None
            };
            resume.wait();

            let error = writer
                .join()
                .unwrap()
                .expect_err("the pre-proof crash must leave a prepared-phase transaction");
            assert_eq!(error.code(), "orchestration-store-unavailable");
            assert_eq!(
                orchestration::recover_group_cleanup_progress_principal(
                    &context,
                    "unrelated-selector",
                    "unrelated-incarnation",
                    "unrelated-idempotency",
                    &"b".repeat(64),
                )
                .unwrap(),
                None
            );
            assert!(
                !current_path.exists(),
                "prepared-phase recovery must not admit an unproven destination"
            );
            if let Some((bytes, device, inode)) = replacement {
                assert_eq!(fs::read(&source_path).unwrap(), bytes);
                let metadata = fs::symlink_metadata(&source_path).unwrap();
                assert_eq!(
                    (metadata.dev(), metadata.ino()),
                    (device, inode),
                    "prepared-phase recovery must restore the exact displaced inode"
                );
            } else {
                assert!(
                    !source_path.exists(),
                    "destination removal must converge without manufacturing a source"
                );
            }
            assert!(
                orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context)
            );
        }
    }

    #[test]
    fn group_cleanup_progress_recycle_recovers_a_post_swap_replacement_after_crash() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-post-swap-race".to_string(),
                request_digest: "1".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let abandoned_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &abandoned_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let abandoned_path = progress_dir.join(&abandoned_key);
        fs::File::options()
            .write(true)
            .open(&abandoned_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let (swapped, resume, recycle_path) =
            orchestration::install_group_cleanup_progress_recycle_hook_for_test(&context);
        orchestration::fail_group_cleanup_progress_after_recycle_for_test(&context);
        let writer_context = context.clone();
        let incoming = progress_bytes("new-main", "new-incarnation");
        let retry_incoming = incoming.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &"f".repeat(64), &incoming)
        });
        swapped.wait();
        resume.wait();

        let crash_error = writer
            .join()
            .unwrap()
            .expect_err("the injected post-exchange crash must preserve its active journal");
        assert_eq!(crash_error.code(), "orchestration-store-unavailable");
        let (recovery_ready, recovery_resume) =
            orchestration::install_group_cleanup_progress_recovery_exchange_hook_for_test(
                &abandoned_path,
            );
        let retry_context = context.clone();
        let retry = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(
                &retry_context,
                &"f".repeat(64),
                &retry_incoming,
            )
        });
        recovery_ready.wait();
        let replacement_session_id = "replacement-live-main";
        let replacement_incarnation = "replacement-live-incarnation";
        crate::write_session_record(
            &context,
            &cleanup_test_session(replacement_session_id, replacement_incarnation),
        )
        .unwrap();
        fs::remove_file(&abandoned_path).unwrap();
        let replacement_bytes = progress_bytes(replacement_session_id, replacement_incarnation);
        fs::write(&abandoned_path, &replacement_bytes).unwrap();
        fs::set_permissions(&abandoned_path, fs::Permissions::from_mode(0o600)).unwrap();
        let replacement_metadata = fs::symlink_metadata(&abandoned_path).unwrap();
        recovery_resume.wait();

        let error = retry
            .join()
            .unwrap()
            .expect_err("recovery must restore the live replacement and fail capacity closed");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(
            fs::read(&abandoned_path).unwrap(),
            replacement_bytes,
            "rollback must preserve the replacement rather than unlinking it"
        );
        let replacement_after = fs::symlink_metadata(&abandoned_path).unwrap();
        assert_eq!(
            (replacement_after.dev(), replacement_after.ino()),
            (replacement_metadata.dev(), replacement_metadata.ino()),
            "recovery must restore the exact replacement inode after a raced exchange"
        );
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
        assert!(
            orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context),
            "post-swap rollback must leave the recycle slot reusable"
        );
        assert!(
            recycle_path.exists(),
            "the bounded recycle slot must remain available after recovery"
        );
    }

    #[test]
    fn group_cleanup_alias_recovery_reconciles_post_exchange_residue_before_scanning() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (progress_dir, source_path, _) =
            group_cleanup_progress_race_fixture(&context, "alias-post-exchange-residue");
        let source_before = fs::read(&source_path).unwrap();
        let source_metadata_before = fs::symlink_metadata(&source_path).unwrap();
        let requested_alias = "main-c";
        let canonical = "main-controller-unique";
        let incarnation = "alias-post-exchange-incarnation";
        let idempotency_key = "alias-post-exchange-residue";
        let request_digest = "4".repeat(64);
        let incoming = serde_json::to_vec(&GroupCleanupProgressReceipt {
            schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_SCHEMA.to_string(),
            requested_session_id: Some(requested_alias.to_string()),
            principal_session_id: canonical.to_string(),
            principal_incarnation: incarnation.to_string(),
            idempotency_key: idempotency_key.to_string(),
            request_digest: request_digest.clone(),
            outcome: json!({
                "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                "completed": false,
                "_resume": {
                    "plan": {
                        "main": {
                            "session_id": canonical,
                            "session_incarnation": incarnation
                        }
                    },
                    "pending_registry_fences": [{
                        "session_id": canonical,
                        "runtime_launch_id": incarnation
                    }]
                }
            }),
        })
        .unwrap();

        orchestration::fail_group_cleanup_progress_after_recycle_for_test(&context);
        let crash_error =
            orchestration::store_group_cleanup_progress(&context, &"f".repeat(64), &incoming)
                .expect_err("the fixture must leave an active post-exchange transaction");
        assert_eq!(crash_error.code(), "orchestration-store-unavailable");
        assert_ne!(
            fs::read(&source_path).unwrap(),
            source_before,
            "the crash point must expose the not-yet-admitted prepared receipt at the stale key"
        );

        assert_eq!(
            orchestration::recover_group_cleanup_progress_principal(
                &context,
                requested_alias,
                incarnation,
                idempotency_key,
                &request_digest,
            )
            .unwrap(),
            None,
            "alias recovery must reconcile the active journal before considering receipt bodies"
        );
        assert_eq!(fs::read(&source_path).unwrap(), source_before);
        let source_metadata_after = fs::symlink_metadata(&source_path).unwrap();
        assert_eq!(
            (source_metadata_after.dev(), source_metadata_after.ino()),
            (source_metadata_before.dev(), source_metadata_before.ino()),
            "recovery must restore the exact stale inode to its origin key"
        );
        assert!(!progress_dir.join("f".repeat(64)).exists());
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_recycle_recovers_durable_residue() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-residue-recovery".to_string(),
                request_digest: "2".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let abandoned_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &abandoned_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let abandoned_path = progress_dir.join(&abandoned_key);
        fs::File::options()
            .write(true)
            .open(&abandoned_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        orchestration::seed_group_cleanup_progress_recycle_residue_for_test(&context, b"{")
            .unwrap();

        let incoming_key = "f".repeat(64);
        let incoming = progress_bytes("new-main", "new-incarnation");
        let abandoned_before = fs::read(&abandoned_path).unwrap();
        let abandoned_metadata_before = fs::symlink_metadata(&abandoned_path).unwrap();
        orchestration::fail_group_cleanup_progress_journal_sync_for_test(&context);
        let durability_error =
            orchestration::store_group_cleanup_progress(&context, &incoming_key, &incoming)
                .expect_err("exchange must not start without a durable active journal");
        assert_eq!(durability_error.code(), "orchestration-store-unavailable");
        assert_eq!(fs::read(&abandoned_path).unwrap(), abandoned_before);
        let abandoned_metadata_after = fs::symlink_metadata(&abandoned_path).unwrap();
        assert_eq!(
            (
                abandoned_metadata_after.dev(),
                abandoned_metadata_after.ino()
            ),
            (
                abandoned_metadata_before.dev(),
                abandoned_metadata_before.ino()
            ),
            "the exchange must not run before the active journal directory is durable"
        );
        assert!(!progress_dir.join(&incoming_key).exists());

        orchestration::fail_group_cleanup_progress_directory_sync_for_test(&context);
        let progress_sync_error =
            orchestration::store_group_cleanup_progress(&context, &incoming_key, &incoming)
                .expect_err("the journal must remain active until progress namespace sync");
        assert_eq!(
            progress_sync_error.code(),
            "orchestration-store-unavailable"
        );
        orchestration::store_group_cleanup_progress(&context, &incoming_key, &incoming).unwrap();

        assert_eq!(
            fs::read(progress_dir.join(&incoming_key)).unwrap(),
            incoming
        );
        assert!(!abandoned_path.exists());
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
        assert!(
            orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context),
            "a crash residue must converge to the bounded retired slot"
        );
    }

    #[test]
    fn group_cleanup_progress_journal_dirfd_rejects_parent_replacement() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let (renamed, resume, recycle_parent) =
            orchestration::install_group_cleanup_progress_journal_hook_for_test(&context);
        let writer_context = context.clone();
        let writer = thread::spawn(move || {
            orchestration::store_idle_group_cleanup_progress_recycle_journal_for_test(
                &writer_context,
            )
        });
        renamed.wait();
        let moved_parent = recycle_parent.with_file_name("group-cleanup-progress-recycle-moved");
        fs::rename(&recycle_parent, &moved_parent).unwrap();
        fs::create_dir(&recycle_parent).unwrap();
        fs::set_permissions(&recycle_parent, fs::Permissions::from_mode(0o700)).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("a replacement directory must not satisfy the durability barrier");
        assert_eq!(error.code(), "orchestration-store-unavailable");
        assert!(
            moved_parent.join("journal.json").exists(),
            "the journal remains in the exact directory pinned by the write"
        );
        assert!(
            !recycle_parent.join("journal.json").exists(),
            "the replacement directory must never be mistaken for the synced journal parent"
        );
    }

    #[test]
    fn group_cleanup_progress_exchange_keeps_the_pinned_recycle_parent() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: "pinned-recycle-incarnation".to_string(),
                idempotency_key: "pinned-recycle-parent".to_string(),
                request_digest: "5".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false
                }),
            })
            .unwrap()
        };
        let source_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &source_key,
            &progress_bytes("abandoned-main-0"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let source_path = progress_dir.join(&source_key);
        fs::File::options()
            .write(true)
            .open(&source_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&format!("abandoned-main-{index}"))).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        let source_before = fs::read(&source_path).unwrap();
        let source_metadata_before = fs::symlink_metadata(&source_path).unwrap();
        let (ready, resume, recycle_parent) =
            orchestration::install_group_cleanup_progress_pre_exchange_hook_for_test(&context);
        let writer_context = context.clone();
        let incoming_key = "f".repeat(64);
        let incoming = progress_bytes("new-main");
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(&writer_context, &incoming_key, &incoming)
        });
        ready.wait();
        let moved_parent =
            recycle_parent.with_file_name("group-cleanup-progress-recycle-post-active");
        fs::rename(&recycle_parent, &moved_parent).unwrap();
        fs::create_dir(&recycle_parent).unwrap();
        fs::set_permissions(&recycle_parent, fs::Permissions::from_mode(0o700)).unwrap();
        let replacement_slot = recycle_parent.join("slot");
        fs::write(&replacement_slot, b"replacement-slot").unwrap();
        fs::set_permissions(&replacement_slot, fs::Permissions::from_mode(0o600)).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("post-journal parent replacement must abort before exchange");
        assert_eq!(error.code(), "orchestration-store-unavailable");
        assert_eq!(fs::read(&source_path).unwrap(), source_before);
        let source_metadata_after = fs::symlink_metadata(&source_path).unwrap();
        assert_eq!(
            (source_metadata_after.dev(), source_metadata_after.ino()),
            (source_metadata_before.dev(), source_metadata_before.ino())
        );
        assert_eq!(fs::read(&replacement_slot).unwrap(), b"replacement-slot");
        assert!(!progress_dir.join("f".repeat(64)).exists());
        assert!(moved_parent.join("journal.json").exists());
    }

    #[test]
    fn group_cleanup_progress_recycle_compacts_bytes_without_path_deletion() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, padding: usize| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: "retention-byte-incarnation".to_string(),
                idempotency_key: "retention-byte-compaction".to_string(),
                request_digest: "3".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "padding": "x".repeat(padding)
                }),
            })
            .unwrap()
        };
        let current_key = "f".repeat(64);
        orchestration::store_group_cleanup_progress(
            &context,
            &current_key,
            &progress_bytes("current-main", 0),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let stale_padding = 1024 * 1024 - 2048;
        let mut large_paths = Vec::new();
        let stale_bytes = progress_bytes("abandoned-main", stale_padding);
        for index in 0..15 {
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, &stale_bytes).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            large_paths.push(path);
        }

        let incoming = progress_bytes("current-main", 4 * 1024 * 1024 - 2048);
        let retired_len = serde_json::to_vec(&GroupCleanupProgressReceipt {
            schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
            requested_session_id: None,
            principal_session_id: "retired-progress".to_string(),
            principal_incarnation: "retired-progress".to_string(),
            idempotency_key: "retired-progress".to_string(),
            request_digest: "0".repeat(64),
            outcome: json!({}),
        })
        .unwrap()
        .len() as u64;
        let projected_bytes =
            large_paths.len() as u64 * stale_bytes.len() as u64 + incoming.len() as u64;
        let reclaim_per_compaction = stale_bytes.len() as u64 - retired_len;
        let expected_compactions = projected_bytes
            .saturating_sub(16 * 1024 * 1024)
            .div_ceil(reclaim_per_compaction) as usize;
        assert!(
            expected_compactions >= 2,
            "fixture must require multiple stale compactions"
        );
        orchestration::fail_group_cleanup_progress_directory_sync_for_test(&context);
        let sync_error =
            orchestration::store_group_cleanup_progress(&context, &current_key, &incoming)
                .expect_err("byte compaction must sync the progress namespace before idle");
        assert_eq!(sync_error.code(), "orchestration-store-unavailable");
        orchestration::reset_group_cleanup_progress_compactions_for_test();
        orchestration::store_group_cleanup_progress(&context, &current_key, &incoming).unwrap();

        assert_eq!(fs::read(progress_dir.join(current_key)).unwrap(), incoming);
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 16);
        let compacted = large_paths
            .iter()
            .filter(|path| {
                fs::read(path).is_ok_and(|bytes| {
                    serde_json::from_slice::<Value>(&bytes).is_ok_and(|receipt| {
                        receipt["principal_session_id"].as_str() == Some("retired-progress")
                    })
                })
            })
            .count();
        assert_eq!(
            compacted, expected_compactions,
            "successful admission must use the exact planned stale compaction count"
        );
        assert_eq!(
            orchestration::group_cleanup_progress_compactions_for_test() as usize,
            expected_compactions,
            "the retry must execute only the minimum sufficient compaction prefix"
        );
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_byte_compaction_recovers_when_source_disappears_before_exchange() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (source_path, current_key, incoming) = group_cleanup_progress_byte_pressure_fixture(
            &context,
            "byte-source-missing-before-exchange",
        );
        let (ready, resume, _) =
            orchestration::install_group_cleanup_progress_pre_exchange_hook_for_test(&context);
        let writer_context = context.clone();
        let writer_key = current_key.clone();
        let writer_incoming = incoming.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(
                &writer_context,
                &writer_key,
                &writer_incoming,
            )
        });
        ready.wait();
        fs::remove_file(&source_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("a vanished pre-exchange source must abort the planned compaction");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        orchestration::store_group_cleanup_progress(&context, &current_key, &incoming).unwrap();
        assert!(
            !source_path.exists(),
            "recovery must not recreate a source deliberately removed before exchange"
        );
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_byte_compaction_recovers_when_source_disappears_after_exchange() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let (source_path, current_key, incoming) = group_cleanup_progress_byte_pressure_fixture(
            &context,
            "byte-source-missing-after-exchange",
        );
        let (swapped, resume, _) =
            orchestration::install_group_cleanup_progress_recycle_hook_for_test(&context);
        orchestration::fail_group_cleanup_progress_after_recycle_for_test(&context);
        let writer_context = context.clone();
        let writer_key = current_key.clone();
        let writer_incoming = incoming.clone();
        let writer = thread::spawn(move || {
            orchestration::store_group_cleanup_progress(
                &writer_context,
                &writer_key,
                &writer_incoming,
            )
        });
        swapped.wait();
        fs::remove_file(&source_path).unwrap();
        resume.wait();

        let error = writer
            .join()
            .unwrap()
            .expect_err("the injected crash must preserve the active post-exchange journal");
        assert_eq!(error.code(), "orchestration-store-unavailable");
        orchestration::store_group_cleanup_progress(&context, &current_key, &incoming).unwrap();
        assert!(
            !source_path.exists(),
            "recovery must not recreate a source removed after the exact exchange"
        );
        assert!(orchestration::group_cleanup_progress_recycle_slot_is_retired_for_test(&context));
    }

    #[test]
    fn group_cleanup_progress_capacity_preflight_does_not_compact_when_reclaim_is_insufficient() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str,
                              principal_incarnation: &str,
                              padding: usize| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "capacity-preflight".to_string(),
                request_digest: "8".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "padding": "x".repeat(padding),
                }),
            })
            .unwrap()
        };
        let stale_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &stale_key,
            &progress_bytes("stale-main", "stale-incarnation", 0),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let stale_path = progress_dir.join(&stale_key);
        fs::File::options()
            .write(true)
            .open(&stale_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..=4 {
            let session_id = format!("live-capacity-main-{index}");
            let incarnation = format!("live-capacity-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            orchestration::store_group_cleanup_progress(
                &context,
                &format!("{index:064x}"),
                &progress_bytes(&session_id, &incarnation, 3_200_000),
            )
            .unwrap();
        }

        orchestration::reset_group_cleanup_progress_compactions_for_test();
        let error = orchestration::store_group_cleanup_progress(
            &context,
            &"f".repeat(64),
            &progress_bytes("incoming-main", "incoming-incarnation", 4_000_000),
        )
        .expect_err("insufficient total reclaim must fail before durable compaction");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(
            orchestration::group_cleanup_progress_compactions_for_test(),
            0,
            "capacity preflight must not start sync-heavy compaction when admission is impossible"
        );
        assert_eq!(
            fs::read(&stale_path).unwrap(),
            progress_bytes("stale-main", "stale-incarnation", 0),
            "failed preflight must leave the stale candidate unchanged"
        );
    }

    #[test]
    fn group_cleanup_progress_recycle_does_not_install_before_combined_capacity() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str,
                              principal_incarnation: &str,
                              padding: usize| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-combined-capacity".to_string(),
                request_digest: "4".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "padding": "x".repeat(padding)
                }),
            })
            .unwrap()
        };
        let stale_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &stale_key,
            &progress_bytes("abandoned-main", "abandoned-incarnation", 0),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let stale_path = progress_dir.join(&stale_key);
        fs::File::options()
            .write(true)
            .open(&stale_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        let large_padding = 4 * 1024 * 1024 - 2048;
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let padding = usize::from(index <= 3) * large_padding;
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation, padding)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let incoming_key = "f".repeat(64);
        let incoming = progress_bytes("new-main", "new-incarnation", large_padding);
        let error = orchestration::store_group_cleanup_progress(&context, &incoming_key, &incoming)
            .expect_err("combined pressure without enough stale bytes must fail closed");

        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert!(
            !progress_dir.join(incoming_key).exists(),
            "the incoming checkpoint is the final mutation after both limits are secured"
        );
        let aggregate = fs::read_dir(&progress_dir)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum::<u64>();
        assert!(aggregate <= 16 * 1024 * 1024);
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
    }

    #[test]
    fn group_cleanup_progress_retention_treats_uncertain_session_lookup_as_live_capacity() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "retention-uncertain-session".to_string(),
                request_digest: "f".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        for session_id in ["uncertain-main-a", "uncertain-main-b"] {
            crate::write_session_record(
                &context,
                &cleanup_test_session(session_id, "uncertain-incarnation"),
            )
            .unwrap();
        }
        let uncertain_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(
            &context,
            &uncertain_key,
            &progress_bytes("uncertain-main", "uncertain-incarnation"),
        )
        .unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let uncertain_path = progress_dir.join(&uncertain_key);
        fs::File::options()
            .write(true)
            .open(&uncertain_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-main-{index}");
            let incarnation = format!("live-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let error = orchestration::store_group_cleanup_progress(
            &context,
            &"f".repeat(64),
            &progress_bytes("new-main", "new-incarnation"),
        )
        .expect_err("uncertain session lookup must fail capacity admission closed");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert!(
            uncertain_path.exists(),
            "an uncertain principal lookup must never make progress evictable"
        );
        assert_eq!(fs::read_dir(&progress_dir).unwrap().count(), 128);
    }

    #[test]
    fn group_cleanup_progress_retention_preserves_legacy_alias_fence_after_alias_reuse() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
            serde_json::to_vec(&GroupCleanupProgressReceipt {
                schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
                requested_session_id: None,
                principal_session_id: principal_session_id.to_string(),
                principal_incarnation: principal_incarnation.to_string(),
                idempotency_key: "v1-alias-fence-retention".to_string(),
                request_digest: "7".repeat(64),
                outcome: json!({
                    "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                    "completed": false,
                    "workers": []
                }),
            })
            .unwrap()
        };
        let legacy_bytes = serde_json::to_vec(&GroupCleanupProgressReceipt {
            schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA.to_string(),
            requested_session_id: None,
            principal_session_id: "main-c".to_string(),
            principal_incarnation: "original-incarnation".to_string(),
            idempotency_key: "v1-alias-fence-retention".to_string(),
            request_digest: "7".repeat(64),
            outcome: json!({
                "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                "completed": false,
                "_resume": {
                    "plan": {
                        "main": {
                            "session_id": "main-controller",
                            "session_incarnation": "original-incarnation"
                        }
                    },
                    "pending_registry_fences": [{
                        "session_id": "main-controller",
                        "runtime_launch_id": "original-incarnation"
                    }]
                }
            }),
        })
        .unwrap();
        crate::write_session_record(
            &context,
            &cleanup_test_session("main-cat", "replacement-incarnation"),
        )
        .unwrap();
        let legacy_key = format!("{:064x}", 0);
        orchestration::store_group_cleanup_progress(&context, &legacy_key, &legacy_bytes).unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        let legacy_path = progress_dir.join(&legacy_key);
        fs::File::options()
            .write(true)
            .open(&legacy_path)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
            )
            .unwrap();
        for index in 1..128 {
            let session_id = format!("live-v1-main-{index}");
            let incarnation = format!("live-v1-incarnation-{index}");
            crate::write_session_record(&context, &cleanup_test_session(&session_id, &incarnation))
                .unwrap();
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let error = orchestration::store_group_cleanup_progress(
            &context,
            &"f".repeat(64),
            &progress_bytes("incoming-main", "incoming-incarnation"),
        )
        .expect_err("a matching canonical pending fence must protect released-v1 alias progress");
        assert_eq!(error.code(), "group-cleanup-progress-capacity");
        assert_eq!(fs::read(&legacy_path).unwrap(), legacy_bytes);
    }

    #[test]
    fn group_cleanup_progress_retention_protects_stable_parse_and_open_failures() {
        let _fixture_ownership = GlobalStateLock::new();
        for failure in ["malformed", "unreadable"] {
            let tmp = tempfile::TempDir::new().unwrap();
            let context = CliContext {
                state_dir: tmp.path().join("state"),
                host: None,
            };
            fs::create_dir(&context.state_dir).unwrap();
            let progress_bytes = |principal_session_id: &str, principal_incarnation: &str| {
                serde_json::to_vec(&GroupCleanupProgressReceipt {
                    schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_V1_SCHEMA
                        .to_string(),
                    requested_session_id: None,
                    principal_session_id: principal_session_id.to_string(),
                    principal_incarnation: principal_incarnation.to_string(),
                    idempotency_key: "retention-reader-failure".to_string(),
                    request_digest: "a".repeat(64),
                    outcome: json!({
                        "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                        "completed": false,
                        "workers": []
                    }),
                })
                .unwrap()
            };
            let protected_key = format!("{:064x}", 0);
            let protected_bytes = match failure {
                "malformed" => b"{".to_vec(),
                "unreadable" => progress_bytes("unreadable-main", "unreadable-incarnation"),
                _ => unreachable!(),
            };
            orchestration::store_group_cleanup_progress(&context, &protected_key, &protected_bytes)
                .unwrap();
            let progress_dir = context
                .state_dir
                .join("orchestration/group-cleanup-progress");
            let protected_path = progress_dir.join(&protected_key);
            fs::File::options()
                .write(true)
                .open(&protected_path)
                .unwrap()
                .set_times(
                    fs::FileTimes::new()
                        .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1)),
                )
                .unwrap();
            for index in 1..128 {
                let session_id = format!("live-main-{index}");
                let incarnation = format!("live-incarnation-{index}");
                crate::write_session_record(
                    &context,
                    &cleanup_test_session(&session_id, &incarnation),
                )
                .unwrap();
                let path = progress_dir.join(format!("{index:064x}"));
                fs::write(&path, progress_bytes(&session_id, &incarnation)).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            }
            if failure == "unreadable" {
                orchestration::fail_group_cleanup_progress_read_for_test(&protected_path);
            }

            let error = orchestration::store_group_cleanup_progress(
                &context,
                &"f".repeat(64),
                &progress_bytes("new-main", "new-incarnation"),
            )
            .expect_err("unverifiable progress must fail capacity admission closed");
            assert_eq!(
                error.code(),
                "group-cleanup-progress-capacity",
                "failure={failure}"
            );
            assert!(protected_path.exists(), "failure={failure}");
            assert_eq!(
                fs::read_dir(&progress_dir).unwrap().count(),
                128,
                "failure={failure}"
            );
        }
    }

    #[test]
    fn group_cleanup_progress_recovery_requires_an_exact_durable_requested_selector() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let canonical = "main-controller-unique";
        let alias = "main-c";
        let collision = "main-cat";
        let incarnation = "main-incarnation";
        let idempotency_key = "exact-progress-selector";
        let request_digest = "f".repeat(64);
        let outcome = json!({
            "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
            "completed": false,
            "_resume": {
                "plan": {
                    "main": {
                        "session_id": canonical,
                        "session_incarnation": incarnation
                    }
                },
                "pending_registry_fences": [{
                    "session_id": canonical,
                    "runtime_launch_id": incarnation
                }]
            }
        });
        let progress_key = "a".repeat(64);
        orchestration::store_group_cleanup_progress(
            &context,
            &progress_key,
            &serde_json::to_vec(&json!({
                "schema_version": "agent-session.main-agent-group-cleanup-receipt.v1",
                "principal_session_id": canonical,
                "principal_incarnation": incarnation,
                "idempotency_key": idempotency_key,
                "request_digest": request_digest,
                "outcome": outcome,
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            orchestration::recover_group_cleanup_progress_principal(
                &context,
                alias,
                incarnation,
                idempotency_key,
                &request_digest,
            )
            .unwrap(),
            None,
            "a canonical principal that merely shares the alias prefix is not durable alias evidence"
        );

        orchestration::remove_group_cleanup_progress(&context, &progress_key).unwrap();
        orchestration::store_group_cleanup_progress(
            &context,
            &progress_key,
            &serde_json::to_vec(&json!({
                "schema_version": "agent-session.main-agent-group-cleanup-receipt.v2",
                "requested_session_id": alias,
                "principal_session_id": canonical,
                "principal_incarnation": incarnation,
                "idempotency_key": idempotency_key,
                "request_digest": request_digest,
                "outcome": outcome,
            }))
            .unwrap(),
        )
        .unwrap();
        orchestration::store_group_cleanup_progress(
            &context,
            &"b".repeat(64),
            &serde_json::to_vec(&json!({
                "schema_version": "agent-session.main-agent-group-cleanup-receipt.v2",
                "requested_session_id": "other-selector",
                "principal_session_id": "other-principal",
                "principal_incarnation": "other-incarnation",
                "idempotency_key": "other-idempotency-key",
                "request_digest": "0".repeat(64),
                "outcome": {},
            }))
            .unwrap(),
        )
        .unwrap();
        orchestration::reset_group_cleanup_progress_receipt_decodes_for_test();
        assert_eq!(
            orchestration::recover_group_cleanup_progress_principal(
                &context,
                alias,
                incarnation,
                idempotency_key,
                &request_digest,
            )
            .unwrap()
            .as_deref(),
            Some(canonical),
            "the exact requested selector must recover its canonical principal"
        );
        assert_eq!(
            orchestration::group_cleanup_progress_receipt_decodes_for_test(),
            2,
            "each candidate body must be decoded exactly once while recovery holds the progress lock"
        );
        assert_eq!(
            orchestration::recover_group_cleanup_progress_principal(
                &context,
                collision,
                incarnation,
                idempotency_key,
                &request_digest,
            )
            .unwrap(),
            None,
            "a longer colliding selector must not adopt another alias mapping"
        );
    }

    #[test]
    fn group_cleanup_progress_recovery_rejects_a_wrong_selector_without_pending_fences() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let canonical = "main-controller-unique";
        let original_alias = "main-c";
        let other_alias = "main-cat";
        let incarnation = "main-incarnation";
        let idempotency_key = "exact-progress-selector-without-fences";
        let request_digest = "e".repeat(64);
        let progress = GroupCleanupProgressReceipt {
            schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_SCHEMA.to_string(),
            requested_session_id: Some(original_alias.to_string()),
            principal_session_id: canonical.to_string(),
            principal_incarnation: incarnation.to_string(),
            idempotency_key: idempotency_key.to_string(),
            request_digest: request_digest.clone(),
            outcome: json!({
                "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                "completed": true,
                "_resume": {
                    "plan": {
                        "main": {
                            "session_id": canonical,
                            "session_incarnation": incarnation
                        }
                    },
                    "pending_registry_fences": []
                }
            }),
        };
        orchestration::store_group_cleanup_progress(
            &context,
            &"a".repeat(64),
            &serde_json::to_vec(&progress).unwrap(),
        )
        .unwrap();

        assert_eq!(
            orchestration::recover_group_cleanup_progress_principal(
                &context,
                other_alias,
                incarnation,
                idempotency_key,
                &request_digest,
            )
            .unwrap(),
            None,
            "absence of a pending fence must not substitute for an exact durable selector mapping"
        );
    }

    #[test]
    fn group_cleanup_live_replay_requires_the_original_exact_selector() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        let canonical = "main-controller-unique";
        let original_alias = "main-c";
        let other_alias = "main-cont";
        let incarnation = "main-incarnation";
        let idempotency_key = "live-exact-selector";
        let request_digest = "9".repeat(64);
        let plan = GroupCleanupPlan {
            schema_version: GROUP_CLEANUP_SCHEMA.to_string(),
            main: SessionRef {
                machine: None,
                session_id: canonical.to_string(),
                session_incarnation: incarnation.to_string(),
                session_created_at: "2030-01-01T00:00:00Z".to_string(),
            },
            run_id: "run-live-selector".to_string(),
            run_revision: 7,
            requires_force: false,
            workers: Vec::new(),
            plan_digest: format!("sha256:{}", "a".repeat(64)),
        };
        let outcome = group_cleanup_stored_outcome(
            &group_cleanup_progress_value(&plan, &[], false, "authority_sealed"),
            &group_cleanup_resume_state(&plan, &[], &[], &[], false),
        )
        .unwrap();
        let progress_key = group_cleanup_progress_key(canonical, incarnation, idempotency_key);
        let progress = serde_json::to_vec(&GroupCleanupProgressReceipt {
            schema_version: orchestration::GROUP_CLEANUP_PROGRESS_RECEIPT_SCHEMA.to_string(),
            requested_session_id: Some(original_alias.to_string()),
            principal_session_id: canonical.to_string(),
            principal_incarnation: incarnation.to_string(),
            idempotency_key: idempotency_key.to_string(),
            request_digest: request_digest.to_string(),
            outcome: outcome.clone(),
        })
        .unwrap();
        orchestration::store_group_cleanup_progress(&context, &progress_key, &progress).unwrap();

        let error = match group_cleanup_replay_with_legacy_alias(
            &context,
            &orchestration::Registry::default(),
            canonical,
            Some(other_alias),
            incarnation,
            idempotency_key,
            &request_digest,
            "group-cleanup",
        ) {
            Ok(_) => panic!("a different live alias must not adopt exact-selector progress"),
            Err(error) => error,
        };
        assert_eq!(error.code(), "group-cleanup-progress-invalid");
        assert_eq!(
            orchestration::read_group_cleanup_progress(&context, &progress_key)
                .unwrap()
                .unwrap(),
            progress,
            "a rejected alias must not rewrite the original durable selector"
        );

        orchestration::remove_group_cleanup_progress(&context, &progress_key).unwrap();
        let mut registry = orchestration::Registry::default();
        store_receipt_for_principal(
            &mut registry,
            canonical,
            incarnation,
            idempotency_key,
            "group-cleanup",
            &request_digest,
            json!({
                "schema_version": GROUP_CLEANUP_RESULT_SCHEMA,
                "completed": true,
            }),
        )
        .unwrap();
        assert!(
            group_cleanup_replay_with_legacy_alias(
                &context,
                &registry,
                canonical,
                Some(other_alias),
                incarnation,
                idempotency_key,
                &request_digest,
                "group-cleanup",
            )
            .unwrap()
            .is_none(),
            "a canonical terminal receipt must not authorize a different exact alias"
        );
    }

    #[test]
    fn completed_group_cleanup_recovery_requires_an_exact_alias_receipt() {
        let canonical = "main-controller-unique";
        let alias = "main-c";
        let incarnation = "main-incarnation";
        let idempotency_key = "exact-completed-selector";
        let request_digest = "a".repeat(64);
        let outcome = json!({
            "completed": true,
            "_resume": {
                "plan": {
                    "main": {
                        "session_id": canonical,
                        "session_incarnation": incarnation
                    }
                }
            }
        });
        let mut registry = orchestration::Registry::default();
        store_receipt_for_principal(
            &mut registry,
            canonical,
            incarnation,
            idempotency_key,
            "group-cleanup",
            &request_digest,
            outcome.clone(),
        )
        .unwrap();
        assert_eq!(
            recover_completed_group_cleanup_principal(
                &registry,
                alias,
                incarnation,
                idempotency_key,
                &request_digest,
                "group-cleanup",
            )
            .unwrap(),
            None,
            "a canonical receipt must not be discovered through prefix inference"
        );

        registry.receipts.clear();
        store_receipt_for_principal(
            &mut registry,
            alias,
            incarnation,
            idempotency_key,
            "group-cleanup",
            &request_digest,
            outcome,
        )
        .unwrap();
        assert_eq!(
            recover_completed_group_cleanup_principal(
                &registry,
                alias,
                incarnation,
                idempotency_key,
                &request_digest,
                "group-cleanup",
            )
            .unwrap()
            .as_deref(),
            Some(canonical),
            "the exact alias-keyed terminal receipt must recover the canonical plan principal"
        );
        assert_eq!(
            recover_completed_group_cleanup_principal(
                &registry,
                "main-cat",
                incarnation,
                idempotency_key,
                &request_digest,
                "group-cleanup",
            )
            .unwrap(),
            None,
            "a colliding longer selector must not adopt the exact alias receipt"
        );
    }

    #[test]
    fn group_cleanup_progress_scans_stop_at_the_first_capacity_overflow() {
        let _fixture_ownership = GlobalStateLock::new();
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        fs::create_dir(&context.state_dir).unwrap();
        orchestration::store_group_cleanup_progress(&context, &"a".repeat(64), b"{}").unwrap();
        let progress_dir = context
            .state_dir
            .join("orchestration/group-cleanup-progress");
        for index in 0..300 {
            let path = progress_dir.join(format!("{index:064x}"));
            fs::write(&path, []).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }

        let visits =
            orchestration::install_group_cleanup_progress_visit_counter_for_test(&progress_dir);
        let store_error =
            orchestration::store_group_cleanup_progress(&context, &"f".repeat(64), b"{}")
                .expect_err("an externally flooded progress directory must fail capacity closed");
        orchestration::clear_group_cleanup_progress_visit_counter_for_test();
        assert_eq!(store_error.code(), "group-cleanup-progress-capacity");
        assert_eq!(
            visits.load(Ordering::Acquire),
            129,
            "retention admission must stop at the first file-count overflow"
        );

        let visits =
            orchestration::install_group_cleanup_progress_visit_counter_for_test(&progress_dir);
        let recovery_error = orchestration::recover_group_cleanup_progress_principal(
            &context,
            "main",
            "main-incarnation",
            "capacity-recovery",
            &"c".repeat(64),
        )
        .expect_err("alias recovery must reject a flooded progress directory");
        orchestration::clear_group_cleanup_progress_visit_counter_for_test();
        assert_eq!(recovery_error.code(), "group-cleanup-progress-capacity");
        assert_eq!(
            visits.load(Ordering::Acquire),
            129,
            "alias recovery must stop at the first file-count overflow"
        );
    }

    #[test]
    fn group_cleanup_run_revision_overflow_fails_before_registry_or_session_mutation() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let mut main = cleanup_test_session("main", "main-incarnation");
        crate::mark_tmux_runtime_never_launched(&mut main);
        fs::create_dir_all(session_dir(&context, &main.id)).unwrap();
        crate::write_session_record(&context, &main).unwrap();
        let mut run = run_record("run-max", false);
        run.controller = session_ref(&context, &main, "main-incarnation");
        run.revision = u64::MAX;
        {
            let mut locked = orchestration::lock_registry(&context).unwrap();
            locked.registry.runs.insert(run.run_id.clone(), run);
            locked.save().unwrap();
        }
        let preview = preview_group_cleanup(&context, "main").unwrap();
        let before_registry =
            fs::read(context.state_dir.join("orchestration/registry.json")).unwrap();
        let request = GroupCleanupRequest {
            schema_version: GROUP_CLEANUP_REQUEST_SCHEMA.to_string(),
            expected_main_incarnation: "main-incarnation".to_string(),
            expected_run_revision: u64::MAX,
            expected_plan_digest: preview["plan_digest"].as_str().unwrap().to_string(),
            mode: GroupCleanupMode::Safe,
            idempotency_key: "run-revision-overflow".to_string(),
        };

        let error = execute_group_cleanup(&context, "main", request, PathBuf::from("/bin/false"))
            .err()
            .expect("an exhausted run revision must fail before cleanup effects");
        assert_eq!(error.code(), "orchestration-revision-capacity");
        assert_eq!(
            fs::read(context.state_dir.join("orchestration/registry.json")).unwrap(),
            before_registry,
            "run revision overflow must leave the registry byte-for-byte unchanged"
        );
        assert!(
            session_dir(&context, "main").exists(),
            "run revision overflow must not delete the Main Agent"
        );
    }
}

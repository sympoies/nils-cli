
use super::*;
use crate::{
    CliContext, ProviderResume, RecordRequest, SessionRecord, create_record, write_session_record,
};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::sync::{Arc, Barrier};

fn event(kind: TurnEventKind, event_id: &str) -> TurnEvent {
    TurnEvent {
        schema_version: TURN_EVENT_VERSION.to_string(),
        event_id: event_id.to_string(),
        runtime_id: "runtime-1".to_string(),
        provider: "codex".to_string(),
        provider_session_id: Some("session-1".to_string()),
        provider_turn_id: Some("turn-1".to_string()),
        kind,
        failure_reason: None,
        attention_id: None,
        attention_kind: None,
        attention_correlation_ambiguous: false,
        attention_correlation_exact: false,
        confidence: Confidence::Observed,
        source_kind: SourceKind::ProviderHook,
        provider_time: None,
    }
}

fn document() -> ActivityDocument {
    ActivityDocument {
        schema_version: ACTIVITY_DOCUMENT_VERSION.to_string(),
        runtime_id: "runtime-1".to_string(),
        runtime_generation: 1,
        state: starting_state("2026-07-10T00:00:00Z".to_string(), 1, None),
        pending_attention: Vec::new(),
        overflow_attention: None,
        seen_event_count: 0,
        last_semantic_event: None,
        last_semantic_event_at: None,
        last_provider_event_kind: None,
        last_provider_event_provider: None,
        last_provider_event_at: None,
        last_provider_event_turn_id: None,
        provider_session_id: None,
        last_event_at: None,
        pending_journal: None,
        runtime_unhealthy_reason: None,
        operator_provider_turn_receipts: Vec::new(),
        extra: Map::new(),
    }
}

#[test]
fn activity_document_without_operator_receipts_remains_compatible() {
    let mut value = serde_json::to_value(document()).expect("activity document");
    value
        .as_object_mut()
        .expect("activity object")
        .remove("operator_provider_turn_receipts");

    let restored: ActivityDocument =
        serde_json::from_value(value).expect("pre-receipt activity document");
    assert!(restored.operator_provider_turn_receipts.is_empty());
}

#[test]
fn idless_completion_does_not_inherit_unrelated_operator_reconciliation() {
    let mut document = document();
    let mut reconciliation = Map::new();
    reconciliation.insert(
        "operator_reconciliation".to_string(),
        json!({"canary": true}),
    );
    document.state.last_turn = Some(LastTurn {
        provider_turn_id: Some("prior-provider-turn".to_string()),
        started_at: Some("2026-07-10T00:00:00Z".to_string()),
        completed_at: "2026-07-10T00:00:01Z".to_string(),
        outcome: "operator_reconciled".to_string(),
        extra: reconciliation,
    });
    let mut completion = event(TurnEventKind::TurnCompleted, "unrelated-idless-completion");
    completion.provider_turn_id = None;

    reduce(&mut document, &completion, "2026-07-10T00:00:02Z");

    assert!(
        document
            .state
            .last_turn
            .as_ref()
            .is_some_and(|turn| !turn.extra.contains_key("operator_reconciliation"))
    );
}

#[test]
fn replacement_runtime_idless_completion_does_not_migrate_reconciliation_provenance() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
    reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        revision,
        "operator-provider-turn-provenance",
    )
    .expect("reconcile prior runtime");
    let mut replacement = created.record.clone();
    let replacement_runtime_id = {
        let replacement_runtime = replacement.runtime.as_mut().expect("runtime");
        replacement_runtime.launch_id = "replacement-runtime".to_string();
        replacement_runtime.generation = replacement_runtime.generation.saturating_add(1);
        replacement_runtime.started_at = "2026-07-10T00:00:03Z".to_string();
        replacement_runtime.launch_id.clone()
    };
    write_session_record(&context, &replacement).expect("replacement session");
    activate_runtime(&context, &replacement).expect("replacement activity");
    let mut completion = event(
        TurnEventKind::TurnCompleted,
        "replacement-runtime-idless-completion",
    );
    completion.runtime_id = replacement_runtime_id;
    completion.provider_turn_id = None;

    let completed = ingest_event(&context, &replacement.id, completion).expect("idless completion");
    let completed = serde_json::to_value(completed.turn_state).expect("completed state");

    assert_eq!(completed["last_turn"]["outcome"], "completed");
    assert!(
        completed["last_turn"]
            .get("operator_reconciliation")
            .is_none()
    );
}

fn prepare_operator_turn(
    tmp: &tempfile::TempDir,
) -> (CliContext, crate::CreatedRecord, String, u64) {
    let (context, created) = test_session(tmp);
    activate_runtime(&context, &created.record).expect("activate runtime");
    let runtime_id = created
        .record
        .runtime
        .as_ref()
        .expect("runtime")
        .launch_id
        .clone();
    for (event_id, kind) in [
        ("operator-start", TurnEventKind::TurnStarted),
        ("operator-stop", TurnEventKind::StopObserved),
    ] {
        let mut provider_event = event(kind, event_id);
        provider_event.runtime_id = runtime_id.clone();
        ingest_event(&context, &created.record.id, provider_event).expect("provider event");
    }
    let revision = activity_status(&context, &created.record.id)
        .expect("activity")
        .turn_state
        .revision;
    (context, created, runtime_id, revision)
}

fn reconcile_operator_turn(
    context: &CliContext,
    record: &SessionRecord,
    runtime_id: &str,
    revision: u64,
    idempotency_key: &str,
) -> Result<Value, CliError> {
    let provider_turn_id = activity_status(context, &record.id)?
        .turn_state
        .current_turn
        .as_ref()
        .and_then(|turn| turn.provider_turn_id.as_deref())
        .ok_or_else(|| {
            CliError::data(
                "provider-turn-id-mismatch",
                "test fixture requires a canonical current provider turn",
                None,
            )
        })?
        .to_string();
    let dir = session_dir(context, &record.id);
    let activity_lock = acquire_lock(&dir)?;
    let health_fence = acquire_runtime_health_fence(context, record)?;
    operator_reconcile_provider_turn_locked(
        context,
        &record.id,
        &activity_lock,
        &health_fence,
        OperatorProviderTurnReconcileInput {
            session_incarnation: runtime_id,
            runtime_launch_id: runtime_id,
            runtime_generation: record.runtime.as_ref().expect("runtime").generation,
            activity_revision: revision,
            provider: &record.agent,
            provider_turn_id: &provider_turn_id,
            reason: "authoritative-completion-signal-missing",
            idempotency_key,
            request_digest: idempotency_key,
        },
    )
}

fn test_operator_receipt(
    idempotency_key: String,
    request_digest: String,
    expires_at_epoch: i64,
) -> OperatorProviderTurnReceipt {
    OperatorProviderTurnReceipt {
        idempotency_key,
        request_digest,
        reason: "authoritative-completion-signal-missing".to_string(),
        reconciliation: OperatorProviderTurnReconciliation {
            schema_version: "agent-session.operator-provider-turn-reconciliation.v1".to_string(),
            provider_turn_id: "local:v1:test-provider-turn".to_string(),
            state: "operator_reconciled".to_string(),
            activity_revision_before: 2,
            activity_revision_after: 3,
            reconciled_at: "2026-07-10T00:00:02Z".to_string(),
            reason: "authoritative-completion-signal-missing".to_string(),
            provenance: "server_operator".to_string(),
        },
        expires_at_epoch,
    }
}

#[test]
fn operator_reconcile_recovers_pre_selector_snapshot_from_exact_journal_tail() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut pre_selector_document: Value =
        serde_json::from_slice(&fs::read(&path).expect("activity bytes"))
            .expect("activity document");
    pre_selector_document
        .as_object_mut()
        .expect("activity object")
        .remove("last_provider_event_turn_id");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&pre_selector_document).expect("pre-selector activity document"),
    )
    .expect("seed pre-selector activity");

    let result = reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        revision,
        "operator-provider-turn-pre-selector",
    )
    .expect("exact journal tail should recover the absent selector");

    assert_eq!(
        result["provider_turn_reconciliation"]["state"],
        json!("operator_reconciled")
    );
}

#[test]
fn operator_reconcile_pre_selector_rejections_are_independently_fail_closed() {
    for case in [
        "present-mismatch",
        "tail-timestamp-mismatch",
        "malformed-middle",
        "missing-final-newline",
    ] {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
        let dir = session_dir(&context, &created.record.id);
        let path = dir.join(ACTIVITY_FILE);
        let journal_path = dir.join(ACTIVITY_JOURNAL_FILE);
        let mut activity: Value = serde_json::from_slice(&fs::read(&path).expect("activity bytes"))
            .expect("activity document");
        let activity_object = activity.as_object_mut().expect("activity object");
        if case == "present-mismatch" {
            activity_object.insert(
                "last_provider_event_turn_id".to_string(),
                json!("local:v1:present-but-different"),
            );
        } else {
            activity_object.remove("last_provider_event_turn_id");
        }
        fs::write(
            &path,
            serde_json::to_vec_pretty(&activity).expect("pre-selector activity document"),
        )
        .expect("seed activity");

        match case {
            "tail-timestamp-mismatch" => {
                let journal = fs::read_to_string(&journal_path).expect("journal");
                let mut entries = journal
                    .lines()
                    .map(|line| serde_json::from_str::<JournalEntry>(line).expect("entry"))
                    .collect::<Vec<_>>();
                entries.last_mut().expect("exact journal tail").received_at =
                    "2030-01-01T00:00:00Z".to_string();
                let mut changed = Vec::new();
                for entry in entries {
                    serde_json::to_writer(&mut changed, &entry).expect("render entry");
                    changed.push(b'\n');
                }
                fs::write(&journal_path, changed).expect("seed timestamp-mismatched tail");
            }
            "malformed-middle" => {
                let journal = fs::read_to_string(&journal_path).expect("journal");
                let mut lines = journal.lines().collect::<Vec<_>>();
                let exact_tail = lines.pop().expect("exact journal tail");
                let mut malformed = lines.join("\n");
                malformed.push_str("\n{not-valid-json}\n");
                malformed.push_str(exact_tail);
                malformed.push('\n');
                fs::write(&journal_path, malformed).expect("seed malformed journal");
            }
            "missing-final-newline" => {
                let mut journal = fs::read(&journal_path).expect("journal");
                assert_eq!(journal.pop(), Some(b'\n'));
                fs::write(&journal_path, journal).expect("seed truncated journal");
            }
            "present-mismatch" => {}
            _ => unreachable!("bounded rejection case"),
        }
        let activity_before = fs::read(&path).expect("activity before rejection");
        let journal_before = fs::read(&journal_path).expect("journal before rejection");

        let error = reconcile_operator_turn(
            &context,
            &created.record,
            &runtime_id,
            revision,
            &format!("operator-provider-turn-pre-selector-{case}"),
        )
        .expect_err("pre-selector boundary must reject");

        assert_eq!(
            error.code(),
            "operator-provider-turn-reconcile-not-admissible",
            "{case}"
        );
        assert_eq!(
            fs::read(&path).expect("activity after rejection"),
            activity_before,
            "{case}"
        );
        assert_eq!(
            fs::read(&journal_path).expect("journal after rejection"),
            journal_before,
            "{case}"
        );
    }
}

#[test]
fn operator_receipt_quota_admits_64_and_preserves_state_on_65th() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut document = read_document(&path).expect("activity");
    document.operator_provider_turn_receipts = (0..63)
        .map(|index| {
            test_operator_receipt(
                format!("quota-key-{index:02}"),
                format!("quota-digest-{index:02}"),
                i64::MAX,
            )
        })
        .collect();
    write_document(&path, &mut document).expect("seed receipts");

    reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        revision,
        "quota-key-63",
    )
    .expect("64th receipt");
    assert_eq!(
        read_document(&path)
            .expect("activity after 64th")
            .operator_provider_turn_receipts
            .len(),
        64
    );

    for (event_id, kind) in [
        ("operator-start-second", TurnEventKind::TurnStarted),
        ("operator-stop-second", TurnEventKind::StopObserved),
    ] {
        let mut provider_event = event(kind, event_id);
        provider_event.runtime_id = runtime_id.clone();
        ingest_event(&context, &created.record.id, provider_event).expect("second event");
    }
    let second_revision = activity_status(&context, &created.record.id)
        .expect("second activity")
        .turn_state
        .revision;
    let before = fs::read(&path).expect("activity before quota rejection");
    let error = reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        second_revision,
        "quota-key-64",
    )
    .expect_err("65th receipt must reject");
    assert_eq!(error.code(), "quota-exceeded");
    assert_eq!(
        fs::read(&path).expect("activity after quota rejection"),
        before
    );
}

#[test]
fn expired_operator_receipts_prune_on_hot_path_and_key_is_reusable() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut document = read_document(&path).expect("activity");
    document.operator_provider_turn_receipts = (0..64)
        .map(|index| {
            test_operator_receipt(
                if index == 0 {
                    "expired-reusable-key".to_string()
                } else {
                    format!("expired-key-{index:02}")
                },
                "expired-digest".to_string(),
                0,
            )
        })
        .collect();
    let seeded = serde_json::to_vec_pretty(&document).expect("expired receipt document");
    fs::write(&path, seeded).expect("seed expired receipts");

    reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        revision,
        "expired-reusable-key",
    )
    .expect("expired key reuse");
    let persisted = read_document(&path).expect("activity after expired key reuse");
    assert_eq!(persisted.operator_provider_turn_receipts.len(), 1);
    assert_eq!(
        persisted.operator_provider_turn_receipts[0].idempotency_key,
        "expired-reusable-key"
    );
    let live_receipt =
        serde_json::to_value(&persisted.operator_provider_turn_receipts[0]).expect("live receipt");

    let mut with_expired_hot_path_receipt = persisted;
    with_expired_hot_path_receipt
        .operator_provider_turn_receipts
        .push(test_operator_receipt(
            "expired-hot-path-key".to_string(),
            "expired-hot-path-digest".to_string(),
            0,
        ));
    fs::write(
        &path,
        serde_json::to_vec_pretty(&with_expired_hot_path_receipt)
            .expect("hot-path receipt document"),
    )
    .expect("seed hot-path expired receipt");
    let mut progress = event(TurnEventKind::Progress, "post-reconcile-progress");
    progress.runtime_id = runtime_id;
    ingest_event(&context, &created.record.id, progress).expect("ordinary provider event");
    let persisted = read_document(&path).expect("activity after hot-path prune");
    assert_eq!(
        persisted.operator_provider_turn_receipts.len(),
        1,
        "the current live receipt remains while expired payloads stay pruned"
    );
    assert_eq!(
        serde_json::to_value(&persisted.operator_provider_turn_receipts[0])
            .expect("persisted live receipt"),
        live_receipt,
        "ordinary persistence must not mutate the retained live receipt"
    );
}

#[test]
fn operator_receipt_ttl_boundary_and_pending_journal_replay_are_exact_and_read_only() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, created, runtime_id, revision) = prepare_operator_turn(&tmp);
    let runtime_generation = created.record.runtime.as_ref().expect("runtime").generation;
    let admission_before = crate::coordination::now_epoch();
    let result = reconcile_operator_turn(
        &context,
        &created.record,
        &runtime_id,
        revision,
        "operator-provider-turn-ttl-boundary",
    )
    .expect("reconcile");
    let admission_after = crate::coordination::now_epoch();
    let path = session_dir(&context, &created.record.id).join(ACTIVITY_FILE);
    let mut document = read_document(&path).expect("reconciled activity");
    let receipt = document
        .operator_provider_turn_receipts
        .first()
        .expect("receipt");
    let expires_at_epoch = receipt.expires_at_epoch;
    assert!(
        admission_before + 24 * 60 * 60 <= expires_at_epoch
            && expires_at_epoch <= admission_after + 24 * 60 * 60
    );
    let encoded = serde_json::to_vec(receipt).expect("encoded receipt");
    assert!(encoded.len() < 1_024, "receipt must remain compact");
    let encoded = String::from_utf8(encoded).expect("receipt utf8");
    assert!(!encoded.contains("\"result\""));
    assert!(!encoded.contains("operator-provider-turn-reconcile-result"));
    let lock = acquire_coordination_activity_lock(&context, &created.record.id).expect("lock");
    let mut replay_result = result.clone();
    replay_result["session_incarnation"] = json!("distinct-session-incarnation");
    assert_eq!(
        operator_provider_turn_replay_locked_at(
            &context,
            &created.record.id,
            &lock,
            OperatorProviderTurnReplaySelector {
                session_incarnation: "distinct-session-incarnation",
                runtime_launch_id: &runtime_id,
                runtime_generation,
            },
            "operator-provider-turn-ttl-boundary",
            "operator-provider-turn-ttl-boundary",
            expires_at_epoch - 1,
        )
        .expect("replay before expiry"),
        Some(replay_result)
    );
    assert_eq!(
        operator_provider_turn_replay_locked_at(
            &context,
            &created.record.id,
            &lock,
            OperatorProviderTurnReplaySelector {
                session_incarnation: &runtime_id,
                runtime_launch_id: &runtime_id,
                runtime_generation,
            },
            "operator-provider-turn-ttl-boundary",
            "operator-provider-turn-ttl-boundary",
            expires_at_epoch,
        )
        .expect("lookup at expiry"),
        None
    );

    document.pending_journal = Some(JournalEntry {
        received_at: "2030-01-01T00:00:00Z".to_string(),
        event: event(TurnEventKind::StopObserved, "pending-after-reconciliation"),
    });
    write_document(&path, &mut document).expect("pending activity");
    let pending_bytes = fs::read(&path).expect("pending bytes");
    assert_eq!(
        operator_provider_turn_replay_locked(
            &context,
            &created.record.id,
            &lock,
            OperatorProviderTurnReplaySelector {
                session_incarnation: &runtime_id,
                runtime_launch_id: &runtime_id,
                runtime_generation,
            },
            "operator-provider-turn-ttl-boundary",
            "operator-provider-turn-ttl-boundary",
        )
        .expect("pending replay"),
        Some(result)
    );
    let reused = operator_provider_turn_replay_locked(
        &context,
        &created.record.id,
        &lock,
        OperatorProviderTurnReplaySelector {
            session_incarnation: &runtime_id,
            runtime_launch_id: &runtime_id,
            runtime_generation,
        },
        "operator-provider-turn-ttl-boundary",
        "changed-digest",
    )
    .expect_err("changed digest must reject");
    assert_eq!(reused.code(), "idempotency-key-reused");
    assert_eq!(
        operator_provider_turn_replay_locked(
            &context,
            &created.record.id,
            &lock,
            OperatorProviderTurnReplaySelector {
                session_incarnation: &runtime_id,
                runtime_launch_id: &runtime_id,
                runtime_generation,
            },
            "operator-provider-turn-new-key",
            "operator-provider-turn-new-key",
        )
        .expect("new key lookup"),
        None
    );
    let health_fence =
        acquire_runtime_health_fence(&context, &created.record).expect("health fence");
    let rejected = operator_reconcile_provider_turn_locked(
        &context,
        &created.record.id,
        &lock,
        &health_fence,
        OperatorProviderTurnReconcileInput {
            session_incarnation: &runtime_id,
            runtime_launch_id: &runtime_id,
            runtime_generation,
            activity_revision: document.state.revision,
            provider: &created.record.agent,
            provider_turn_id: "local:v1:unused-pending-selector",
            reason: "authoritative-completion-signal-missing",
            idempotency_key: "operator-provider-turn-new-key",
            request_digest: "operator-provider-turn-new-key",
        },
    )
    .expect_err("pending journal must reject a new key");
    assert_eq!(
        rejected.code(),
        "operator-provider-turn-reconcile-not-admissible"
    );
    assert_eq!(
        fs::read(&path).expect("activity after lookups"),
        pending_bytes
    );
}

#[test]
fn coordination_activity_lock_cannot_authorize_another_session() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let (context, alpha) = test_session(&tmp);
    let mut beta = alpha.record.clone();
    beta.id = "activity-test-beta".to_string();
    beta.tmux_session = "hs-codex-activity-test-beta".to_string();
    fs::create_dir_all(session_dir(&context, &beta.id)).expect("beta session dir");
    write_session_record(&context, &beta).expect("beta session");
    activate_runtime(&context, &alpha.record).expect("alpha activity");
    activate_runtime(&context, &beta).expect("beta activity");
    let alpha_lock =
        acquire_coordination_activity_lock(&context, &alpha.record.id).expect("alpha lock");
    let beta_path = session_dir(&context, &beta.id).join(ACTIVITY_FILE);
    let before = fs::read(&beta_path).expect("beta activity before mismatch");
    let runtime = beta.runtime.as_ref().expect("beta runtime");

    let error = operator_provider_turn_replay_locked(
        &context,
        &beta.id,
        &alpha_lock,
        OperatorProviderTurnReplaySelector {
            session_incarnation: &runtime.launch_id,
            runtime_launch_id: &runtime.launch_id,
            runtime_generation: runtime.generation,
        },
        "operator-provider-turn-lock-mismatch",
        "operator-provider-turn-lock-mismatch",
    )
    .expect_err("another session's activity lock must reject");

    assert_eq!(error.code(), "activity-lock-session-mismatch");
    let health_fence = acquire_runtime_health_fence(&context, &beta).expect("beta health fence");
    let error = operator_reconcile_provider_turn_locked(
        &context,
        &beta.id,
        &alpha_lock,
        &health_fence,
        OperatorProviderTurnReconcileInput {
            session_incarnation: &runtime.launch_id,
            runtime_launch_id: &runtime.launch_id,
            runtime_generation: runtime.generation,
            activity_revision: 1,
            provider: &beta.agent,
            provider_turn_id: "local:v1:mismatched-lock-provider-turn",
            reason: "authoritative-completion-signal-missing",
            idempotency_key: "operator-provider-turn-lock-mismatch-fresh",
            request_digest: "operator-provider-turn-lock-mismatch-fresh",
        },
    )
    .expect_err("another session's activity lock must reject a fresh reconciliation");
    assert_eq!(error.code(), "activity-lock-session-mismatch");
    assert_eq!(
        fs::read(&beta_path).expect("beta activity after mismatch"),
        before
    );
}

#[test]
fn stream_projection_exposes_only_bounded_activity_evidence_metadata() {
    let state: TurnState = serde_json::from_value(json!({
        "schema_version": TURN_STATE_VERSION,
        "phase": "needs_input",
        "phase_changed_at": "2026-07-29T00:00:00Z",
        "revision": 9,
        "source": {
            "kind": "provider_hook",
            "provider": "claude",
            "confidence": "observed"
        },
        "semantic_event": {
            "kind": "progress",
            "observed_at": "2026-07-29T00:00:04Z"
        },
        "diagnostic": {
            "reason": "completion_evidence_pending"
        },
        "shadow_observation": {
            "observer_version": "terminal-shadow.v1",
            "rule_id": "claude-working-indicator",
            "observed_at": "2026-07-29T00:00:05Z",
            "projection": "working",
            "disagrees": false,
            "terminal": "must-not-stream",
            "prompt": "must-not-stream"
        },
        "current_turn": {
            "started_at": "2026-07-29T00:00:00Z",
            "attention": {
                "kind": "approval",
                "requested_at": "2026-07-29T00:00:02Z",
                "pending_count": 1,
                "certainty": "conservative",
                "response": "must-not-stream"
            }
        },
        "provider_payload": "must-not-stream"
    }))
    .expect("forward-compatible state");

    let projection = serde_json::to_value(stream_projection(&state)).expect("projection");

    assert_eq!(projection["semantic_event"]["kind"], "progress");
    assert_eq!(
        projection["diagnostic"]["reason"],
        "completion_evidence_pending"
    );
    assert_eq!(
        projection["current_turn"]["attention"]["certainty"],
        "conservative"
    );
    assert_eq!(
        projection["shadow_observation"]["observer_version"],
        "terminal-shadow.v1"
    );
    let encoded = projection.to_string();
    for forbidden in [
        "must-not-stream",
        "\"provider_payload\":",
        "\"terminal\":",
        "\"prompt\":",
        "\"response\":",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "leaked forbidden field: {forbidden}"
        );
    }
}

#[test]
fn stream_projection_omits_unallowlisted_activity_evidence_metadata() {
    let mut state = starting_state("2026-07-29T00:00:00Z".to_string(), 1, None);
    state.semantic_event = Some(SemanticEventView {
        kind: "provider payload".to_string(),
        observed_at: "2026-07-29T00:00:01Z".to_string(),
        extra: Map::new(),
    });
    state.diagnostic = Some(ActivityDiagnosticView {
        reason: "provider secret".to_string(),
        extra: Map::new(),
    });
    state.shadow_observation = Some(ShadowObservationView {
        observer_version: "terminal-shadow.v1".to_string(),
        rule_id: "prompt content".to_string(),
        observed_at: "2026-07-29T00:00:02Z".to_string(),
        projection: "provider response".to_string(),
        disagrees: true,
        extra: Map::new(),
    });

    let projection = serde_json::to_value(stream_projection(&state)).expect("projection");

    assert!(projection.get("semantic_event").is_none());
    assert!(projection.get("diagnostic").is_none());
    assert!(projection.get("shadow_observation").is_none());
    let encoded = projection.to_string();
    assert!(!encoded.contains("provider payload"));
    assert!(!encoded.contains("provider secret"));
    assert!(!encoded.contains("prompt content"));
    assert!(!encoded.contains("provider response"));
}

#[test]
fn exact_and_uncorrelated_attention_project_distinct_certainty() {
    let exact_raw = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "AskUserQuestion",
        "tool_use_id": "exact-question"
    });
    let exact = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &exact_raw)
        .expect("exact attention")
        .expect("recognized exact attention");
    let mut exact_document = document();
    reduce(&mut exact_document, &exact, "2026-07-29T00:00:01Z");

    let conservative_raw = json!({
        "hook_event_name": "PermissionRequest",
        "tool_name": "Bash"
    });
    let conservative =
        normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &conservative_raw)
            .expect("conservative attention")
            .expect("recognized conservative attention");
    let mut conservative_document = document();
    reduce(
        &mut conservative_document,
        &conservative,
        "2026-07-29T00:00:01Z",
    );

    let exact_state =
        serde_json::to_value(stream_projection(&exact_document.state)).expect("exact state");
    let conservative_state = serde_json::to_value(stream_projection(&conservative_document.state))
        .expect("conservative state");
    assert_eq!(
        exact_state["current_turn"]["attention"]["certainty"],
        "exact"
    );
    assert_eq!(
        conservative_state["current_turn"]["attention"]["certainty"],
        "conservative"
    );
}

#[derive(Deserialize)]
struct ActivityScenarioCorpus {
    schema_version: String,
    scenarios: Vec<ActivityScenario>,
}

#[derive(Deserialize)]
struct ActivityScenario {
    id: String,
    provider: String,
    truth: String,
    events: Vec<String>,
    shadow: Option<String>,
    attention_certainty: Option<AttentionCertainty>,
    expected: ActivityScenarioExpected,
}

#[derive(Deserialize)]
struct ActivityScenarioExpected {
    phase: TurnPhase,
    diagnostic: Option<String>,
    shadow_disagrees: Option<bool>,
    false_idle: Option<bool>,
    false_working: Option<bool>,
    false_blocked: Option<bool>,
}

fn execute_activity_scenario(scenario: &ActivityScenario) -> ActivityDocument {
    let mut result = document();
    result.state = TurnState {
        schema_version: TURN_STATE_VERSION.to_string(),
        phase: TurnPhase::Unknown,
        phase_changed_at: "2026-07-10T00:00:00Z".to_string(),
        revision: 0,
        source: runtime_source(),
        semantic_event: None,
        diagnostic: None,
        shadow_observation: None,
        current_turn: None,
        last_turn: None,
        extra: Map::new(),
    };
    let mut last_progress = None;
    for (index, action) in scenario.events.iter().enumerate() {
        if action == "runtime_changed" {
            result.state = starting_state(format!("2026-07-10T00:00:{:02}Z", index + 1), 1, None);
            continue;
        }
        let kind = match action.as_str() {
            "turn_started" => TurnEventKind::TurnStarted,
            "attention_requested" => TurnEventKind::AttentionRequested,
            "progress" | "duplicate_progress" => TurnEventKind::Progress,
            "stop_observed" => TurnEventKind::StopObserved,
            "turn_completed" => TurnEventKind::TurnCompleted,
            "late_completion" => TurnEventKind::TurnCompleted,
            unknown => panic!("{} has unknown corpus event {unknown}", scenario.id),
        };
        let event_id = if action == "duplicate_progress" {
            last_progress
                .clone()
                .unwrap_or_else(|| format!("{}-progress", scenario.id))
        } else {
            format!("{}-{index}", scenario.id)
        };
        let mut input = event(kind.clone(), &event_id);
        input.provider = scenario.provider.clone();
        if kind == TurnEventKind::AttentionRequested {
            input.attention_id = Some(format!("{}-attention", scenario.id));
            input.attention_kind = Some("approval".to_string());
            input.attention_correlation_exact =
                scenario.attention_certainty == Some(AttentionCertainty::Exact);
        }
        if action == "late_completion" {
            input.provider_turn_id = Some("older-turn".to_string());
        }
        let at = format!("2026-07-10T00:00:{:02}Z", index + 1);
        reduce(&mut result, &input, &at);
        if action == "progress" {
            last_progress = Some(event_id);
        }
        result.state.diagnostic = (kind == TurnEventKind::StopObserved
            && result.state.current_turn.is_some())
        .then(|| ActivityDiagnosticView {
            reason: "completion_evidence_pending".to_string(),
            extra: Map::new(),
        });
    }
    result
}

#[test]
fn provider_activity_scenario_corpus_is_executable_content_free_and_covers_drift_risks() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/activity/provider-activity-scenarios.json"
    ))
    .expect("scenario corpus");
    assert_eq!(
        fixture["schema_version"],
        "agent-session.activity-scenarios.v1"
    );
    let scenarios = fixture["scenarios"].as_array().expect("scenario array");
    assert!(scenarios.len() >= 12);
    let ids = scenarios
        .iter()
        .filter_map(|scenario| scenario["id"].as_str())
        .collect::<Vec<_>>();
    for required in [
        "raw-stop-without-completion",
        "dropped-codex-completion",
        "generic-permission-later-progress",
        "exact-correlated-prompt",
        "nested-agent-completion",
        "background-helper-output",
        "transcript-view",
        "osc-disabled",
        "stale-osc-title",
        "runtime-reconnect",
        "duplicate-out-of-order-events",
        "provider-ui-drift",
    ] {
        assert!(ids.contains(&required), "missing scenario {required}");
    }
    let encoded = fixture.to_string();
    for forbidden in [
        "\"prompt\"",
        "\"command\"",
        "\"response\"",
        "\"terminal\"",
        "\"transcript_path\"",
        "\"credential\"",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "scenario corpus leaked a content-bearing field: {forbidden}"
        );
    }

    let corpus: ActivityScenarioCorpus =
        serde_json::from_value(fixture).expect("typed scenario corpus");
    assert_eq!(corpus.schema_version, "agent-session.activity-scenarios.v1");
    for scenario in corpus.scenarios {
        let result = execute_activity_scenario(&scenario);
        assert_eq!(
            result.state.phase, scenario.expected.phase,
            "{} phase",
            scenario.id
        );
        if let Some(expected) = scenario.expected.diagnostic.as_deref() {
            assert_eq!(
                result
                    .state
                    .diagnostic
                    .as_ref()
                    .map(|value| value.reason.as_str()),
                Some(expected),
                "{} diagnostic",
                scenario.id
            );
        }
        if let Some(expected) = scenario.expected.shadow_disagrees {
            assert_eq!(
                shadow::disagrees(
                    &result.state.phase,
                    scenario.shadow.as_deref().unwrap_or("unknown")
                ),
                expected,
                "{} shadow disagreement",
                scenario.id
            );
        }
        let false_idle = result.state.phase == TurnPhase::Waiting && scenario.truth != "waiting";
        let false_working = result.state.phase == TurnPhase::Working
            && !matches!(
                scenario.truth.as_str(),
                "working" | "working_or_unknown" | "working_with_unconfirmed_attention"
            );
        let exact_attention = result
            .state
            .current_turn
            .as_ref()
            .and_then(|turn| turn.attention.as_ref())
            .is_some_and(|attention| attention.certainty == AttentionCertainty::Exact);
        let false_blocked = result.state.phase == TurnPhase::NeedsInput
            && exact_attention
            && scenario.truth != "needs_input";
        if let Some(expected) = scenario.expected.false_idle {
            assert_eq!(false_idle, expected, "{} false idle", scenario.id);
        }
        if let Some(expected) = scenario.expected.false_working {
            assert_eq!(false_working, expected, "{} false working", scenario.id);
        }
        if let Some(expected) = scenario.expected.false_blocked {
            assert_eq!(false_blocked, expected, "{} false blocked", scenario.id);
        }
    }
}

#[test]
fn claude_rate_limit_failure_is_authoritative_and_content_free() {
    let raw = json!({
        "hook_event_name": "StopFailure",
        "session_id": "claude-session",
        "error": "rate_limit",
        "error_details": "sensitive provider detail",
        "last_assistant_message": "sensitive rendered error",
        "transcript_path": "/private/transcript.jsonl"
    });

    let event = normalize_provider_hook(AgentKind::Claude, None, "runtime-1", &raw)
        .expect("recognized failure")
        .expect("normalized event");

    assert_eq!(event.kind, TurnEventKind::TurnFailed);
    assert_eq!(event.confidence, Confidence::Authoritative);
    assert_eq!(event.failure_reason.as_deref(), Some("usage_exhausted"));

    let serialized = serde_json::to_string(&event).expect("serialize event");
    for forbidden in [
        "sensitive provider detail",
        "sensitive rendered error",
        "/private/transcript.jsonl",
        "error_details",
        "last_assistant_message",
        "transcript_path",
    ] {
        assert!(!serialized.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn auto_resume_failure_fixture_arms_only_authoritative_usage_exhaustion() {
    for line in include_str!("../../../tests/fixtures/activity/auto-resume-failures.jsonl").lines()
    {
        let mut raw: Value = serde_json::from_str(line).expect("failure fixture");
        let provider = raw
            .get("provider")
            .and_then(Value::as_str)
            .and_then(AgentKind::from_name)
            .expect("fixture provider");
        let expected = raw
            .get("expected")
            .and_then(Value::as_str)
            .map(str::to_string);
        let arms = raw.get("arms").and_then(Value::as_bool).unwrap_or(false);
        raw.as_object_mut().unwrap().remove("provider");
        raw.as_object_mut().unwrap().remove("expected");
        raw.as_object_mut().unwrap().remove("arms");
        let event = normalize_provider_hook(provider, None, "runtime-1", &raw)
            .expect("fixture normalization")
            .expect("recognized fixture");
        assert_eq!(event.failure_reason, expected);
        assert_eq!(
            event.failure_reason.as_deref() == Some("usage_exhausted")
                && event.confidence == Confidence::Authoritative,
            arms
        );
    }
}

#[test]
fn auth_loss_claude_hook_projects_failure_kind() {
    let tmp = tempfile::TempDir::new().unwrap();
    let (context, created) = test_session_for_agent(&tmp, AgentKind::Claude);
    activate_runtime(&context, &created.record).unwrap();
    let runtime_id = &created.record.runtime.as_ref().unwrap().launch_id;
    let raw = json!({"hook_event_name":"StopFailure", "error":"authentication_failed", "session_id":"session-1"});
    let failure = normalize_provider_hook(AgentKind::Claude, None, runtime_id, &raw)
        .unwrap()
        .unwrap();
    let result = ingest_event(&context, &created.record.id, failure).unwrap();
    assert_eq!(
        result.turn_state.last_turn.unwrap().provider_failure_kind(),
        Some("authentication")
    );
    let incident: Value = serde_json::from_slice(
        &fs::read(session_dir(&context, &created.record.id).join("auth-incidents.json"))
            .expect("durable auth incident"),
    )
    .unwrap();
    assert_eq!(incident["incidents"][0]["provider"], "claude");
    assert_eq!(incident["incidents"][0]["runtime_incarnation"], *runtime_id);
    assert!(!incident.to_string().contains("authentication_failed"));
}

#[test]
fn timed_activity_lock_wait_is_bounded() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let _held = acquire_lock(tmp.path()).expect("held lock");
    let started = Instant::now();
    let error = acquire_lock_with_timeout(tmp.path(), Duration::from_millis(50))
        .expect_err("timed lock must not wait forever");
    assert_eq!(error.code(), "activity-lock-timeout");
    assert!(started.elapsed() < Duration::from_secs(1));
}

fn test_session_for_agent(
    tmp: &tempfile::TempDir,
    agent: AgentKind,
) -> (CliContext, crate::CreatedRecord) {
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).expect("repo dir");
    let mut created = create_record(RecordRequest {
        context: &context,
        agent,
        mode: "interactive",
        coordination_mode: crate::cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("activity-test"),
        cwd: &cwd,
        prompt: None,
        log_file_name: None,
        provider_resume: Some(ProviderResume {
            provider: agent.as_str().to_string(),
            session_id: "session-1".to_string(),
            captured_at: "2026-07-10T00:00:00Z".to_string(),
            capture_method: "test".to_string(),
            resume_args: vec!["resume".to_string(), "session-1".to_string()],
            extra: BTreeMap::new(),
        }),
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .expect("test session");
    created.release_lifecycle_lock();
    (context, created)
}

fn test_session(tmp: &tempfile::TempDir) -> (CliContext, crate::CreatedRecord) {
    test_session_for_agent(tmp, AgentKind::Codex)
}

mod provider_hooks;
mod replay_journal;

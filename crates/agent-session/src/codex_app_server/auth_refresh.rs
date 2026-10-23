//! Same-account external-token refresh receipts. Only the official provider
//! request can initiate refresh; no prompts or tool operations are reissued.
use super::*;
use nils_common::fs::{SECRET_FILE_MODE, write_atomic};

const FILE: &str = "codex-auth-recovery.json";
const SCHEMA: &str = "agent-session.codex-auth-recovery.v1";
const MAX_OPERATIONS: usize = 64;
const WINDOW_MS: i64 = 60_000;
const DEADLINE_MS: i64 = 9_000;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    schema_version: String,
    runtime_incarnation: String,
    runtime_generation: u64,
    operations: Vec<Receipt>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    operation_id: String,
    #[serde(default)]
    connection_id: Option<String>,
    #[serde(default)]
    connection_closed: bool,
    account: String,
    binding_revision: u64,
    provider_session_id: Option<String>,
    started_at_epoch_ms: i64,
    deadline_epoch_ms: i64,
    state: String,
    reason: Option<String>,
    unknown_in_flight_effects: Vec<crate::coordination::auth_effects::UnknownOperation>,
    unknown_operation_count: usize,
    untracked_effects: String,
    replayed: bool,
}

// The guard is owned by the real control loop, so every normal/error return
// and task cancellation retires that connection. A crash without retirement
// evidence keeps its receipts conservatively; a different UUID is not proof
// that an older control connection has closed.
pub(super) struct Connection {
    id: String,
    context: CliContext,
    record: SessionRecord,
}

impl Connection {
    pub(super) fn new(context: &CliContext, record: &SessionRecord) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            context: context.clone(),
            record: record.clone(),
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.id
    }
}

fn retire(context: &CliContext, expected: &SessionRecord, id: &str) -> Result<(), String> {
    let _lock =
        crate::acquire_session_record_lock_timed(context, &expected.id, Duration::from_millis(500))
            .map_err(|_| "Codex refresh session-lock-unavailable")?;
    let record = current(context, expected)?;
    let mut store = read(context, &record)?;
    let mut changed = false;
    for receipt in &mut store.operations {
        if receipt.connection_id.as_deref() == Some(id) {
            receipt.connection_closed = true;
            changed = true;
        }
    }
    if changed {
        prune_closed(&mut store, Timestamp::now().as_millisecond());
        save(context, &record, &store)?;
    }
    Ok(())
}

impl Drop for Connection {
    fn drop(&mut self) {
        // Never block a runtime worker on a record lock: another connection
        // may hold it across an awaited credential write. Retirement is safe
        // to delay because unretired receipts are retained conservatively.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let id = self.id.clone();
        let context = self.context.clone();
        let record = self.record.clone();
        runtime.spawn_blocking(move || {
            if retire(&context, &record, &id).is_err() {
                eprintln!("warning: Codex refresh connection retirement degraded");
            }
        });
    }
}

fn prune_closed(store: &mut Store, now: i64) {
    store.operations.retain(|receipt| {
        // Retain the entire retry window even after close, and never expire
        // uncertain outcomes, live-connection identities or unscoped receipts
        // whose connection lifetime is not known.
        !(receipt.connection_closed
            && receipt.connection_id.is_some()
            && matches!(
                receipt.state.as_str(),
                "credentials_supplied" | "refresh_failed"
            )
            && now.saturating_sub(receipt.started_at_epoch_ms) >= WINDOW_MS)
    });
}

fn save(context: &CliContext, record: &SessionRecord, store: &Store) -> Result<(), String> {
    let bytes = serde_json::to_vec(store).map_err(|_| "Codex refresh receipt encode failed")?;
    if bytes.len() > 128 * 1024 {
        return Err("Codex refresh receipt capacity-exhausted".into());
    }
    write_atomic(
        &crate::session_dir(context, &record.id).join(FILE),
        &bytes,
        SECRET_FILE_MODE,
    )
    .map_err(|_| "Codex refresh receipt store-unavailable".into())
}

fn read(context: &CliContext, record: &SessionRecord) -> Result<Store, String> {
    let runtime = record
        .runtime
        .as_ref()
        .ok_or("Codex runtime identity is missing")?;
    let bytes = match fs::read(crate::session_dir(context, &record.id).join(FILE)) {
        Ok(bytes) if bytes.len() <= 128 * 1024 => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        _ => return Err("Codex refresh receipt store-unavailable".into()),
    };
    if let Some(bytes) = bytes {
        let store: Store =
            serde_json::from_slice(&bytes).map_err(|_| "Codex refresh receipt store-invalid")?;
        if store.schema_version != SCHEMA || store.operations.len() > MAX_OPERATIONS {
            return Err("Codex refresh receipt store-invalid".into());
        }
        if store.runtime_incarnation == runtime.launch_id
            && store.runtime_generation == runtime.generation
        {
            return Ok(store);
        }
    }
    Ok(Store {
        schema_version: SCHEMA.into(),
        runtime_incarnation: runtime.launch_id.clone(),
        runtime_generation: runtime.generation,
        operations: Vec::new(),
    })
}

fn current(context: &CliContext, expected: &SessionRecord) -> Result<SessionRecord, String> {
    let record = crate::load_session_record(context, &expected.id)
        .map_err(|_| "Codex refresh session-unavailable")?;
    crate::ensure_same_session_identity(expected, &record)
        .map_err(|_| "Codex refresh superseded")?;
    if !record
        .runtime
        .as_ref()
        .zip(expected.runtime.as_ref())
        .is_some_and(|(a, b)| a.launch_id == b.launch_id && a.generation == b.generation)
    {
        return Err("Codex refresh superseded".into());
    }
    Ok(record)
}

/// The caller holds the record lock. Never switch while an official refresh
/// might still deliver credentials to the old account.
pub(crate) fn failover_safe(context: &CliContext, record: &SessionRecord) -> bool {
    read(context, record).is_ok_and(|store| {
        store
            .operations
            .iter()
            .all(|r| matches!(r.state.as_str(), "refresh_failed" | "credentials_supplied"))
    })
}

fn begin(
    context: &CliContext,
    expected: &SessionRecord,
    account: &str,
    id: &Value,
    connection_id: &str,
) -> Result<(String, i64), String> {
    let _lock = crate::acquire_session_record_lock(context, &expected.id)
        .map_err(|_| "Codex refresh session-lock-unavailable")?;
    let record = current(context, expected)?;
    let binding = crate::codex_account::view_for_record(&record);
    if crate::codex_account::next_transition_state(&record)
        != crate::codex_account::NextTransitionState::Absent
    {
        return Err("Codex refresh account-switch-pending".into());
    }
    if binding.state != "bound"
        || binding.selected_account.as_deref() != Some(account)
        || binding.applied_runtime_id.as_deref()
            != record.runtime.as_ref().map(|r| r.launch_id.as_str())
    {
        return Err("Codex account refresh superseded".into());
    }
    let mut store = read(context, &record)?;
    let mut identity = Sha256::new();
    identity.update(connection_id.as_bytes());
    identity.update([0]);
    identity.update(
        json_id_key(id)
            .ok_or("Codex refresh invalid request id")?
            .as_bytes(),
    );
    let operation_id: String = identity
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if store
        .operations
        .iter()
        .any(|r| r.operation_id == operation_id)
    {
        return Err("Codex account refresh duplicate-request; response is not replayed".into());
    }
    if store
        .operations
        .iter()
        .any(|r| matches!(r.state.as_str(), "resolving" | "response_unknown"))
    {
        return Err("Codex account refresh previous-outcome-unknown; no automatic replay".into());
    }
    let now = Timestamp::now().as_millisecond();
    if store
        .operations
        .iter()
        .filter(|r| now.saturating_sub(r.started_at_epoch_ms) < WINDOW_MS)
        .count()
        >= 3
    {
        return Err("Codex account refresh retry-budget-exhausted".into());
    }
    if store.operations.last().is_some_and(|r| {
        r.state == "refresh_failed" && now.saturating_sub(r.started_at_epoch_ms) < 1000
    }) {
        return Err("Codex account refresh retry-backoff".into());
    }
    prune_closed(&mut store, now);
    if store.operations.len() == MAX_OPERATIONS {
        return Err("Codex refresh receipt capacity-exhausted".into());
    }
    let (unknown_operation_count, unknown_in_flight_effects) =
        crate::coordination::auth_effects::snapshot(context, &record)
            .map_err(|_| "Codex refresh effect-snapshot-unavailable")?;
    store.operations.push(Receipt {
        operation_id: operation_id.clone(),
        connection_id: Some(connection_id.into()),
        connection_closed: false,
        account: account.into(),
        binding_revision: binding.revision,
        provider_session_id: record
            .provider_resume
            .as_ref()
            .map(|r| r.session_id.clone()),
        started_at_epoch_ms: now,
        deadline_epoch_ms: now.saturating_add(DEADLINE_MS),
        state: "resolving".into(),
        reason: None,
        unknown_in_flight_effects,
        unknown_operation_count,
        untracked_effects: "unknown".into(),
        replayed: false,
    });
    save(context, &record, &store)?;
    Ok((operation_id, now.saturating_add(DEADLINE_MS)))
}

// Called while the existing binding/runtime fence holds the session lock.
fn mark_response_unknown(
    context: &CliContext,
    record: &SessionRecord,
    operation_id: &str,
) -> Result<(), String> {
    let latest = current(context, record)?;
    let mut store = read(context, &latest)?;
    let receipt = store
        .operations
        .iter_mut()
        .find(|r| r.operation_id == operation_id)
        .ok_or("Codex refresh receipt missing")?;
    let binding = crate::codex_account::view_for_record(&latest);
    if receipt.provider_session_id
        != latest
            .provider_resume
            .as_ref()
            .map(|r| r.session_id.clone())
        || binding.revision != receipt.binding_revision.saturating_add(1)
        || binding.selected_account.as_deref() != Some(receipt.account.as_str())
    {
        return Err("Codex refresh conversation-or-binding-superseded".into());
    }
    receipt.state = "response_unknown".into();
    save(context, record, &store)
}

fn finish(
    context: &CliContext,
    record: &SessionRecord,
    operation_id: &str,
    succeeded: bool,
) -> Result<(), String> {
    let _lock = crate::acquire_session_record_lock(context, &record.id)
        .map_err(|_| "Codex refresh session-lock-unavailable")?;
    let current = current(context, record)?;
    let mut store = read(context, &current)?;
    let receipt = store
        .operations
        .iter_mut()
        .find(|r| r.operation_id == operation_id)
        .ok_or("Codex refresh receipt missing")?;
    // A failed/timeout write may have arrived. Keep its uncertainty durable.
    if succeeded {
        receipt.state = "credentials_supplied".into();
    } else if receipt.state == "resolving" {
        receipt.state = "refresh_failed".into();
    }
    receipt.reason = Some(
        if succeeded {
            "official_retry_only"
        } else {
            "refresh_failed_no_replay"
        }
        .into(),
    );
    save(context, &current, &store)
}

pub(super) async fn respond_to_external_auth_refresh<S>(
    websocket: &mut S,
    value: &Value,
    external_auth: Option<(&CliContext, &SessionRecord, &str, &str)>,
) -> Result<bool, String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if value.get("method").and_then(Value::as_str) != Some("account/chatgptAuthTokens/refresh") {
        return Ok(false);
    }
    if value.pointer("/params/reason").and_then(Value::as_str) != Some("unauthorized") {
        return Err("Codex requested an unsupported external-auth refresh".into());
    }
    let id = value
        .get("id")
        .filter(|id| json_id_key(id).is_some())
        .ok_or("Codex external-auth refresh request omitted a valid id")?;
    let (context, record, account, connection_id) =
        external_auth.ok_or("Codex requested an external-auth refresh without a bound account")?;
    let lock_context = context.clone();
    let lock_id = record.id.clone();
    let _operation_lock = tokio::task::spawn_blocking(move || {
        crate::coordination::lock_private_store(
            &lock_context,
            &format!("codex-refresh-{lock_id}.lock"),
        )
    })
    .await
    .map_err(|_| "Codex refresh lock worker failed")?
    .map_err(|_| "Codex refresh operation-lock-unavailable")?;
    let begin_context = context.clone();
    let begin_record = record.clone();
    let begin_account = account.to_string();
    let begin_id = id.clone();
    let connection_id = connection_id.to_string();
    let (operation_id, deadline) = tokio::task::spawn_blocking(move || {
        begin(
            &begin_context,
            &begin_record,
            &begin_account,
            &begin_id,
            &connection_id,
        )
    })
    .await
    .map_err(|_| "Codex refresh receipt worker failed")??;
    let result = respond_to_external_auth_refresh_inner(
        websocket,
        value,
        external_auth,
        &operation_id,
        deadline,
    )
    .await;
    let finish_context = context.clone();
    let finish_record = record.clone();
    let succeeded = matches!(result, Ok(true));
    tokio::task::spawn_blocking(move || {
        finish(&finish_context, &finish_record, &operation_id, succeeded)
    })
    .await
    .map_err(|_| "Codex refresh receipt worker failed")??;
    let report_context = context.clone();
    let report_record = record.clone();
    if !matches!(
        tokio::task::spawn_blocking(move || crate::auth_incident::recover(
            &report_context,
            &report_record,
            if succeeded {
                "credentials_refreshed"
            } else {
                "refresh_failed"
            },
            &Timestamp::now().to_string()
        ))
        .await,
        Ok(Ok(()))
    ) {
        eprintln!("warning: provider authentication recovery reporting degraded");
    }
    result
}

async fn respond_to_external_auth_refresh_inner<S>(
    websocket: &mut S,
    value: &Value,
    external_auth: Option<(&CliContext, &SessionRecord, &str, &str)>,
    operation_id: &str,
    deadline_epoch_ms: i64,
) -> Result<bool, String>
where
    S: futures_util::Sink<Message, Error = tokio_tungstenite::tungstenite::Error> + Unpin,
{
    if value.get("method").and_then(Value::as_str) != Some("account/chatgptAuthTokens/refresh") {
        return Ok(false);
    }
    if value.pointer("/params/reason").and_then(Value::as_str) != Some("unauthorized") {
        return Err("Codex requested an unsupported external-auth refresh".to_string());
    }
    let id = value
        .get("id")
        .filter(|id| json_id_key(id).is_some())
        .cloned()
        .ok_or_else(|| "Codex external-auth refresh request omitted a valid id".to_string())?;
    let (context, record, account, _) = external_auth.ok_or_else(|| {
        "Codex requested an external-auth refresh without a bound account".to_string()
    })?;
    let launch_id = record
        .runtime
        .as_ref()
        .map(|runtime| runtime.launch_id.clone())
        .ok_or_else(|| "Codex runtime identity is missing".to_string())?;
    let incident_context = context.clone();
    let incident_record = record.clone();
    let incident_event = format!("external-refresh:{operation_id}");
    if !matches!(
        tokio::task::spawn_blocking(move || crate::auth_incident::observe(
            &incident_context,
            &incident_record,
            &incident_event,
            crate::auth_incident::AuthSource::CodexExternalRefresh,
            crate::activity::Confidence::Authoritative,
            &jiff::Timestamp::now().to_string()
        ))
        .await,
        Ok(Ok(()))
    ) {
        eprintln!("warning: provider authentication incident persistence degraded");
    }
    let begin_context = context.clone();
    let begin_id = record.id.clone();
    let begin_launch_id = launch_id.clone();
    let begin_account = account.to_string();
    let attempt = tokio::task::spawn_blocking(move || {
        crate::codex_account::begin_refresh_binding(
            &begin_context,
            &begin_id,
            &begin_launch_id,
            &begin_account,
        )
    })
    .await
    .map_err(|_| "Codex account refresh worker failed".to_string())?
    .map_err(|err| format!("Codex account refresh rejected: {}", err.code()))?;
    let revision = attempt.revision();
    let refresh_account = account.to_string();
    let credentials_result = tokio::task::spawn_blocking(move || {
        crate::codex_account::resolve_account(&refresh_account, true)
    })
    .await
    .map_err(|_| "Codex account refresh worker failed".to_string());
    let credentials = match credentials_result {
        Ok(Ok(credentials)) => credentials,
        Ok(Err(err)) => {
            let _ = restore_account_binding_after_refresh_failure(
                context, record, &launch_id, account, attempt,
            )
            .await;
            return Err(format!("Codex account refresh failed: {}", err.code()));
        }
        Err(error) => {
            let _ = restore_account_binding_after_refresh_failure(
                context, record, &launch_id, account, attempt,
            )
            .await;
            return Err(error);
        }
    };
    let expected_generation = record.runtime.as_ref().unwrap().generation;
    let fence_context = context.clone();
    let fence_id = record.id.clone();
    let fence_launch_id = launch_id.clone();
    let fence_account = account.to_string();
    let refresh_fence = tokio::task::spawn_blocking(move || {
        let lock = crate::acquire_session_record_lock(&fence_context, &fence_id)?;
        let current = crate::load_session_record(&fence_context, &fence_id)?;
        if current.runtime.as_ref().is_none_or(|runtime| {
            runtime.launch_id != fence_launch_id || runtime.generation != expected_generation
        }) {
            return Err(CliError::runtime(
                "codex-account-refresh-superseded",
                "Codex runtime changed during credential refresh",
                Some(json!({ "id": current.id })),
            ));
        }
        let view = crate::codex_account::view_for_record(&current);
        if view.state != "pending"
            || view.selected_account.as_deref() != Some(fence_account.as_str())
            || view.revision != revision
        {
            return Err(CliError::runtime(
                "codex-account-refresh-superseded",
                "Codex account binding changed during credential refresh",
                Some(json!({ "id": current.id })),
            ));
        }
        Ok::<_, CliError>(lock)
    })
    .await
    .map_err(|_| "Codex account refresh fence worker failed".to_string())?
    .map_err(|err| format!("Codex account refresh rejected: {}", err.code()))?;
    // Commit uncertainty before writing credentials to the provider. A crash
    // cannot distinguish a sent response from an unsent one and must not replay.
    if Timestamp::now().as_millisecond() >= deadline_epoch_ms {
        drop(refresh_fence);
        let _ = restore_account_binding_after_refresh_failure(
            context, record, &launch_id, account, attempt,
        )
        .await;
        return Err("Codex account refresh deadline-exceeded".into());
    }
    if let Err(error) = mark_response_unknown(context, record, operation_id) {
        drop(refresh_fence);
        let _ = restore_account_binding_after_refresh_failure(
            context, record, &launch_id, account, attempt,
        )
        .await;
        return Err(error);
    }
    let remaining = (deadline_epoch_ms - Timestamp::now().as_millisecond()).clamp(1, 500) as u64;
    let send_result = tokio::time::timeout(
        Duration::from_millis(remaining),
        send_json(
            websocket,
            external_auth_refresh_response(
                id,
                &credentials.access_token,
                &credentials.chatgpt_account_id,
                credentials.chatgpt_plan_type.as_deref(),
            ),
        ),
    )
    .await
    .unwrap_or_else(|_| Err("Codex account refresh response-outcome-unknown".into()));
    drop(refresh_fence);
    if let Err(error) = send_result {
        let _ = restore_account_binding_after_refresh_failure(
            context, record, &launch_id, account, attempt,
        )
        .await;
        return Err(error);
    }
    finish_account_binding(context, record, &launch_id, account, revision, Ok(())).await?;
    Ok(true)
}

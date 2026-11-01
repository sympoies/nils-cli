//! At-most-once prompt attempts. An unfinished durable claim is never replayed.
use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

mod client_message;
pub(crate) mod owner_commands;
mod reconcile;
mod store;
mod terminal;

pub(crate) fn history_bindings(
    state_dir: &std::path::Path,
) -> HashMap<String, HashMap<String, crate::provider_history::PromptRequestBinding>> {
    store::history_bindings(state_dir)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubmitBody {
    text: String,
    expected_session_incarnation: String,
    request_id: Option<String>,
    #[serde(default)]
    require_idle: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LookupQuery {
    expected_session_incarnation: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Submitted,
    CommandDispatched,
    Refused,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ConfirmationDiagnostic {
    AcceptanceUnobservable,
    NativeInputUnconfirmed,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Receipt {
    request_id: String,
    session_id: String,
    session_incarnation: String,
    outcome: Outcome,
    code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    composer_cleanup: Option<terminal::ComposerCleanup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmation_diagnostic: Option<ConfirmationDiagnostic>,
}

impl Receipt {
    fn unknown(session_id: String, incarnation: String, request_id: String) -> Self {
        Self {
            request_id,
            session_id,
            session_incarnation: incarnation,
            outcome: Outcome::Unknown,
            code: Some("structured-prompt-outcome-unknown".into()),
            composer_cleanup: None,
            confirmation_diagnostic: Some(ConfirmationDiagnostic::AcceptanceUnobservable),
        }
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct ResultRecord {
    receipt: Receipt,
    status: u16,
    #[serde(default)]
    queued: bool,
    // Private provider evidence used to join the accepted request to history.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    history_session_id: Option<String>,
    // Private storage only: this record is never returned as an API envelope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    native_confirmation: Option<reconcile::NativeProof>,
}

impl ResultRecord {
    fn unknown(receipt: Receipt) -> Self {
        Self {
            receipt,
            status: 500,
            queued: false,
            history_turn_id: None,
            history_session_id: None,
            native_confirmation: None,
        }
    }

    fn response(&self, machine: &str) -> Response {
        if matches!(
            self.receipt.outcome,
            Outcome::Submitted | Outcome::CommandDispatched
        ) {
            let mut data = json!({"machine":machine, "submitted":true,
                "session_incarnation":self.receipt.session_incarnation, "receipt":self.receipt});
            if self.receipt.outcome == Outcome::CommandDispatched {
                data["submitted"] = json!(false);
                data["command_dispatched"] = json!(true);
            }
            if self.queued {
                data["queued"] = json!(true);
            }
            envelope_ok(data)
        } else {
            (
                StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
                Json(json!({"schema_version":serve_schema(), "ok":false,
                    "error":{"code":self.receipt.code,
                        "message":if self.receipt.outcome == Outcome::Refused {
                            "prompt attempt was refused before delivery"
                        } else { "structured prompt submission outcome is unknown" },
                        "details":{"receipt":self.receipt}}})),
            )
                .into_response()
        }
    }
}

fn valid_incarnation(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

pub(super) async fn submit(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath(id): AxPath<String>,
    body: Result<Json<SubmitBody>, JsonRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Json(body)) = body else {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-json-body",
            "request body must contain text, expected_session_incarnation and optional request_id and require_idle",
        );
    };
    if !valid_incarnation(&body.expected_session_incarnation) {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-session-incarnation",
            "expected_session_incarnation must be a nonblank identifier of at most 128 bytes",
        );
    }
    if body
        .request_id
        .as_deref()
        .is_some_and(|id| !valid_request_id(id))
    {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-request-id",
            "request_id must contain 1 to 128 ASCII letters, digits, dots, hyphens or underscores",
        );
    }
    let Some(request_id) = body.request_id.clone() else {
        return dispatch(state, id, body).await;
    };
    let context = state.context.clone();
    let incarnation = body.expected_session_incarnation.clone();
    let fingerprint = store::fingerprint(&body.text, body.require_idle);
    let claimed = tokio::task::spawn_blocking(move || {
        // Resolve aliases before deriving the key. Do not fence against the current
        // runtime here: replay/lookup must work after runtime replacement.
        let record = load_session_record(&context, &id)?;
        let receipt = Receipt::unknown(record.id, incarnation, request_id);
        store::claim(&context, receipt, &fingerprint)
    })
    .await;
    let claim = match claimed {
        Ok(Ok(store::ClaimResult::Existing(result))) => {
            let was_bound = result.history_turn_id.is_some();
            let result = reconcile::attempt(&state.context, result).await;
            if !was_bound && result.history_turn_id.is_some() {
                state.history_catalog.invalidate();
            }
            return result.response(&state.machine);
        }
        Ok(Ok(store::ClaimResult::New(claim))) => claim,
        Ok(Err(error)) if error.code() == "request-id-conflict" => {
            return status_json(
                StatusCode::CONFLICT,
                error.code(),
                "request_id was already used with another payload",
            );
        }
        Ok(Err(error)) => return envelope_err(error),
        Err(_) => return join_err(),
    };
    let receipt = claim.receipt.clone();
    let proof = Arc::new(Mutex::new(None));
    let response = dispatch_observing(
        state.clone(),
        receipt.session_id.clone(),
        body,
        Some(proof.clone()),
    )
    .await;
    let mut result = classify(response, receipt).await;
    if result.receipt.outcome == Outcome::Unknown {
        result.native_confirmation = proof.lock().ok().and_then(|mut proof| proof.take());
        if result.native_confirmation.is_some() {
            result.receipt.confirmation_diagnostic =
                Some(ConfirmationDiagnostic::NativeInputUnconfirmed);
        }
    }
    let persisted = result.clone();
    match tokio::task::spawn_blocking(move || store::finish(claim, &persisted)).await {
        Ok(Ok(())) => {
            if result.history_turn_id.is_some() {
                state.history_catalog.invalidate();
            }
            result.response(&state.machine)
        }
        // The claim stays consumed even when publishing a result fails. Never
        // acknowledge a terminal result that lookup cannot durably recover.
        _ => ResultRecord::unknown(Receipt {
            outcome: Outcome::Unknown,
            code: Some("structured-prompt-outcome-unknown".into()),
            confirmation_diagnostic: Some(ConfirmationDiagnostic::AcceptanceUnobservable),
            ..result.receipt
        })
        .response(&state.machine),
    }
}

pub(super) async fn lookup(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    AxPath((id, request_id)): AxPath<(String, String)>,
    query: Result<Query<LookupQuery>, QueryRejection>,
) -> Response {
    if let Some(response) = deny_unauthorized(&state, &headers) {
        return response;
    }
    let Ok(Query(query)) = query else {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-query",
            "expected_session_incarnation is required",
        );
    };
    if !valid_incarnation(&query.expected_session_incarnation) || !valid_request_id(&request_id) {
        return status_json(
            StatusCode::BAD_REQUEST,
            "invalid-receipt-key",
            "invalid receipt key",
        );
    }
    let context = state.context.clone();
    match tokio::task::spawn_blocking(move || {
        let record = load_session_record(&context, &id)?;
        let receipt = Receipt::unknown(record.id, query.expected_session_incarnation, request_id);
        store::lookup(&context, &receipt)
    })
    .await
    {
        Ok(Ok(Some(result))) => {
            let was_bound = result.history_turn_id.is_some();
            let result = reconcile::attempt(&state.context, result).await;
            if !was_bound && result.history_turn_id.is_some() {
                state.history_catalog.invalidate();
            }
            envelope_ok(json!({"machine":state.machine,"receipt":result.receipt}))
        }
        Ok(Ok(None)) => status_json(
            StatusCode::NOT_FOUND,
            "prompt-receipt-not-found",
            "no receipt exists for this key",
        ),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn dispatch(state: Arc<ServeState>, id: String, body: SubmitBody) -> Response {
    dispatch_observing(state, id, body, None).await
}

async fn dispatch_observing(
    state: Arc<ServeState>,
    id: String,
    mut body: SubmitBody,
    proof: Option<Arc<Mutex<Option<reconcile::NativeProof>>>>,
) -> Response {
    // Only this bearer-authenticated conversation entry creates owner origin.
    // Force terminal guards even when the client normally sends busy replies.
    let owner_command = owner_commands::parse(&body.text);
    if let Some(command) = &owner_command {
        body.text = command.text().to_owned();
        body.require_idle = true;
    }
    if !body.require_idle {
        return structured_prompt::submit_structured_prompt_handler_for_receipt(
            state,
            id,
            body.text,
            Some(body.expected_session_incarnation),
        )
        .await;
    }
    // Match the structured route's input validation before entering the CLI
    // guard (which also supports non-prompt terminal input).
    if body.text.trim().is_empty() {
        return status_json(
            StatusCode::BAD_REQUEST,
            "empty-prompt",
            "structured prompt text must not be blank",
        );
    }
    if body.text.len() > MAX_PROVIDER_PROMPT_BYTES {
        return status_json(
            StatusCode::PAYLOAD_TOO_LARGE,
            "prompt-too-large",
            "structured prompt exceeds the maximum supported size",
        );
    }
    if structured_prompt_has_unsafe_control(&body.text) {
        return status_json(
            StatusCode::BAD_REQUEST,
            "unsafe-prompt-control",
            "structured prompt contains an unsupported control character",
        );
    }
    let record = match load_structured_prompt_record_locked(
        &state,
        &id,
        Some(&body.expected_session_incarnation),
    )
    .await
    {
        Ok(record) => record,
        Err(response) => return response,
    };
    if !crate::claude_structured_prompt_supported(&record)
        && !codex_app_server::runtime_is_supported(&record)
    {
        return status_json(
            StatusCode::CONFLICT,
            "structured-prompt-unsupported",
            "this session does not provide structured prompt submission",
        );
    }
    let fingerprint = store::fingerprint(&body.text, true);
    let history_session_id = record
        .provider_resume
        .as_ref()
        .filter(|resume| resume.provider == "codex")
        .map(|resume| resume.session_id.clone());
    // Only a claimed request has a durable identity to derive the id from.
    let client_message_id = proof
        .as_ref()
        .and(body.request_id.as_deref())
        .map(|request_id| {
            client_message::id(&record.id, &body.expected_session_incarnation, request_id)
        });
    let mut args = crate::send::unguarded_args(
        record.id,
        Some(body.text),
        vec![SpecialKey::Enter],
        false,
        state.tmux_bin.clone(),
    );
    args.guards = crate::send_guard::GuardArgs {
        expect_runtime: Some(body.expected_session_incarnation.clone()),
        expect_provider_session: owner_command.as_ref().and_then(|_| {
            record
                .provider_resume
                .as_ref()
                .map(|resume| resume.session_id.clone())
        }),
        require_idle: true,
        require_empty_composer: true,
        client_message_id,
        ..Default::default()
    };
    let context = state.context.clone();
    let observed_proof = proof.clone();
    match tokio::task::spawn_blocking(move || {
        crate::send::send_owner_prompt_observing(
            &context,
            args,
            owner_command,
            &mut |record, turn| {
                if let Some(proof) = &observed_proof
                    && let Ok(mut proof) = proof.lock()
                {
                    *proof = reconcile::NativeProof::new(record, turn, &fingerprint);
                }
            },
        )
    })
    .await
    {
        Ok(Ok(sent)) if sent.command_dispatched => envelope_ok(json!({
            "machine":state.machine, "submitted":false, "command_dispatched":true,
            "session_incarnation":body.expected_session_incarnation
        })),
        Ok(Ok(sent))
            if sent.submission.is_some_and(|s| {
                matches!(
                    s.outcome,
                    crate::send_submit::SubmitOutcome::Submitted
                        | crate::send_submit::SubmitOutcome::Queued
                )
            }) =>
        {
            let mut data = json!({"machine":state.machine,"submitted":true,
                "session_incarnation":body.expected_session_incarnation});
            if let Some(history_turn_id) = proof
                .as_ref()
                .and_then(|proof| proof.lock().ok())
                .and_then(|proof| proof.as_ref().map(|proof| proof.turn_id.clone()))
            {
                data["history_turn_id"] = json!(history_turn_id);
                if let Some(history_session_id) = history_session_id {
                    data["history_session_id"] = json!(history_session_id);
                }
            }
            if sent
                .submission
                .is_some_and(|s| s.outcome == crate::send_submit::SubmitOutcome::Queued)
            {
                data["queued"] = json!(true);
            }
            envelope_ok(data)
        }
        Ok(Err(error)) => {
            let details = error.details();
            // These CLI codes are emitted only before paste/native dispatch.
            // A guard failure at submit can already have pasted text, so it
            // must never become a safe busy refusal.
            let before_delivery = details.is_some_and(|d| {
                d["stage"] == "admission" && d["pasted"] == false && d["submitted"] == false
            });
            if before_delivery
                && matches!(
                    error.code(),
                    "send-session-not-idle"
                        | "send-provider-event-pending"
                        | "send-composer-not-empty"
                )
            {
                status_json(
                    StatusCode::CONFLICT,
                    "busy",
                    "session is busy or its composer contains a draft",
                )
            } else {
                terminal::failure(error)
            }
        }
        Ok(Ok(_)) | Err(_) => status_json(
            StatusCode::CONFLICT,
            "structured-prompt-outcome-unknown",
            "structured prompt submission outcome is unknown",
        ),
    }
}

async fn classify(response: Response, mut receipt: Receipt) -> ResultRecord {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024).await;
    let body: Value = bytes
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    if status.is_success() && body["data"]["command_dispatched"] == true {
        receipt.outcome = Outcome::CommandDispatched;
        receipt.code = None;
        receipt.confirmation_diagnostic = None;
        return ResultRecord {
            receipt,
            status: status.as_u16(),
            queued: false,
            history_turn_id: None,
            history_session_id: None,
            native_confirmation: None,
        };
    }
    if status.is_success() && body["data"]["submitted"] == true {
        receipt.outcome = Outcome::Submitted;
        receipt.code = None;
        receipt.confirmation_diagnostic = None;
        let history_turn_id = body["data"]["history_turn_id"]
            .as_str()
            .filter(|id| !id.trim().is_empty() && id.len() <= 256)
            .map(str::to_string);
        let history_session_id = body["data"]["history_session_id"]
            .as_str()
            .filter(|id| {
                !id.trim().is_empty()
                    && id.len() <= 256
                    && !id.contains(['/', '\\'])
                    && !id.contains("://")
            })
            .map(str::to_string);
        let (history_turn_id, history_session_id) =
            if history_turn_id.is_some() && history_session_id.is_some() {
                (history_turn_id, history_session_id)
            } else {
                (None, None)
            };
        return ResultRecord {
            receipt,
            status: status.as_u16(),
            queued: body["data"]["queued"] == true,
            history_turn_id,
            history_session_id,
            native_confirmation: None,
        };
    }
    let code = body["error"]["code"].as_str().unwrap_or("");
    let details = &body["error"]["details"];
    receipt.composer_cleanup = terminal::ComposerCleanup::from_details(details);
    let safe_guard_refusal = details["stage"] == "admission"
        && details["pasted"] == false
        && details["submitted"] == false;
    if safe_guard_refusal
        || matches!(
            code,
            "busy"
                | "empty-prompt"
                | "prompt-too-large"
                | "unsafe-prompt-control"
                | "session-incarnation-conflict"
                | "structured-prompt-unsupported"
                | "structured-prompt-unavailable"
                | "agent-blocked"
        )
    {
        receipt.outcome = Outcome::Refused;
        receipt.code = Some(code.to_string());
        receipt.confirmation_diagnostic = None;
        ResultRecord {
            receipt,
            status: status.as_u16(),
            queued: false,
            history_turn_id: None,
            history_session_id: None,
            native_confirmation: None,
        }
    } else {
        let mut result = ResultRecord::unknown(receipt);
        if status == StatusCode::CONFLICT {
            result.status = status.as_u16();
        }
        result
    }
}

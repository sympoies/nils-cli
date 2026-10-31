//! Loopback routes for explicitly admitted local services.
use super::*;

/// Admission documents of the services this daemon accepts; each is disabled when omitted.
#[derive(Default)]
pub(super) struct ServiceFiles {
    mailbox: Option<PathBuf>,
    console_start: Option<PathBuf>,
}
impl ServiceFiles {
    pub(super) fn from_args(args: &crate::cli::ServeArgs) -> Self {
        let file = |flag: &Option<PathBuf>, env: &str| {
            flag.clone()
                .or_else(|| crate::non_empty_env(env).map(PathBuf::from))
        };
        Self {
            mailbox: file(
                &args.mailbox_services_file,
                "AGENT_SESSION_MAILBOX_SERVICES_FILE",
            ),
            console_start: file(
                &args.console_start_services_file,
                "AGENT_SESSION_CONSOLE_START_SERVICES_FILE",
            ),
        }
    }
}

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route(
            crate::coordination::service::ROUTE,
            post(service_mailbox_handler),
        )
        .route(
            crate::coordination::console_start::service::ROUTE,
            post(service_console_start_handler),
        )
}

/// Bounds concurrent Console starts by admitted services, apart from mailbox submission.
static CONSOLE_START_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(8);

/// `POST /coordination/services/console-start/v1`: direct loopback peers only,
/// with an admitted service's own credential; never a session capability.
async fn service_console_start_handler(
    State(state): State<Arc<ServeState>>,
    request: axum::extract::Request,
) -> Response {
    use crate::coordination::console_start::service;
    let (parts, body) = request.into_parts();
    let token = match remote_session_token(&parts.headers) {
        Ok(token) if token.len() <= 256 && crate::activity_ingress::from_loopback_peer(&parts) => {
            token
        }
        _ => return envelope_err(service::unauthorized()),
    };
    let Ok(permit) = CONSOLE_START_WORKERS.try_acquire() else {
        return envelope_err(CliError::unavailable(
            "console-start-unavailable",
            "service console start workers are busy",
            None,
        ));
    };
    let body = match tokio::time::timeout(
        Duration::from_secs(5),
        axum::body::to_bytes(body, 2 * 1024 * 1024),
    )
    .await
    {
        Ok(Ok(bytes)) => serde_json::from_slice::<Value>(&bytes).ok(),
        _ => None,
    };
    let Some(body) = body else {
        return envelope_err(service::invalid_request());
    };
    let owner = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut forbidden = service_mailbox_tokens(&owner);
        if let Some(token) = owner.token.as_deref() {
            forbidden.push(token.to_owned());
        }
        if let Some(config) = owner.federation.as_ref() {
            forbidden.extend([config.token.clone(), config.ingress_token.clone()]);
        }
        let forbidden: Vec<&str> = forbidden.iter().map(String::as_str).collect();
        service::relay_route(
            &owner.context,
            owner.federation.as_ref(),
            owner.service_files.console_start.as_deref(),
            &token,
            &forbidden,
            &body,
        )
    })
    .await;
    drop(permit);
    match result {
        Ok(Ok(value)) => (StatusCode::CREATED, Json(value)).into_response(),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

/// A Console-start credential must not also authorize mailbox submission.
fn service_mailbox_tokens(state: &ServeState) -> Vec<String> {
    crate::coordination::service::admitted_tokens(state.service_files.mailbox.as_deref())
}

// Remote submission trusts only local session capability, never an operator token.
async fn service_mailbox_handler(
    State(state): State<Arc<ServeState>>,
    request: axum::extract::Request,
) -> Response {
    use crate::coordination::service;
    let (parts, body) = request.into_parts();
    if !crate::activity_ingress::from_loopback_peer(&parts) {
        return envelope_err(service::recovery(CliError::data(
            "mailbox-service-forbidden",
            "service submission accepts direct loopback connections only",
            None,
        )));
    }
    let token = match remote_session_token(&parts.headers) {
        Ok(token) if token.len() <= 256 => token,
        _ => return envelope_err(service::unauthorized()),
    };
    let permit = match state.mailbox_service_workers.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return envelope_err(service::recovery(CliError::unavailable(
                "coordination-unavailable",
                "service submission workers are busy",
                None,
            )));
        }
    };
    let bytes = match tokio::time::timeout(
        Duration::from_secs(2),
        axum::body::to_bytes(body, 6 * 16 * 1024 + 4096),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        _ => {
            return envelope_err(service::recovery(CliError::usage(
                "mailbox-service-request-invalid",
                "service request body is invalid or oversized",
                None,
            )));
        }
    };
    let args: service::Submit = match serde_json::from_slice(&bytes) {
        Ok(args) => args,
        Err(_) => {
            return envelope_err(service::recovery(CliError::usage(
                "mailbox-service-request-invalid",
                "service request body is invalid",
                None,
            )));
        }
    };
    let service_id = args.service_id.clone();
    let generation = args.service_generation.clone();
    let owner = state.clone();
    match tokio::task::spawn_blocking(move || {
        // The blocking transaction owns its slot even if the HTTP client disconnects.
        let _permit = permit;
        let mut forbidden = Vec::new();
        if let Some(token) = owner.token.as_deref() {
            forbidden.push(token);
        }
        if let Some(config) = owner.federation.as_ref() {
            forbidden.extend([config.token.as_str(), config.ingress_token.as_str()]);
        }
        service::submit(
            &owner.context,
            &owner.machine,
            owner.federation.as_ref(),
            args,
            || {
                service::authenticate(
                    owner.service_files.mailbox.as_deref(),
                    &service_id,
                    &generation,
                    &token,
                    &forbidden,
                )
            },
        )
    })
    .await
    {
        Ok(Ok(value)) => {
            state.remote_outbox_wake.notify_one();
            state.coordination_notification_wake.notify_one();
            Json(value).into_response()
        }
        Ok(Err(error)) => envelope_err(service::recovery(error)),
        Err(_) => join_err(),
    }
}

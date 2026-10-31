//! Loopback routes for explicitly admitted local services.
use super::*;

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new().route(
        crate::coordination::service::ROUTE,
        post(service_mailbox_handler),
    )
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
                    owner.mailbox_services_file.as_deref(),
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

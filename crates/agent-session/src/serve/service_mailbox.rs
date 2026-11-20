//! Local HTTP routing for credential-scoped polling and daemon-bound replies.
use super::*;
use crate::coordination::{service, service_mailbox::Operation, service_reply};
use axum::extract::DefaultBodyLimit;
use axum::http::request::Parts;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Selector {
    service_id: String,
    service_generation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxQuery {
    service_id: String,
    service_generation: String,
    state: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

pub(super) fn routes() -> Router<Arc<ServeState>> {
    Router::new()
        .route(service::ROUTE, get(inbox))
        .route("/coordination/services/messages/{message_id}/v1", get(show))
        .route(
            "/coordination/services/messages/{message_id}/ack/v1",
            post(ack),
        )
        .route(
            "/sessions/{id}/messages/{message_id}/service-reply/v1",
            post(reply).layer(DefaultBodyLimit::max(6 * 16 * 1024 + 4096)),
        )
}

async fn inbox(
    State(state): State<Arc<ServeState>>,
    parts: Parts,
    query: Result<Query<InboxQuery>, QueryRejection>,
) -> Response {
    let query = match coordination_query(query) {
        Ok(query) => query,
        Err(response) => return response,
    };
    poll(
        state,
        parts,
        Selector {
            service_id: query.service_id,
            service_generation: query.service_generation,
        },
        Operation::Inbox {
            state: query.state,
            cursor: query.cursor,
            limit: query.limit,
        },
    )
    .await
}

async fn show(
    State(state): State<Arc<ServeState>>,
    parts: Parts,
    AxPath(message): AxPath<String>,
    query: Result<Query<Selector>, QueryRejection>,
) -> Response {
    let selector = match coordination_query(query) {
        Ok(query) => query,
        Err(response) => return response,
    };
    poll(state, parts, selector, Operation::Show(message)).await
}

async fn ack(
    State(state): State<Arc<ServeState>>,
    parts: Parts,
    AxPath(message): AxPath<String>,
    query: Result<Query<Selector>, QueryRejection>,
    body: Result<Json<coordination_server::AckBody>, JsonRejection>,
) -> Response {
    let selector = match coordination_query(query) {
        Ok(query) => query,
        Err(response) => return response,
    };
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    poll(
        state,
        parts,
        selector,
        Operation::Ack {
            message,
            if_revision: body.if_revision,
            idempotency_key: body.idempotency_key,
        },
    )
    .await
}

fn local_mailbox_only(parts: &Parts) -> Result<(), CliError> {
    crate::activity_ingress::from_loopback_peer(parts)
        .then_some(())
        .ok_or_else(|| {
            CliError::data(
                "mailbox-service-forbidden",
                "mailbox service requests require direct loopback",
                None,
            )
        })
}

async fn poll(
    state: Arc<ServeState>,
    parts: Parts,
    selector: Selector,
    operation: Operation,
) -> Response {
    if let Err(error) = local_mailbox_only(&parts) {
        return envelope_err(error);
    }
    let token = match remote_session_token(&parts.headers) {
        Ok(token) => token,
        Err(_) => return envelope_err(service::unauthorized()),
    };
    let permit = match state.mailbox_service_workers.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => {
            return envelope_err(CliError::unavailable(
                "coordination-unavailable",
                "mailbox service workers are busy",
                None,
            ));
        }
    };
    let owner = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mut forbidden: Vec<&str> = owner.token.as_deref().into_iter().collect();
        if let Some(config) = &owner.federation {
            forbidden.extend([config.token.as_str(), config.ingress_token.as_str()]);
        }
        let recipient = service::Origin {
            machine: owner.machine.clone(),
            service_id: selector.service_id,
            service_generation: selector.service_generation,
        };
        crate::coordination::service_mailbox::poll(&owner.context, &recipient, operation, || {
            service::authenticate_inbox(
                owner.service_files.mailbox.as_deref(),
                &recipient.service_id,
                &recipient.service_generation,
                &token,
                &forbidden,
            )
        })
    })
    .await;
    match result {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

async fn reply(
    State(state): State<Arc<ServeState>>,
    parts: Parts,
    AxPath((id, message)): AxPath<(String, String)>,
    body: Result<Json<service_reply::Request>, JsonRejection>,
) -> Response {
    if let Err(error) = local_mailbox_only(&parts) {
        return envelope_err(error);
    }
    let token = match remote_session_token(&parts.headers) {
        Ok(token) => token,
        Err(error) => return envelope_err(error),
    };
    let body = match coordination_json(body) {
        Ok(body) => body,
        Err(response) => return response,
    };
    let owner = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        coordination_server::service_reply(
            &owner.context,
            &id,
            &message,
            &token,
            body,
            mailbox_daemon(&owner),
        )
    })
    .await;
    match result {
        Ok(Ok(value)) => coordination_ok(&state, value),
        Ok(Err(error)) => envelope_err(error),
        Err(_) => join_err(),
    }
}

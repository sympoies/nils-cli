//! Loopback activity hook ingress: `POST /activity/hook/v1` on the serve
//! daemon, plus the `activity hook --via http` client that reports to it.
//!
//! A provider hook normally writes its lifecycle metadata into the session
//! state directory. A provider whose file access is sandboxed can instead hand
//! the identical payload to the local daemon, which owns the write. The route
//! authenticates with the session's existing per-incarnation coordination
//! capability (never the operator bearer), accepts only loopback peers, and
//! shares the file path's normalization, so both transports accept exactly the
//! same payload schema and produce the same `turn_state` transition.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::activity::{self, MAX_EVENT_BYTES, ProviderHookInput};
use crate::cli::AgentKind;
use crate::coordination;
use crate::serve::{envelope_err, envelope_ok, status_json};
use crate::{CliContext, CliError};

pub(crate) const ROUTE: &str = "/activity/hook/v1";
pub(crate) const REQUEST_SCHEMA: &str = "agent-session.activity-hook.v1";
const CAPABILITY_HEADER: &str = "x-agent-session-capability";
/// A request that passed through a reverse proxy did not originate on this
/// host's loopback interface, even though the proxy's own hop did.
const FORWARDING_HEADERS: [&str; 3] = ["forwarded", "x-forwarded-for", "x-real-ip"];
/// The payload travels as a JSON string, whose escaping can expand one byte
/// into six; the provider payload limit itself is enforced after decoding.
const MAX_REQUEST_BYTES: usize = 6 * (MAX_EVENT_BYTES as usize + 1) + 4096;
/// Requests admitted to read, parse, and authenticate at once, across all
/// callers. Holding one covers the buffered body until authentication, so at
/// most this many unauthenticated bodies are ever in memory.
const MAX_ADMITTING: usize = 16;
const SESSION_BURST: f64 = 64.0;
const SESSION_REFILL_PER_SECOND: f64 = 20.0;
/// Concurrent ingests per session. Ingests of one session serialize on its
/// record lock anyway; the cap only stops a session whose lock is held
/// elsewhere from tying up an unbounded number of blocking threads.
const MAX_SESSION_IN_FLIGHT: usize = 4;
/// How long a request may wait for an admission or ingest slot. The file
/// path waits on the same locks rather than dropping the event, so admission
/// waits too, inside the client's two-second budget.
const ADMISSION_WAIT: Duration = Duration::from_millis(1500);
const MAX_TRACKED_SESSIONS: usize = 1024;
const MAX_SELECTOR_CHARS: usize = 256;
/// A hook client sends its small body at once within its own two-second
/// budget, so a slower sender only holds an admission slot this long.
const BODY_READ_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(2);
const CLIENT_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
const DAEMON_ENDPOINT_FILE: &str = "coordination/daemon-endpoint.json";

/// The request body: the file path's inputs (`--agent`, `--event`, the
/// managed runtime environment, and stdin) carried over HTTP.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HookRequest {
    schema_version: String,
    session_id: String,
    session_incarnation: String,
    agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    attention_authority: Option<String>,
    /// The exact provider hook payload text the file path reads from stdin.
    payload: String,
}

struct IngressState {
    context: CliContext,
    machine: String,
    admitting: Arc<Semaphore>,
    limiter: Arc<SessionRateLimiter>,
}

/// Routes owned by this module, merged into the serve router.
pub(crate) fn router<S>(context: CliContext, machine: String) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(ROUTE, post(hook_handler))
        .with_state(Arc::new(IngressState {
            context,
            machine,
            admitting: Arc::new(Semaphore::new(MAX_ADMITTING)),
            limiter: Arc::new(SessionRateLimiter::default()),
        }))
}

async fn hook_handler(State(state): State<Arc<IngressState>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    if !from_loopback_peer(&parts) {
        return status_json(
            StatusCode::FORBIDDEN,
            "activity-ingress-forbidden",
            "activity hook ingress accepts direct loopback connections only",
        );
    }
    let Some(capability) = capability(&parts.headers) else {
        return envelope_err(coordination::unauthorized());
    };
    let declared_length = parts
        .headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok());
    if declared_length.is_some_and(|length| length > MAX_REQUEST_BYTES) {
        return request_too_large();
    }
    // One deadline bounds every admission wait. The admission slot is taken
    // before the body is buffered and held through authentication, so
    // concurrent callers cannot buffer an unbounded number of bodies.
    let deadline = tokio::time::Instant::now() + ADMISSION_WAIT;
    let Ok(Ok(admission)) =
        tokio::time::timeout_at(deadline, state.admitting.clone().acquire_owned()).await
    else {
        return envelope_err(rate_limited());
    };
    let bytes =
        match tokio::time::timeout(BODY_READ_TIMEOUT, to_bytes(body, MAX_REQUEST_BYTES)).await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(_)) => return request_too_large(),
            Err(_) => {
                return status_json(
                    StatusCode::REQUEST_TIMEOUT,
                    "activity-hook-request-timeout",
                    "activity hook request body was not received in time",
                );
            }
        };
    let (agent, request) = match parse_request(&parts.headers, &bytes) {
        Ok(parsed) => parsed,
        Err(error) => return envelope_err(error),
    };
    match admit_and_ingest(&state, admission, deadline, capability, agent, request).await {
        Ok(ingested) => envelope_ok(json!({
            "machine": state.machine,
            "ingested": ingested,
        })),
        Err(error) => envelope_err(error),
    }
}

fn request_too_large() -> Response {
    status_json(
        StatusCode::PAYLOAD_TOO_LARGE,
        "activity-hook-request-too-large",
        "activity hook request exceeds the ingress body limit",
    )
}

/// Authenticate under the admission slot, then ingest under one of the
/// session's ingest slots, waiting for it no later than `deadline`.
async fn admit_and_ingest(
    state: &IngressState,
    admission: OwnedSemaphorePermit,
    deadline: tokio::time::Instant,
    capability: String,
    agent: AgentKind,
    request: HookRequest,
) -> Result<bool, CliError> {
    let context = state.context.clone();
    let request = Arc::new(request);
    let incarnation = {
        let request = request.clone();
        tokio::task::spawn_blocking(move || {
            let _admission = admission;
            authenticate(&context, &capability, &request)
        })
        .await
        .map_err(|_| task_failed())??
    };
    let session_slots = state
        .limiter
        .admit(&request.session_id, Instant::now())
        .ok_or_else(rate_limited)?;
    let ingest_permit = tokio::time::timeout_at(deadline, session_slots.acquire_owned())
        .await
        .map_err(|_| rate_limited())?
        .map_err(|_| rate_limited())?;
    let context = state.context.clone();
    tokio::task::spawn_blocking(move || {
        let _ingest_permit = ingest_permit;
        ingest(&context, agent, &request, &incarnation)
    })
    .await
    .map_err(|_| task_failed())?
}

fn task_failed() -> CliError {
    CliError::runtime("serve-task-failed", "internal task failed", None)
}

fn from_loopback_peer(parts: &Parts) -> bool {
    let Some(ConnectInfo(peer)) = parts.extensions.get::<ConnectInfo<SocketAddr>>() else {
        // Without transport peer information the origin is unknown.
        return false;
    };
    peer.ip().to_canonical().is_loopback()
        && !FORWARDING_HEADERS
            .iter()
            .any(|name| parts.headers.contains_key(*name))
}

fn capability(headers: &HeaderMap) -> Option<String> {
    headers
        .get(CAPABILITY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse_request(headers: &HeaderMap, bytes: &[u8]) -> Result<(AgentKind, HookRequest), CliError> {
    let invalid = || {
        CliError::usage(
            "invalid-json-body",
            "activity hook request body is invalid",
            None,
        )
    };
    let json_content = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"));
    if !json_content {
        return Err(invalid());
    }
    let request: HookRequest = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if request.schema_version != REQUEST_SCHEMA {
        return Err(CliError::usage(
            "unsupported-schema-version",
            "activity hook request schema version is unsupported",
            None,
        ));
    }
    let bounded = |value: &str| !value.is_empty() && value.chars().count() <= MAX_SELECTOR_CHARS;
    if !bounded(&request.session_id)
        || !bounded(&request.session_incarnation)
        || !request.event.as_deref().is_none_or(bounded)
        || !request.attention_authority.as_deref().is_none_or(bounded)
    {
        return Err(invalid());
    }
    let agent = AgentKind::from_name(&request.agent).ok_or_else(|| {
        CliError::usage(
            "unsupported-activity-agent",
            "activity hook request names an unsupported provider",
            None,
        )
    })?;
    Ok((agent, request))
}

fn authenticate(
    context: &CliContext,
    capability: &str,
    request: &HookRequest,
) -> Result<String, CliError> {
    let (_record, incarnation) =
        coordination::authenticate_token(context, &request.session_id, capability)?;
    // The capability already binds one incarnation; a request naming another
    // is a stale or replayed runtime and is refused the same way.
    if incarnation != request.session_incarnation {
        return Err(coordination::unauthorized());
    }
    Ok(incarnation)
}

fn ingest(
    context: &CliContext,
    agent: AgentKind,
    request: &HookRequest,
    incarnation: &str,
) -> Result<bool, CliError> {
    let result = activity::ingest_provider_hook_input(
        context,
        agent,
        request.event.as_deref(),
        ProviderHookInput {
            id: &request.session_id,
            runtime_id: incarnation,
            payload: request.payload.as_bytes(),
            attention_authority: request.attention_authority.as_deref(),
        },
    );
    match &result {
        Ok(true) => {
            activity::clear_hook_diagnostic_for(
                context,
                agent,
                &request.session_id,
                Some(incarnation),
            );
        }
        Ok(false) => {}
        Err(error) => activity::record_hook_diagnostic_for(
            context,
            agent,
            &request.session_id,
            Some(incarnation),
            error.code(),
        ),
    }
    result
}

fn rate_limited() -> CliError {
    CliError::runtime(
        "rate-limited",
        "activity hook ingress rate limit exceeded",
        None,
    )
}

/// Per-session admission: a token bucket bounds the request rate and a small
/// semaphore bounds concurrent ingests. Only authenticated requests are
/// admitted here, so a caller without the capability cannot drain another
/// session's budget; unauthenticated load is bounded by the shared
/// admission semaphore instead.
#[derive(Default)]
struct SessionRateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    refilled_at: Instant,
    ingest_slots: Arc<Semaphore>,
}

impl Bucket {
    fn refill(&mut self, now: Instant) {
        let elapsed = now
            .saturating_duration_since(self.refilled_at)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * SESSION_REFILL_PER_SECOND).min(SESSION_BURST);
        self.refilled_at = now;
    }

    /// A full, idle bucket carries no state a fresh one would not.
    fn reclaimable(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= SESSION_BURST
            && self.ingest_slots.available_permits() == MAX_SESSION_IN_FLIGHT
    }
}

impl SessionRateLimiter {
    /// Draw one token and return the session's ingest slots, or `None` when
    /// the session's rate is exhausted or no session can be tracked.
    fn admit(&self, session_id: &str, now: Instant) -> Option<Arc<Semaphore>> {
        let mut buckets = self.buckets.lock().unwrap_or_else(PoisonError::into_inner);
        if !buckets.contains_key(session_id) && buckets.len() >= MAX_TRACKED_SESSIONS {
            buckets.retain(|_, bucket| !bucket.reclaimable(now));
            if buckets.len() >= MAX_TRACKED_SESSIONS {
                return None;
            }
        }
        let bucket = buckets
            .entry(session_id.to_string())
            .or_insert_with(|| Bucket {
                tokens: SESSION_BURST,
                refilled_at: now,
                ingest_slots: Arc::new(Semaphore::new(MAX_SESSION_IN_FLIGHT)),
            });
        bucket.refill(now);
        if bucket.tokens < 1.0 {
            return None;
        }
        bucket.tokens -= 1.0;
        Some(bucket.ingest_slots.clone())
    }
}

/// `activity hook --via http`: report the hook payload to the local daemon.
/// Like the file path, this is fail-open telemetry and never blocks the
/// provider. It performs no state-directory writes, so a failure cannot be
/// recorded locally; the daemon records ingestion diagnostics itself.
pub(crate) fn forward_provider_hook_fail_open(
    context: &CliContext,
    agent: AgentKind,
    event_override: Option<&str>,
) {
    let _ = forward_provider_hook(context, agent, event_override);
}

fn forward_provider_hook(
    context: &CliContext,
    agent: AgentKind,
    event_override: Option<&str>,
) -> Result<(), CliError> {
    let Some((id, runtime_id)) = activity::provider_hook_runtime_from_env() else {
        return Ok(());
    };
    let payload = activity::read_provider_hook_stdin()?;
    let capability_file = coordination::mailbox::resolve_capability_file(None)?;
    let capability =
        coordination::read_private_text(&capability_file, 512, "coordination-unauthorized")?;
    let url = local_endpoint(context)?;
    let request = HookRequest {
        schema_version: REQUEST_SCHEMA.to_string(),
        session_id: id,
        session_incarnation: runtime_id,
        agent: agent.as_str().to_string(),
        event: event_override.map(str::to_string),
        attention_authority: std::env::var(crate::codex_app_server::ATTENTION_AUTHORITY_ENV).ok(),
        // Invalid UTF-8 is never valid JSON, so the file path rejects it as
        // `provider-hook-invalid`; an empty payload reproduces that outcome.
        payload: String::from_utf8(payload).unwrap_or_default(),
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(CLIENT_TIMEOUT)
        .connect_timeout(CLIENT_CONNECT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        // The capability must never be handed to an HTTP proxy.
        .no_proxy()
        .build()
        .map_err(|_| ingress_unavailable())?;
    client
        .post(url)
        .header(CAPABILITY_HEADER, capability)
        .json(&request)
        .send()
        .map_err(|_| ingress_unavailable())?;
    Ok(())
}

/// The daemon publishes its bound loopback URL in the private state root.
fn local_endpoint(context: &CliContext) -> Result<reqwest::Url, CliError> {
    let raw = coordination::read_private_text(
        &context.state_dir.join(DAEMON_ENDPOINT_FILE),
        4096,
        "activity-ingress-unavailable",
    )?;
    let endpoint: Value = serde_json::from_str(&raw).map_err(|_| ingress_unavailable())?;
    let url = endpoint
        .get("url")
        .and_then(Value::as_str)
        .and_then(|url| reqwest::Url::parse(url).ok())
        .ok_or_else(ingress_unavailable)?;
    let loopback = match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback()),
        None => false,
    };
    if url.scheme() != "http" || !loopback || !url.username().is_empty() || url.password().is_some()
    {
        return Err(ingress_unavailable());
    }
    url.join(ROUTE).map_err(|_| ingress_unavailable())
}

fn ingress_unavailable() -> CliError {
    CliError::runtime(
        "activity-ingress-unavailable",
        "the local activity hook ingress is unavailable",
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn session_bucket_bounds_a_burst_and_refills_over_time() {
        let limiter = SessionRateLimiter::default();
        let start = Instant::now();
        let admitted = (0..100)
            .filter(|_| limiter.admit("alpha", start).is_some())
            .count();
        assert_eq!(admitted, SESSION_BURST as usize);
        assert!(
            limiter.admit("beta", start).is_some(),
            "one session's burst must not consume another session's budget"
        );
        assert!(
            limiter
                .admit("alpha", start + Duration::from_millis(10))
                .is_none()
        );
        assert!(
            limiter
                .admit("alpha", start + Duration::from_millis(100))
                .is_some()
        );
    }

    #[tokio::test]
    async fn a_busy_session_waits_for_an_ingest_slot_instead_of_dropping_the_event() {
        let limiter = SessionRateLimiter::default();
        let start = Instant::now();
        let slots = limiter.admit("busy", start).expect("admitted");
        let held = (0..MAX_SESSION_IN_FLIGHT)
            .map(|_| slots.clone().try_acquire_owned().expect("ingest slot"))
            .collect::<Vec<_>>();
        let waiting = limiter.admit("busy", start).expect("rate budget remains");
        assert!(waiting.clone().try_acquire_owned().is_err());
        assert!(
            limiter
                .admit("other", start)
                .expect("admitted")
                .try_acquire_owned()
                .is_ok(),
            "one session's held slots never block another session"
        );
        let waiter = tokio::spawn(async move {
            tokio::time::timeout(ADMISSION_WAIT, waiting.acquire_owned())
                .await
                .is_ok()
        });
        drop(held);
        assert!(
            waiter.await.expect("waiter"),
            "a released slot admits the waiter"
        );
    }

    #[test]
    fn tracked_sessions_are_bounded_and_idle_buckets_are_reclaimed() {
        let limiter = SessionRateLimiter::default();
        let start = Instant::now();
        for index in 0..MAX_TRACKED_SESSIONS {
            assert!(limiter.admit(&format!("session-{index}"), start).is_some());
        }
        assert!(
            limiter.admit("overflow", start).is_none(),
            "an untracked session is refused while every bucket is active"
        );
        assert!(
            limiter
                .admit("overflow", start + Duration::from_secs(1))
                .is_some(),
            "refilled idle buckets are reclaimed for a new session"
        );
    }

    fn saturated_state() -> Arc<IngressState> {
        Arc::new(IngressState {
            context: CliContext {
                state_dir: std::path::PathBuf::from("/nonexistent-activity-ingress-state"),
                host: None,
            },
            machine: "test".to_string(),
            admitting: Arc::new(Semaphore::new(0)),
            limiter: Arc::new(SessionRateLimiter::default()),
        })
    }

    fn loopback_request(body: axum::body::Body, content_length: Option<usize>) -> Request {
        let mut builder = axum::http::Request::builder()
            .method("POST")
            .uri(ROUTE)
            .header(CONTENT_TYPE, "application/json")
            .header(
                CAPABILITY_HEADER,
                "unverified-capability-material-000000001",
            );
        if let Some(length) = content_length {
            builder = builder.header(CONTENT_LENGTH, length);
        }
        let mut request = builder.body(body).expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:4000".parse::<SocketAddr>().unwrap()));
        request
    }

    #[tokio::test(start_paused = true)]
    async fn bodies_are_not_buffered_without_an_admission_slot() {
        let polled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = polled.clone();
        let body = axum::body::Body::from_stream(futures_util::stream::poll_fn(move |_| {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            std::task::Poll::Ready(None::<Result<axum::body::Bytes, std::io::Error>>)
        }));
        let response = hook_handler(State(saturated_state()), loopback_request(body, None)).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(
            !polled.load(std::sync::atomic::Ordering::SeqCst),
            "the body must not be read before an admission slot is held"
        );
    }

    #[tokio::test]
    async fn an_oversized_declared_body_is_refused_before_admission() {
        let response = hook_handler(
            State(saturated_state()),
            loopback_request(axum::body::Body::empty(), Some(MAX_REQUEST_BYTES + 1)),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    fn parts(peer: Option<&str>, headers: &[(&str, &str)]) -> Parts {
        let mut builder = axum::http::Request::builder().method("POST").uri(ROUTE);
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(()).expect("request");
        if let Some(peer) = peer {
            request
                .extensions_mut()
                .insert(ConnectInfo(peer.parse::<SocketAddr>().expect("peer")));
        }
        request.into_parts().0
    }

    #[test]
    fn only_direct_loopback_peers_are_accepted() {
        assert!(from_loopback_peer(&parts(Some("127.0.0.1:4000"), &[])));
        assert!(from_loopback_peer(&parts(Some("[::1]:4000"), &[])));
        assert!(from_loopback_peer(&parts(
            Some("[::ffff:127.0.0.1]:4000"),
            &[]
        )));
        assert!(!from_loopback_peer(&parts(Some("192.0.2.10:4000"), &[])));
        assert!(!from_loopback_peer(&parts(None, &[])));
        for header in FORWARDING_HEADERS {
            assert!(
                !from_loopback_peer(&parts(Some("127.0.0.1:4000"), &[(header, "192.0.2.10")])),
                "{header} marks a proxied request"
            );
        }
    }

    #[test]
    fn request_schema_is_closed_and_bounded() {
        let headers = {
            let mut headers = HeaderMap::new();
            headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
            headers
        };
        let valid = json!({
            "schema_version": REQUEST_SCHEMA,
            "session_id": "alpha",
            "session_incarnation": "launch-alpha",
            "agent": "dsh",
            "event": "pre_llm_call",
            "payload": "{}",
        });
        let (agent, _) =
            parse_request(&headers, valid.to_string().as_bytes()).expect("valid request");
        assert_eq!(agent, AgentKind::Dsh);

        let mut long_selector = valid.clone();
        long_selector["event"] = json!("e".repeat(MAX_SELECTOR_CHARS + 1));
        let mut missing_payload = valid.clone();
        missing_payload.as_object_mut().unwrap().remove("payload");
        for (name, body) in [
            ("long selector", long_selector),
            ("missing payload", missing_payload),
        ] {
            assert_eq!(
                parse_request(&headers, body.to_string().as_bytes())
                    .expect_err(name)
                    .code(),
                "invalid-json-body",
                "{name}"
            );
        }
        assert_eq!(
            parse_request(&HeaderMap::new(), valid.to_string().as_bytes())
                .expect_err("content type")
                .code(),
            "invalid-json-body"
        );
    }
}

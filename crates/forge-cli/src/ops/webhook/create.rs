//! `org hook create` — create one organization webhook.
//!
//! The secret comes only from the environment variable named by
//! `--secret-env`. It is sent in a private request-body file (never in argv),
//! and it is never printed, logged or kept after the call. A rerun is safe: an
//! organization hook that already delivers to the same URL is refused.
use nils_common::cli_contract::{OutputFormat, schema_version_for};
use serde::Serialize;
use serde_json::{Value, json};

use super::engine::{
    Kind, REDACTED, api_call, hook_view, invalid_response, list_hooks, run_call, schema_err,
    valid_url,
};
use super::target::HookTarget;
use crate::cli::{BINARY, GlobalFlags, OrgHookCreateArgs};
use crate::envelope::emit_success;
use crate::error::ForgeError;
use crate::ops::github_write::payload_file;
use crate::provider::{Provider, ProviderContext};
use crate::rate_limit::default_runner;

#[derive(Serialize)]
struct CreatePayload {
    provider: &'static str,
    org: String,
    dry_run: bool,
    /// `created`, or `would_create` for a dry run.
    status: &'static str,
    id: Option<u64>,
    active: bool,
    events: Vec<String>,
    content_type: &'static str,
    url: String,
    /// Always the marker: a created hook always has the secret from `--secret-env`.
    secret: &'static str,
}

fn refuse(kind: &'static str, message: String) -> ForgeError {
    ForgeError::validation(schema_err(), kind, message, None)
}

fn valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn valid_event(event: &str) -> bool {
    !event.is_empty()
        && event.len() <= 64
        && event
            .bytes()
            .all(|b| b.is_ascii_lowercase() || matches!(b, b'_' | b'*'))
}

pub(crate) fn create(
    global: &GlobalFlags,
    ctx: &ProviderContext,
    org: &str,
    args: OrgHookCreateArgs,
    format: OutputFormat,
) -> Result<i32, ForgeError> {
    let target = HookTarget::Org(org.to_string());
    if !target.valid() {
        return Err(refuse(
            "target_invalid",
            "--org must be an organization login".to_string(),
        ));
    }
    if !valid_url(&args.url) {
        return Err(refuse(
            "hook_url_invalid",
            "--url must be an absolute http(s) URL without whitespace".to_string(),
        ));
    }
    if args.events.iter().any(|event| !valid_event(event)) {
        return Err(refuse(
            "hook_events_invalid",
            "--events must be comma-separated event names (lowercase letters and '_') or '*'"
                .to_string(),
        ));
    }
    if !valid_env_name(&args.secret_env) {
        return Err(refuse(
            "secret_env_invalid",
            "--secret-env must name an environment variable".to_string(),
        ));
    }
    let secret = std::env::var(&args.secret_env)
        .ok()
        .filter(|secret| !secret.is_empty())
        .ok_or_else(|| {
            refuse(
                "secret_env_missing",
                format!(
                    "environment variable {} is not set or is empty",
                    args.secret_env
                ),
            )
        })?;
    let runner = default_runner();
    if let Some(existing) = list_hooks(&runner, ctx, &target)?
        .iter()
        .find(|hook| hook.url.as_deref() == Some(args.url.as_str()))
    {
        return Err(ForgeError::validation(
            schema_err(),
            "hook_url_exists",
            "an organization hook already delivers to this URL; nothing was created",
            Some(format!("existing hook id: {}", existing.id)),
        ));
    }
    let active = !args.inactive;
    let content_type = args.content_type.as_str();
    let mut payload = CreatePayload {
        provider: Provider::GitHub.as_str(),
        org: org.to_string(),
        dry_run: global.dry_run,
        status: "would_create",
        id: None,
        active,
        events: args.events.clone(),
        content_type,
        url: args.url.clone(),
        secret: REDACTED,
    };
    if !global.dry_run {
        let body = json!({
            "name": "web",
            "config": {
                "url": args.url,
                "content_type": content_type,
                "secret": secret,
                "insecure_ssl": "0",
            },
            "events": args.events,
            "active": active,
        });
        let op = Kind::Org.op("create");
        let file = payload_file(body.to_string().as_bytes(), &op)?;
        let input = file.path().to_string_lossy().into_owned();
        let output = run_call(
            &runner,
            &api_call(
                ctx,
                target.path(""),
                &["--method", "POST", "--input", &input],
            ),
        )?;
        let created: Value =
            serde_json::from_str(output.stdout.trim()).map_err(|_| invalid_response("webhook"))?;
        let view = hook_view(&created)?;
        if view.url.as_deref() != Some(payload.url.as_str()) {
            return Err(ForgeError::runtime_failure(
                schema_err(),
                "hook_readback_mismatch",
                "the created hook reports a different URL; check the organization's hooks",
                None,
            ));
        }
        payload.status = "created";
        payload.id = Some(view.id);
        payload.active = view.active;
        payload.events = view.events;
    }
    Ok(emit_success(
        schema_version_for(BINARY, &Kind::Org.op("create"), 1),
        payload,
        format,
        |payload| {
            println!(
                "{} org hook {} {} {} events={} secret=[redacted]",
                payload.org,
                payload.id.map(|id| id.to_string()).unwrap_or_default(),
                payload.status,
                payload.url,
                payload.events.join(","),
            )
        },
    ))
}

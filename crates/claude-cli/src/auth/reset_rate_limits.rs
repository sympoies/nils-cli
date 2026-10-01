//! `claude-cli auth reset-rate-limits`: redeem at most one Claude limit reset
//! for one stored profile.
//!
//! The stored access token is read and never refreshed; an expired token is
//! reported before any request. The command re-reads the usage status with
//! the Claude Code query and User-Agent, returns `unavailable` without posting
//! when the program cannot be used, and otherwise sends exactly one `POST`
//! (never retried). For `cedar_ember` the grant id comes from that status read,
//! so grant ids never leave the host. The contract lives in
//! `docs/specs/claude-cli-auth-reset-rate-limits-json-contract-v1.md`.

use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use chrono::Utc;
use nils_common::diag_output;
use nils_common::env as shared_env;
use reqwest::blocking::Client;
use serde::Serialize;
use serde_json::{Value, json};

use super::store;
use crate::prompt_segment::client::{self, RequestFailure};
use crate::rate_limits::status::{
    self, CEDAR_EMBER, GrantedResets, JUNIPER_TIDE, SessionReset, optional_token, timestamp,
};

const SCHEMA_VERSION: &str = "claude-cli.auth.reset-rate-limits.v1";
const COMMAND: &str = "auth reset-rate-limits";
const API_BASE_URL_ENV: &str = "CLAUDE_RATE_LIMITS_API_BASE_URL";
const RESET_MAX_TIME_ENV: &str = "CLAUDE_RATE_LIMITS_RESET_MAX_TIME_SECONDS";
const DEFAULT_RESET_MAX_TIME_SECONDS: u64 = 25;
const MAX_RESET_MAX_TIME_SECONDS: u64 = 120;
const EXIT_USAGE: i32 = 64;
const EXIT_PROFILE: i32 = 1;
const EXIT_AUTH: i32 = 2;
const EXIT_PROVIDER: i32 = 3;
const OUTCOMES: &[&str] = &[
    "reset",
    "already_used",
    "not_limited",
    "cooldown",
    "ineligible",
    "unavailable",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    JuniperTide,
    CedarEmber,
}

impl Program {
    const fn id(self) -> &'static str {
        match self {
            Self::JuniperTide => JUNIPER_TIDE,
            Self::CedarEmber => CEDAR_EMBER,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ResetOptions {
    pub program: Program,
    pub request_id: Option<String>,
    pub yes: bool,
    pub output_json: bool,
    pub profile: String,
}

#[derive(Debug, Serialize)]
struct ResetResult {
    provider: &'static str,
    program: &'static str,
    outcome: &'static str,
    posted: bool,
    reason: Option<String>,
    resets_left: Option<u64>,
    next_available_at: Option<i64>,
    cooldown_until: Option<i64>,
    weekly_resets_at: Option<i64>,
}

/// The exact upstream request body; field order is the wire order.
#[derive(Serialize)]
struct ResetBody<'a> {
    program: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    grant_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<&'a str>,
}

/// A failure reported with the shared error envelope.
struct Failure {
    code: &'static str,
    message: &'static str,
    exit_code: i32,
    retryable: bool,
    reason_code: Option<&'static str>,
    next_action: &'static str,
}

const fn usage_error(
    code: &'static str,
    message: &'static str,
    next_action: &'static str,
) -> Failure {
    Failure {
        code,
        message,
        exit_code: EXIT_USAGE,
        retryable: false,
        reason_code: None,
        next_action,
    }
}

const fn auth_required(reason_code: &'static str) -> Failure {
    Failure {
        code: "claude-auth-required",
        message: "This operation requires a valid Claude sign-in for the selected profile.",
        exit_code: EXIT_AUTH,
        retryable: false,
        reason_code: Some(reason_code),
        next_action: "Refresh or re-save the profile's Claude login before retrying.",
    }
}

const fn provider_unavailable(reason_code: &'static str) -> Failure {
    Failure {
        code: "provider-unavailable",
        message: "Claude limit-reset redemption could not reach the provider.",
        exit_code: EXIT_PROVIDER,
        retryable: true,
        reason_code: Some(reason_code),
        next_action: "Retry the same logical attempt with the same request id.",
    }
}

const fn invalid_response() -> Failure {
    Failure {
        code: "invalid-provider-response",
        message: "Claude returned an invalid limit-reset response.",
        exit_code: EXIT_PROVIDER,
        retryable: false,
        reason_code: None,
        next_action: "Check the installed claude-cli compatibility before retrying.",
    }
}

const fn provider_rejected() -> Failure {
    Failure {
        code: "provider-rejected",
        message: "The provider rejected Claude limit-reset redemption for this profile.",
        exit_code: EXIT_PROVIDER,
        retryable: false,
        reason_code: Some("unknown"),
        next_action: "Resolve the account state before retrying.",
    }
}

pub fn run(options: &ResetOptions) -> i32 {
    match redeem(options) {
        Ok(Some(result)) => emit_result(options.output_json, &result),
        Ok(None) => {
            println!("Limit reset cancelled.");
            0
        }
        Err(failure) => emit_failure(options.output_json, &failure),
    }
}

fn redeem(options: &ResetOptions) -> Result<Option<ResetResult>, Failure> {
    if store::validate_profile_name(&options.profile).is_err() {
        return Err(usage_error(
            "invalid-profile-name",
            "Profile names start with a letter or digit, use [A-Za-z0-9._-], and have no .json suffix.",
            "Choose one stored profile name.",
        ));
    }
    let non_interactive = options.output_json || !io::stdin().is_terminal();
    if non_interactive && !options.yes {
        return Err(usage_error(
            "confirmation-required",
            "Non-interactive limit-reset redemption requires --yes.",
            "Repeat with --yes only after confirming the profile and program.",
        ));
    }
    let request_id = match options.request_id.as_deref() {
        Some(value) if canonical_uuid(value) => value.to_string(),
        Some(_) => {
            return Err(usage_error(
                "invalid-request-id",
                "--request-id must be a canonical lowercase UUID.",
                "Generate one UUID and reuse it for every retry of this logical attempt.",
            ));
        }
        None if non_interactive => {
            return Err(usage_error(
                "request-id-required",
                "Non-interactive limit-reset redemption requires --request-id.",
                "Generate one UUID and reuse it for every retry of this logical attempt.",
            ));
        }
        None => random_uuid().ok_or(Failure {
            code: "request-id-unavailable",
            message: "A request id could not be generated.",
            exit_code: EXIT_USAGE,
            retryable: false,
            reason_code: None,
            next_action: "Repeat with --request-id <uuid>.",
        })?,
    };

    let login = read_login(&options.profile)?;
    let base_url = api_base_url()?;
    let body =
        status::request_status(&login.access_token).map_err(|failure| request_failure(&failure))?;
    let value: Value = serde_json::from_str(&body).map_err(|_| invalid_response())?;
    if !value.is_object() {
        return Err(invalid_response());
    }
    let resets = status::parse_limit_resets(&value);

    let grant_id = match options.program {
        Program::JuniperTide => match resets.juniper_tide.as_ref() {
            Some(session) if session.available => None,
            session => return Ok(Some(session_unavailable(session))),
        },
        Program::CedarEmber => match resets
            .cedar_ember
            .as_ref()
            .and_then(|granted| granted.next_usable_grant().map(|grant| (granted, grant)))
        {
            Some((granted, grant)) if granted.available => Some(grant.id.clone()),
            _ => return Ok(Some(granted_unavailable(resets.cedar_ember.as_ref()))),
        },
    };

    if !options.yes && !confirm(options.program, resets.cedar_ember.as_ref())? {
        return Ok(None);
    }

    let body = ResetBody {
        program: options.program.id(),
        grant_id: grant_id.as_deref(),
        request_id: grant_id.as_ref().map(|_| request_id.as_str()),
    };
    let url = format!(
        "{base_url}/api/organizations/{}/reset_rate_limits",
        login.organization_uuid
    );
    let (status_code, text) = post_reset(&url, &login.access_token, &body)
        .map_err(|failure| request_failure(&failure))?;
    match status_code {
        200..=299 => parse_reset_response(options.program, &text),
        401 => Err(auth_required("auth_expired")),
        403 => Err(auth_required("permission_denied")),
        429 => Err(provider_unavailable("rate_limited")),
        500..=599 => Err(provider_unavailable("service_unavailable")),
        _ => parse_reset_response(options.program, &text).map_err(|_| provider_rejected()),
    }
    .map(Some)
}

struct Login {
    access_token: String,
    organization_uuid: String,
}

fn read_login(profile: &str) -> Result<Login, Failure> {
    let stored = store::read_profile(profile).map_err(|error| Failure {
        code: if error.code == "profile-not-found" {
            "profile-not-found"
        } else {
            "profile-invalid"
        },
        message: if error.code == "profile-not-found" {
            "The selected Claude profile was not found."
        } else {
            "The selected Claude profile is unreadable or has no access token."
        },
        exit_code: EXIT_PROFILE,
        retryable: false,
        reason_code: None,
        next_action: "Choose one stored profile, or save the login again.",
    })?;
    let access_token = store::non_empty_str(stored.oauth.get("accessToken"))
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if stored
        .expires_at_ms()
        .is_some_and(|expires_at| expires_at <= Utc::now().timestamp_millis())
    {
        return Err(auth_required("auth_expired"));
    }
    let organization_uuid = store::non_empty_str(stored.account.get("organizationUuid"))
        .filter(|org| {
            org.len() <= 64 && org.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .ok_or(Failure {
            code: "organization-unknown",
            message: "The selected Claude profile does not record its organization.",
            exit_code: EXIT_PROFILE,
            retryable: false,
            reason_code: None,
            next_action: "Save the profile again from a current Claude Code login.",
        })?
        .to_string();
    Ok(Login {
        access_token,
        organization_uuid,
    })
}

/// `CLAUDE_RATE_LIMITS_API_BASE_URL`, else the usage endpoint's origin.
fn api_base_url() -> Result<String, Failure> {
    if let Some(base) = shared_env::env_non_empty(API_BASE_URL_ENV) {
        return Ok(base.trim().trim_end_matches('/').to_string());
    }
    reqwest::Url::parse(&client::usage_endpoint())
        .ok()
        .map(|url| url.origin())
        .filter(|origin| origin.is_tuple())
        .map(|origin| origin.ascii_serialization())
        .ok_or(Failure {
            code: "endpoint-invalid",
            message: "The configured Claude usage endpoint is not a valid URL.",
            exit_code: EXIT_PROFILE,
            retryable: false,
            reason_code: None,
            next_action: "Fix CLAUDE_PROMPT_SEGMENT_ENDPOINT or set CLAUDE_RATE_LIMITS_API_BASE_URL.",
        })
}

fn post_reset(
    url: &str,
    access_token: &str,
    body: &ResetBody,
) -> Result<(u16, String), RequestFailure> {
    let payload = serde_json::to_string(body).map_err(|_| RequestFailure::Client)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(reset_max_time_seconds()))
        .build()
        .map_err(|_| RequestFailure::Client)?;
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("anthropic-beta", client::anthropic_beta())
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            client::user_agent(&status::default_user_agent()),
        )
        .header("Accept", "application/json")
        .body(payload)
        .send()
        .map_err(|error| RequestFailure::Transport {
            timeout: error.is_timeout(),
        })?;
    let code = response.status().as_u16();
    let text = response.text().map_err(|error| RequestFailure::Transport {
        timeout: error.is_timeout(),
    })?;
    Ok((code, text))
}

fn reset_max_time_seconds() -> u64 {
    shared_env::env_non_empty(RESET_MAX_TIME_ENV)
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|seconds| (1..=MAX_RESET_MAX_TIME_SECONDS).contains(seconds))
        .unwrap_or(DEFAULT_RESET_MAX_TIME_SECONDS)
}

fn request_failure(failure: &RequestFailure) -> Failure {
    match failure {
        RequestFailure::Client => provider_unavailable("service_unavailable"),
        RequestFailure::Transport { timeout: true } => provider_unavailable("timeout"),
        RequestFailure::Transport { timeout: false } => provider_unavailable("service_unavailable"),
        RequestFailure::Http { status, .. } => match status {
            401 => auth_required("auth_expired"),
            403 => auth_required("permission_denied"),
            429 => provider_unavailable("rate_limited"),
            500..=599 => provider_unavailable("service_unavailable"),
            _ => provider_rejected(),
        },
    }
}

fn parse_reset_response(program: Program, text: &str) -> Result<ResetResult, Failure> {
    let value: Value = serde_json::from_str(text).map_err(|_| invalid_response())?;
    let object = value.as_object().ok_or_else(invalid_response)?;
    let outcome = object
        .get("result")
        .and_then(Value::as_str)
        .and_then(|result| OUTCOMES.iter().find(|known| **known == result).copied())
        .ok_or_else(invalid_response)?;
    Ok(ResetResult {
        provider: "claude",
        program: program.id(),
        outcome,
        posted: true,
        reason: optional_token(object.get("reason")),
        resets_left: object.get("resets_left").and_then(Value::as_u64),
        next_available_at: timestamp(object.get("next_available_at")),
        cooldown_until: timestamp(object.get("cooldown_until")),
        weekly_resets_at: timestamp(object.get("weekly_resets_at")),
    })
}

fn session_unavailable(session: Option<&SessionReset>) -> ResetResult {
    ResetResult {
        provider: "claude",
        program: JUNIPER_TIDE,
        outcome: "unavailable",
        posted: false,
        reason: session.and_then(|session| session.ineligible_reason.clone()),
        resets_left: None,
        next_available_at: session.and_then(|session| session.next_available_at),
        cooldown_until: None,
        weekly_resets_at: session.and_then(|session| session.weekly_resets_at),
    }
}

fn granted_unavailable(granted: Option<&GrantedResets>) -> ResetResult {
    ResetResult {
        provider: "claude",
        program: CEDAR_EMBER,
        outcome: "unavailable",
        posted: false,
        reason: granted.and_then(|granted| granted.ineligible_reason.clone()),
        resets_left: granted
            .and_then(GrantedResets::next_grant)
            .map(|grant| grant.resets_left),
        next_available_at: None,
        cooldown_until: granted.and_then(|granted| granted.cooldown_until),
        weekly_resets_at: granted.and_then(|granted| granted.weekly_resets_at),
    }
}

fn confirm(program: Program, granted: Option<&GrantedResets>) -> Result<bool, Failure> {
    let question = match program {
        Program::JuniperTide => {
            "Use the weekly session reset now? It cannot be undone. [y/N] ".to_string()
        }
        Program::CedarEmber => format!(
            "Use one granted reset now ({} left on the next grant)? It cannot be undone. [y/N] ",
            granted
                .and_then(GrantedResets::next_grant)
                .map(|grant| grant.resets_left)
                .unwrap_or_default()
        ),
    };
    let io_failure = || Failure {
        code: "confirmation-failed",
        message: "The confirmation prompt could not be read.",
        exit_code: EXIT_USAGE,
        retryable: false,
        reason_code: None,
        next_action: "Repeat with --yes and --request-id from a non-interactive caller.",
    };
    print!("{question}");
    io::stdout().flush().map_err(|_| io_failure())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|_| io_failure())?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// A canonical lowercase RFC 4122 UUID with a version from 1 to 8, the
/// grammar agent-session serve also enforces for idempotency keys.
fn canonical_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(byte),
        })
        && (b'1'..=b'8').contains(&bytes[14])
        && matches!(bytes[19], b'8' | b'9' | b'a' | b'b')
}

/// A random version-4 UUID for an interactive run without `--request-id`.
fn random_uuid() -> Option<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

fn emit_result(output_json: bool, result: &ResetResult) -> i32 {
    if output_json {
        return match diag_output::emit_success_result(SCHEMA_VERSION, COMMAND, result) {
            Ok(()) => 0,
            Err(_) => 1,
        };
    }
    let granted = result.program == CEDAR_EMBER;
    let line = match (result.outcome, result.posted) {
        ("reset", _) if granted => match result.resets_left {
            Some(left) => format!("Used a granted limit reset ({left} left)."),
            None => "Used a granted limit reset.".to_string(),
        },
        ("reset", _) => "Used the weekly session reset.".to_string(),
        ("already_used", _) => "This reset was already used; nothing changed.".to_string(),
        ("not_limited", _) => "Not at a limit; no reset was used.".to_string(),
        ("cooldown", _) => "Limit resets are cooling down; no reset was used.".to_string(),
        ("ineligible", _) => "This account is not eligible for this reset.".to_string(),
        (_, false) => match result.reason.as_deref() {
            Some(reason) => format!("No limit reset is available ({reason}); nothing was sent."),
            None => "No limit reset is available; nothing was sent.".to_string(),
        },
        _ => "No limit reset is available right now.".to_string(),
    };
    println!("{line}");
    0
}

fn emit_failure(output_json: bool, failure: &Failure) -> i32 {
    if output_json {
        let _ = diag_output::emit_error(
            SCHEMA_VERSION,
            COMMAND,
            failure.code,
            failure.message,
            Some(json!({
                "retryable": failure.retryable,
                "reason_code": failure.reason_code,
                "next_action": failure.next_action,
            })),
        );
    } else {
        eprintln!("claude-cli auth reset-rate-limits: {}", failure.message);
        eprintln!("Next action: {}", failure.next_action);
    }
    failure.exit_code
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn the_reset_body_keeps_the_wire_order() {
        let body = ResetBody {
            program: CEDAR_EMBER,
            grant_id: Some("g1"),
            request_id: Some("r1"),
        };
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"program":"cedar_ember","grant_id":"g1","request_id":"r1"}"#
        );
        let body = ResetBody {
            program: JUNIPER_TIDE,
            grant_id: None,
            request_id: None,
        };
        assert_eq!(
            serde_json::to_string(&body).unwrap(),
            r#"{"program":"juniper_tide"}"#
        );
    }

    #[test]
    fn request_ids_must_be_canonical_lowercase_uuids() {
        assert!(canonical_uuid("0b5f4c1e-8d2a-4c7b-9e3f-2a1b0c9d8e7f"));
        assert!(canonical_uuid(&random_uuid().expect("random uuid")));
        for bad in [
            "0B5F4C1E-8D2A-4C7B-9E3F-2A1B0C9D8E7F",
            "0b5f4c1e-8d2a-0c7b-9e3f-2a1b0c9d8e7f",
            "0b5f4c1e8d2a4c7b9e3f2a1b0c9d8e7f",
        ] {
            assert!(!canonical_uuid(bad), "{bad}");
        }
    }
}

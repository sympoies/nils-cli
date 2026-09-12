use anyhow::Result;
use serde::Serialize;
use std::io::{self, IsTerminal, Write};
use uuid::Uuid;

use crate::diag_output;
use crate::provider_profile::CODEX_PROVIDER_PROFILE;
use crate::rate_limits;
use crate::rate_limits::client::{
    ResetCreditOutcome, ResetCreditRequest, UsageRequest, consume_reset_credit, fetch_usage,
};
use nils_common::provider_usage::ProviderUsageReason;

const SCHEMA_VERSION: &str = "codex-cli.account.reset-rate-limits.v1";
const COMMAND: &str = "account reset-rate-limits";

#[derive(Clone, Debug)]
pub struct ResetRateLimitsOptions {
    pub yes: bool,
    pub idempotency_key: Option<String>,
    pub output_json: bool,
    pub no_refresh_auth: bool,
    pub secret: Option<String>,
}

#[derive(Serialize)]
struct ResetResult {
    provider: &'static str,
    outcome: ResetCreditOutcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    windows_reset: Option<i64>,
}

#[derive(Serialize)]
struct ResetEnvelope {
    schema_version: &'static str,
    command: &'static str,
    ok: bool,
    result: ResetResult,
}

pub fn run(options: &ResetRateLimitsOptions) -> Result<i32> {
    if let Some(secret) = options.secret.as_deref()
        && !valid_secret_name(secret)
    {
        return emit_error(
            options.output_json,
            "invalid-secret-name",
            "Secret name must be one configured JSON filename without path components.",
            64,
            false,
            "Choose one configured account filename.",
        );
    }
    let non_interactive = options.output_json || !io::stdin().is_terminal();
    if non_interactive && !options.yes {
        return emit_error(
            options.output_json,
            "confirmation-required",
            "Non-interactive reset redemption requires --yes.",
            64,
            false,
            "Repeat with --yes only after confirming the account and one-time reset consumption.",
        );
    }

    let supplied_key = match options.idempotency_key.as_deref() {
        Some(value) if canonical_uuid(value) => Some(value.to_string()),
        Some(_) => {
            return emit_error(
                options.output_json,
                "invalid-idempotency-key",
                "--idempotency-key must be a canonical lowercase UUID.",
                64,
                false,
                "Generate one UUID and reuse it for every retry of this logical redemption attempt.",
            );
        }
        None if non_interactive => {
            return emit_error(
                options.output_json,
                "idempotency-key-required",
                "Non-interactive reset redemption requires --idempotency-key.",
                64,
                false,
                "Generate one UUID and reuse it for every retry of this logical redemption attempt.",
            );
        }
        None => None,
    };

    let target_file = match rate_limits::resolve_target(options.secret.as_deref()) {
        Ok(path) => path,
        Err(_) => {
            return emit_error(
                options.output_json,
                "target-not-configured",
                "No Codex account target is configured.",
                1,
                false,
                "Configure an active ChatGPT account or pass one configured secret filename.",
            );
        }
    };
    if !target_file.is_file() {
        return emit_error(
            options.output_json,
            "target-not-found",
            "The selected Codex account was not found.",
            1,
            false,
            "Choose one configured account.",
        );
    }

    let base_url = std::env::var("CODEX_CHATGPT_BASE_URL")
        .unwrap_or_else(|_| "https://chatgpt.com/backend-api/".to_string());
    let connect_timeout_seconds = env_timeout("CODEX_RATE_LIMITS_CURL_CONNECT_TIMEOUT_SECONDS", 2);
    let max_time_seconds = env_timeout("CODEX_RATE_LIMITS_CURL_MAX_TIME_SECONDS", 8);
    let refresh_on_401 = !options.no_refresh_auth
        && nils_common::env::env_truthy(CODEX_PROVIDER_PROFILE.env.auto_refresh_enabled);

    if !options.yes {
        let usage = match fetch_usage(&UsageRequest {
            target_file: target_file.clone(),
            refresh_on_401,
            suppress_auth_refresh_output: true,
            base_url: base_url.clone(),
            connect_timeout_seconds,
            max_time_seconds,
        }) {
            Ok(usage) => usage,
            Err(error) => {
                return emit_provider_error(options.output_json, error.reason());
            }
        };
        let count = rate_limits::reset_credits_available_count(&usage.json)
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        println!("Account: selected Codex account");
        println!("Earned resets available: {count}");
        print!("Consume one earned reset? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Reset redemption cancelled.");
            return Ok(0);
        }
    }

    let idempotency_key = supplied_key.unwrap_or_else(|| Uuid::new_v4().hyphenated().to_string());
    let response = match consume_reset_credit(&ResetCreditRequest {
        target_file,
        refresh_on_401,
        suppress_auth_refresh_output: true,
        base_url,
        connect_timeout_seconds,
        max_time_seconds,
        idempotency_key,
    }) {
        Ok(response) => response,
        Err(error) => return emit_provider_error(options.output_json, error.reason()),
    };

    let result = ResetResult {
        provider: "codex",
        outcome: response.code,
        windows_reset: response.windows_reset,
    };
    if options.output_json {
        diag_output::emit_json(&ResetEnvelope {
            schema_version: SCHEMA_VERSION,
            command: COMMAND,
            ok: true,
            result,
        })?;
    } else {
        match result.outcome {
            ResetCreditOutcome::Reset => println!(
                "Consumed an earned reset ({} window{} reset).",
                result.windows_reset.unwrap_or(0),
                if result.windows_reset == Some(1) {
                    ""
                } else {
                    "s"
                }
            ),
            ResetCreditOutcome::AlreadyRedeemed => {
                println!("This reset attempt was already redeemed; no duplicate was consumed.")
            }
            ResetCreditOutcome::NothingToReset => {
                println!("No eligible rate-limit window can currently be reset.")
            }
            ResetCreditOutcome::NoCredit => {
                println!("No earned reset is available.")
            }
        }
    }
    Ok(0)
}

fn canonical_uuid(value: &str) -> bool {
    Uuid::parse_str(value).is_ok_and(|parsed| parsed.hyphenated().to_string() == value)
}

fn valid_secret_name(value: &str) -> bool {
    let Some(stem) = value.strip_suffix(".json") else {
        return false;
    };
    !stem.is_empty()
        && !stem.chars().any(char::is_control)
        && !stem.contains('/')
        && !stem.contains('\\')
        && !stem.contains("..")
}

fn env_timeout(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or(default)
}

fn emit_provider_error(output_json: bool, reason: ProviderUsageReason) -> Result<i32> {
    let (code, message, retryable, next_action, exit_code) = match reason {
        ProviderUsageReason::AuthRequired | ProviderUsageReason::AuthExpired => (
            "chatgpt-auth-required",
            "This operation requires a valid ChatGPT-authenticated Codex account.",
            false,
            "Refresh or select a ChatGPT account before retrying.",
            2,
        ),
        ProviderUsageReason::RateLimited
        | ProviderUsageReason::Timeout
        | ProviderUsageReason::ServiceUnavailable => (
            "provider-unavailable",
            "Codex reset redemption could not reach the provider.",
            true,
            "Retry the same logical attempt with the same idempotency key.",
            3,
        ),
        ProviderUsageReason::BillingPastDue
        | ProviderUsageReason::SubscriptionInactive
        | ProviderUsageReason::OrganizationDisabled
        | ProviderUsageReason::PermissionDenied => (
            "provider-rejected",
            "The provider rejected Codex reset redemption for this account.",
            false,
            "Resolve the account or subscription state before retrying.",
            3,
        ),
        _ => (
            "invalid-provider-response",
            "Codex returned an invalid reset-redemption response.",
            false,
            "Check the installed codex-cli compatibility before retrying.",
            3,
        ),
    };
    emit_error(
        output_json,
        code,
        message,
        exit_code,
        retryable,
        next_action,
    )
}

fn emit_error(
    output_json: bool,
    code: &str,
    message: &str,
    exit_code: i32,
    retryable: bool,
    next_action: &str,
) -> Result<i32> {
    if output_json {
        diag_output::emit_error(
            SCHEMA_VERSION,
            COMMAND,
            code,
            message,
            Some(serde_json::json!({
                "retryable": retryable,
                "next_action": next_action,
                "recovery": {
                    "retry_same_idempotency_key": retryable,
                }
            })),
        )?;
    } else {
        eprintln!("codex-cli account reset-rate-limits: {message}");
        eprintln!("Next action: {next_action}");
    }
    Ok(exit_code)
}

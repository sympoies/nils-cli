//! Fixed, metadata-only classification for Claude Code StopFailure hooks.

/// The typed activity reason for a recognized OAuth revocation.
pub const OAUTH_REVOKED_REASON: &str = "authentication_revoked";

/// Recognize only known provider revocation messages.
pub fn is_oauth_revocation(message: &str) -> bool {
    let message = message.trim();
    let message = message.strip_prefix("API Error: ").unwrap_or(message);
    matches!(
        message,
        "OAuth token revoked · Please run /login" | "OAuth access token has been revoked"
    )
}

/// Project a Claude StopFailure to a fixed activity reason.
pub fn normalize(error: Option<&str>, message: Option<&str>) -> &'static str {
    match error {
        Some("rate_limit") => "usage_exhausted",
        Some("authentication_failed") if message.is_some_and(is_oauth_revocation) => {
            OAUTH_REVOKED_REASON
        }
        Some("authentication_failed") => "authentication",
        Some("oauth_org_not_allowed") => "organization",
        Some("billing_error") => "billing",
        Some("invalid_request") => "invalid_request",
        Some("server_error") => "service",
        Some("max_output_tokens") => "max_output_tokens",
        _ => "unknown",
    }
}

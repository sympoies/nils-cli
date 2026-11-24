//! Fixed Claude authentication evidence used to bound late publication recovery.
use super::AuthIncident;
use crate::{
    SessionRecord, restart::recorded_provider_version, send::claude_notice::version::Version,
};

pub(crate) const FAILURE_REASON: &str = "authentication_revoked";
pub(super) const WAIT_SECONDS: i64 = 120;

pub(crate) fn matches_message(message: &str) -> bool {
    let message = message.trim();
    let message = message.strip_prefix("API Error: ").unwrap_or(message);
    matches!(
        message,
        "OAuth token revoked · Please run /login" | "OAuth access token has been revoked"
    )
}

pub(super) fn can_wait_for_publication(incident: &AuthIncident, record: &SessionRecord) -> bool {
    // 2.1.248 documents retryable contention on Claude's native refresh lock.
    // Unknown versions keep the broker-publication path available.
    incident.oauth_revoked
        && recorded_provider_version(record)
            .and_then(Version::parse_banner)
            .is_none_or(|version| version >= Version::parse("2.1.248").unwrap())
}

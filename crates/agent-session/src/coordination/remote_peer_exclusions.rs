//! Relay exclusions explain discovery refusals without granting delivery authority.
use serde_json::{Value, json};
use std::fmt::Write;

use crate::CliError;

pub(super) fn recipient_error(snapshot: &Value, machine: &str, session: &str) -> Option<CliError> {
    let excluded = snapshot["excluded"]
        .as_array()?
        .iter()
        .find(|row| row["machine"] == machine && row["session_id"] == session)?;
    let reason = excluded["reason"].as_str()?;
    let mut details = json!({"machine":machine, "session_id":session, "reason":reason});
    let mut message = format!(
        "remote recipient is excluded from authorized peer discovery (reason: {})",
        reason.escape_debug()
    );
    if reason == "ownership-binding-stale" {
        message.push_str(
            "; recovery requires owner-authorized `import-owner --claim-session --expected-incarnation` (check first, then apply)",
        );
        details["recovery"] = json!({
            "kind":"owner-authorized",
            "command":"import-owner --claim-session --expected-incarnation",
            "sequence":["check", "apply"]
        });
    } else {
        message.push_str("; inspect the exclusion reason using message peers");
    }
    Some(CliError::data(
        "remote-recipient-excluded",
        message,
        Some(details),
    ))
}

pub(super) fn render_text(snapshot: &Value, excluded: &[Value]) -> String {
    let mut text = format!(
        "Authorized peers:\n{}\nExcluded peers:\n",
        serde_json::to_string_pretty(&snapshot["peers"]).unwrap_or_else(|_| "[]".into())
    );
    for row in excluded {
        // Keep relay text inert in a terminal while preserving the raw JSON fields.
        let field = |name| row[name].as_str().unwrap_or("").escape_debug().to_string();
        let _ = writeln!(
            text,
            "excluded {} {} {}",
            field("machine"),
            field("session_id"),
            field("reason")
        );
    }
    text
}

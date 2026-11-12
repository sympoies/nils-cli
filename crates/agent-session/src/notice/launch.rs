//! Launch-owned notice receipts and first-row history binding.
use crate::send::claude_notice::{Launch, sha256};
use crate::{CliContext, CliError, SessionRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

const FILE: &str = "launch-notice.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    launch: Launch,
    provider_session_id: String,
    kind: String,
    source: String,
    bytes: usize,
    sha256: String,
}

pub(crate) fn save(
    context: &CliContext,
    record: &SessionRecord,
    text: &str,
) -> Result<(), CliError> {
    let parsed = super::parse_notice(text).ok_or_else(|| {
        CliError::runtime("launch-notice-invalid", "invalid initial notice", None)
    })?;
    let provider = record
        .provider_resume
        .as_ref()
        .filter(|p| p.provider == "claude" && p.capture_method == "claude-explicit-session-id")
        .ok_or_else(|| {
            CliError::runtime(
                "launch-notice-unavailable",
                "initial notice binding unavailable",
                None,
            )
        })?;
    let receipt = Receipt {
        launch: Launch::from_record(record)?,
        provider_session_id: provider.session_id.clone(),
        kind: parsed.metadata.kind().into(),
        source: parsed.metadata.source().into(),
        bytes: text.len(),
        sha256: sha256(text.as_bytes()),
    };
    let bytes = serde_json::to_vec(&receipt).map_err(|_| {
        CliError::runtime(
            "launch-notice-unavailable",
            "initial notice binding unavailable",
            None,
        )
    })?;
    crate::write_private_file(&crate::session_dir(context, &record.id).join(FILE), &bytes)
}

fn unwrap_paste(text: &str) -> Option<&str> {
    if super::parse_notice(text).is_some() {
        return Some(text);
    }
    let text = text.strip_prefix("\n\n<pasted_content id=\"")?;
    let (id, rest) = text.split_once("\">\n")?;
    if id.len() != 4 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let suffix = format!("</pasted_content id=\"{id}\">\n");
    let inner = rest.strip_suffix(&suffix)?;
    super::parse_notice(inner)?;
    Some(inner)
}

pub(crate) fn matches_row(value: &Value, text: &str) -> bool {
    value["type"] == "user"
        && value["message"]["role"] == "user"
        && value["message"]["content"].as_str().and_then(unwrap_paste) == Some(text)
}

/// Claude repeats the launch text in `last-prompt` and queued-input rows.
pub(crate) fn echoes_notice(value: &Value, text: &str) -> bool {
    let echoed = match value["type"].as_str() {
        Some("last-prompt") => value["lastPrompt"].as_str(),
        Some("queue-operation") if value["operation"] == "enqueue" => value["content"].as_str(),
        _ => None,
    };
    echoed.is_some_and(|echoed| unwrap_paste(echoed) == Some(text) || truncated_echo(echoed, text))
}

/// Claude's display echo flattens newlines to spaces and cuts long text to
/// about 200 characters plus `…`, so only a prefix of the notice survives. The
/// prefix must cover the whole header line to count as the bound notice.
fn truncated_echo(echoed: &str, text: &str) -> bool {
    let Some(prefix) = echoed.strip_suffix('…') else {
        return false;
    };
    prefix.contains("-->") && text.replace('\n', " ").starts_with(prefix)
}

pub(crate) fn bound_row(
    binding: &(u64, String, String),
    offset: u64,
    line: &[u8],
    value: &Value,
) -> bool {
    binding.0 == offset
        && sha256(crate::provider_prompt::native_notice::payload(line)) == binding.2
        && matches_row(value, &binding.1)
}

/// Locate an exact launch receipt and the first user row, independently of paging direction.
pub(crate) fn binding(
    state: &Path,
    session: &crate::provider_history::HistorySession,
    deadline: std::time::Instant,
) -> Option<(u64, String, String)> {
    binding_for_source(
        state,
        &session.provider,
        &session.provider_session_id,
        session.agent_profile.as_deref(),
        &session.transcript_path,
        deadline,
    )
}

pub(crate) fn binding_for_source(
    state: &Path,
    provider: &str,
    provider_session_id: &str,
    agent_profile: Option<&str>,
    transcript_path: &Path,
    deadline: std::time::Instant,
) -> Option<(u64, String, String)> {
    if provider != "claude" {
        return None;
    }
    let context = CliContext {
        state_dir: state.to_path_buf(),
        host: None,
    };
    let mut found = None;
    for (index, entry) in std::fs::read_dir(state.join("sessions")).ok()?.enumerate() {
        if index >= 10_000 || std::time::Instant::now() >= deadline {
            return None;
        }
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_dir() {
            continue;
        }
        let Ok(bytes) =
            crate::send::claude_notice::runtime::private_read(&entry.path().join(FILE), 4096)
        else {
            continue;
        };
        let Ok(receipt) = serde_json::from_slice::<Receipt>(&bytes) else {
            continue;
        };
        if receipt.provider_session_id != provider_session_id {
            continue;
        }
        let id = entry.file_name();
        let id = id.to_str()?;
        let Ok(record) = crate::load_session_record(&context, id) else {
            continue;
        };
        if record.agent != "claude"
            || crate::session_agent_profile(&record) != agent_profile
            || record.provider_resume.as_ref().is_none_or(|p| {
                p.provider != "claude"
                    || p.session_id != provider_session_id
                    || p.capture_method != "claude-explicit-session-id"
            })
        {
            continue;
        }
        if receipt.launch != Launch::from_record(&record).ok()? {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(receipt);
    }
    let receipt = found?;
    use std::io::{BufRead, Read};
    let file = std::fs::File::open(transcript_path).ok()?;
    let mut reader = std::io::BufReader::new(file.take(1024 * 1024 + 1));
    let mut line = Vec::new();
    let mut offset = 0u64;
    loop {
        line.clear();
        if std::time::Instant::now() >= deadline
            || reader.read_until(b'\n', &mut line).ok()? == 0
            || line.len() > 1024 * 1024
            || offset > 1024 * 1024
            || !line.ends_with(b"\n")
        {
            return None;
        }
        let value: Value = serde_json::from_slice(&line).ok()?;
        if value["type"] == "user" && value["message"]["role"] == "user" {
            if value["sessionId"].as_str()? != provider_session_id
                || value["isSidechain"].as_bool() != Some(false)
            {
                return None;
            }
            let text = value["message"]["content"].as_str()?;
            let inner = unwrap_paste(text)?;
            let parsed = super::parse_notice(inner)?;
            if inner.len() != receipt.bytes
                || sha256(inner.as_bytes()) != receipt.sha256
                || parsed.metadata.kind() != receipt.kind
                || parsed.metadata.source() != receipt.source
            {
                return None;
            }
            return Some((
                offset,
                inner.to_owned(),
                sha256(crate::provider_prompt::native_notice::payload(&line)),
            ));
        }
        offset += line.len() as u64;
    }
}

#[cfg(test)]
mod tests {
    use super::unwrap_paste;

    #[test]
    fn accepts_only_exact_single_paste_envelope() {
        let notice = "<!-- agent-notice:v1 kind=example source=fixture -->\n\nbody\n";
        let wrapped =
            format!("\n\n<pasted_content id=\"abcd\">\n{notice}</pasted_content id=\"abcd\">\n");
        assert_eq!(unwrap_paste(notice), Some(notice));
        assert_eq!(unwrap_paste(&wrapped), Some(notice));
        for changed in [
            wrapped.replace(
                "</pasted_content id=\"abcd\">",
                "</pasted_content id=\"abce\">",
            ),
            format!("prefix{wrapped}"),
            format!("{wrapped}extra"),
            format!("\n\n<pasted_content id=\"abcd\">\n{wrapped}</pasted_content id=\"abcd\">\n"),
        ] {
            assert_eq!(unwrap_paste(&changed), None);
        }
    }
}

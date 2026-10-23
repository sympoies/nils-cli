//! Unexpected provider exits are diagnostic metadata, never restart authority.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum Reason {
    ProviderExited,
    AppServerClose,
    RuntimeDisappeared,
    RuntimeStopped,
}

#[derive(Debug, Serialize)]
pub(crate) struct RuntimeExit {
    code: Option<u8>,
    signal: Option<u8>,
    reason: Reason,
    unexpected: bool,
    observed_at: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    recovery: Option<Resume>,
}

#[derive(Debug, Serialize)]
struct Resume {
    label: &'static str,
    command: String,
    method: &'static str,
    path: String,
}

impl RuntimeExit {
    fn with_message(mut self) -> Self {
        self.message = match self.reason {
            Reason::AppServerClose => "Provider app-server connection closed".to_string(),
            Reason::ProviderExited => "Provider exited".to_string(),
            Reason::RuntimeDisappeared => "Managed runtime disappeared".to_string(),
            Reason::RuntimeStopped => "Managed runtime stopped by request".to_string(),
        };
        if let Some(code) = self.code {
            self.message.push_str(&format!(" (status {code})"));
        }
        if let Some(signal) = self.signal {
            self.message.push_str(&format!(" (signal {signal})"));
        }
        self
    }

    fn offer_resume(&mut self, id: &str) {
        self.recovery = Some(Resume {
            label: "Resume",
            command: format!("agent-session resume {id}"),
            method: "POST",
            path: format!("/sessions/{id}/resume"),
        });
    }
}

pub(super) fn observed_exit(context: &CliContext, target: &SessionRecord, known: bool) -> Value {
    let code = crate::read_bounded_startup_marker(
        &crate::session_dir(context, &target.id).join(crate::RUNTIME_EXIT_STATUS_FILE),
    )
    .and_then(|value| value.parse::<u8>().ok());
    json!({
        "code": code, "signal": null,
        "reason": if known { "runtime-stopped" } else if code.is_some() { "provider-exited" } else { "runtime-disappeared" },
        "stopped_by": if known { Some("agent-session") } else { None },
    })
}

/// The control owner supplies the exact snapshot it connected to. A late close
/// therefore cannot overwrite the new runtime's reason after a concurrent resume.
pub(crate) fn record_control_close(context: &CliContext, target: &SessionRecord, error: &str) {
    eprintln!("warning: Codex app-server control ended: {error}");
    super::record(
        context,
        Some(target),
        &target.id,
        "provider-close",
        "controller",
        Ok(()),
        Some(json!({"code":null,"signal":null,"reason":"app-server-close","stopped_by":null})),
    );
}

pub(crate) fn projection(
    context: &CliContext,
    target: &SessionRecord,
    status: &str,
) -> Option<RuntimeExit> {
    if status != "stopped" {
        return None;
    }
    let runtime = target.runtime.as_ref()?;
    super::observe_stopped(context, target);
    let journal = super::read(context, &target.id, 1000).ok()?;
    let rows: Vec<_> = journal["records"]
        .as_array()?
        .iter()
        .filter(|row| {
            row["session_incarnation"].as_str() == Some(runtime.launch_id.as_str())
                && row["session_generation"].as_u64() == Some(runtime.generation)
        })
        .collect();
    let row = rows
        .iter()
        .rev()
        .find(|row| row["operation"] == "exit-observed")?;
    let explicit_stop = rows.iter().any(|row| {
        row["operation"] == "stop"
            && row["result"]["ok"] == true
            && matches!(row["caller"]["kind"].as_str(), Some("cli" | "serve"))
    });
    let code = row["exit"]["code"]
        .as_u64()
        .and_then(|value| u8::try_from(value).ok());
    let signal = row["exit"]["signal"]
        .as_u64()
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| *value > 0 && *value <= 64);
    let reason = if explicit_stop {
        Reason::RuntimeStopped
    } else if code.is_some() || signal.is_some() {
        // Direct process evidence takes precedence over an older control close.
        Reason::ProviderExited
    } else if rows.iter().any(|row| {
        row["operation"] == "provider-close" && row["exit"]["reason"] == "app-server-close"
    }) {
        Reason::AppServerClose
    } else {
        Reason::RuntimeDisappeared
    };
    let mut exit = RuntimeExit {
        code,
        signal,
        reason,
        unexpected: !explicit_stop,
        observed_at: row["timestamp"].as_str()?.to_owned(),
        message: String::new(),
        recovery: None,
    }
    .with_message();
    if exit.unexpected && crate::is_resumable(target) {
        exit.offer_resume(&target.id);
    }
    Some(exit)
}

/// Reconstruct only audited fields; never copy arbitrary provider text or a
/// caller-supplied recovery command into the federated board.
pub(crate) fn board_projection(value: Option<&Value>, id: &str) -> Option<Value> {
    let value = value?;
    crate::validate_id(id).ok()?;
    let mut exit = RuntimeExit {
        code: value["code"].as_u64().and_then(|v| u8::try_from(v).ok()),
        signal: value["signal"]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .filter(|v| *v > 0 && *v <= 64),
        reason: serde_json::from_value(value["reason"].clone()).ok()?,
        unexpected: value["unexpected"].as_bool()?,
        observed_at: value["observed_at"]
            .as_str()?
            .parse::<jiff::Timestamp>()
            .ok()?
            .to_string(),
        message: String::new(),
        recovery: None,
    }
    .with_message();
    if exit.unexpected && value.get("recovery").is_some_and(Value::is_object) {
        exit.offer_resume(id);
    }
    serde_json::to_value(exit).ok()
}

pub(crate) fn list_line(view: &crate::SessionView) -> String {
    let mut text = format!("{}  {}  {}  {}", view.id, view.agent, view.status, view.cwd);
    if let Some(exit) = &view.runtime_exit {
        text.push_str(&format!("\n  {}", exit.message));
        if let Some(resume) = &exit.recovery {
            text.push_str(&format!("\n  {}: {}", resume.label, resume.command));
        }
    }
    text
}

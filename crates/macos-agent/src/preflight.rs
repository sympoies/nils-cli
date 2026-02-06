use std::env;
use std::path::PathBuf;
use std::process::Command;

use serde_json::{json, Value};

pub const CLICLICK_INSTALL_HINT: &str = "Install cliclick with Homebrew: brew install cliclick";
pub const ACCESSIBILITY_HINT: &str = "Open System Settings > Privacy & Security > Accessibility, then enable your terminal app (Terminal, iTerm, or other shell host).";
pub const AUTOMATION_HINT: &str = "Open System Settings > Privacy & Security > Automation, then allow your terminal app to control System Events.";
pub const SCREEN_RECORDING_HINT: &str = "Advisory: if screenshot commands fail, open System Settings > Privacy & Security > Screen Recording and enable your terminal app.";

const ACCESSIBILITY_SCRIPT: &str = r#"tell application "System Events" to get UI elements enabled"#;
const AUTOMATION_SCRIPT: &str = r#"tell application "System Events" to get name of first application process whose frontmost is true"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Ready,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionSignal {
    pub state: PermissionState,
    pub detail: String,
}

impl PermissionSignal {
    pub fn ready(detail: impl Into<String>) -> Self {
        Self {
            state: PermissionState::Ready,
            detail: detail.into(),
        }
    }

    pub fn blocked(detail: impl Into<String>) -> Self {
        Self {
            state: PermissionState::Blocked,
            detail: detail.into(),
        }
    }

    pub fn unknown(detail: impl Into<String>) -> Self {
        Self {
            state: PermissionState::Unknown,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeSnapshot {
    pub osascript_path: Option<String>,
    pub cliclick_path: Option<String>,
    pub accessibility_signal: PermissionSignal,
    pub automation_signal: PermissionSignal,
    pub screen_recording_signal: PermissionSignal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Fail,
    Warn,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Fail => "fail",
            Self::Warn => "warn",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    pub id: &'static str,
    pub label: &'static str,
    pub status: CheckStatus,
    pub blocking: bool,
    pub message: String,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreflightSummary {
    pub ok: bool,
    pub blocking_failures: usize,
    pub warnings: usize,
}

impl PreflightSummary {
    pub fn status(self) -> &'static str {
        if self.ok {
            "ready"
        } else {
            "not_ready"
        }
    }

    fn title(self) -> &'static str {
        if self.ok {
            "ready"
        } else {
            "not ready"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreflightReport {
    pub strict: bool,
    pub checks: Vec<CheckReport>,
}

impl PreflightReport {
    pub fn summary(&self) -> PreflightSummary {
        let blocking_failures = self
            .checks
            .iter()
            .filter(|check| check.blocking && check.status == CheckStatus::Fail)
            .count();
        let warnings = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Warn)
            .count();
        let ok = if self.strict {
            blocking_failures == 0 && warnings == 0
        } else {
            blocking_failures == 0
        };

        PreflightSummary {
            ok,
            blocking_failures,
            warnings,
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn check(&self, id: &str) -> Option<&CheckReport> {
        self.checks.iter().find(|check| check.id == id)
    }
}

pub fn collect_system_snapshot() -> ProbeSnapshot {
    let osascript_path = find_in_path("osascript");
    let cliclick_path = find_in_path("cliclick");
    let osascript_available = osascript_path.is_some();

    let accessibility_signal = if osascript_available {
        probe_accessibility()
    } else {
        PermissionSignal::unknown("Skipped because osascript is missing.")
    };
    let automation_signal = if osascript_available {
        probe_automation()
    } else {
        PermissionSignal::unknown("Skipped because osascript is missing.")
    };
    let screen_recording_signal = PermissionSignal::unknown(
        "Advisory only. Screen Recording is validated when observe screenshot runs.",
    );

    ProbeSnapshot {
        osascript_path,
        cliclick_path,
        accessibility_signal,
        automation_signal,
        screen_recording_signal,
    }
}

pub fn build_report(snapshot: ProbeSnapshot, strict: bool) -> PreflightReport {
    let checks = vec![
        tool_check(
            "osascript",
            "osascript",
            snapshot.osascript_path,
            true,
            None,
        ),
        tool_check(
            "cliclick",
            "cliclick",
            snapshot.cliclick_path,
            true,
            Some(CLICLICK_INSTALL_HINT),
        ),
        permission_check(
            "accessibility",
            "Accessibility",
            snapshot.accessibility_signal,
            true,
            ACCESSIBILITY_HINT,
        ),
        permission_check(
            "automation",
            "Automation",
            snapshot.automation_signal,
            true,
            AUTOMATION_HINT,
        ),
        permission_check(
            "screen_recording",
            "Screen Recording",
            snapshot.screen_recording_signal,
            false,
            SCREEN_RECORDING_HINT,
        ),
    ];

    PreflightReport { strict, checks }
}

pub fn render_text(report: &PreflightReport) -> String {
    let summary = report.summary();
    let mut lines = Vec::with_capacity(2 + report.checks.len() * 2);
    lines.push(format!(
        "preflight: {} (strict={})",
        summary.title(),
        report.strict
    ));
    lines.push(format!(
        "blocking_failures: {}, warnings: {}",
        summary.blocking_failures, summary.warnings
    ));

    for check in &report.checks {
        lines.push(format!(
            "- [{}] {}: {}",
            check.status.as_str(),
            check.label,
            check.message
        ));
        if let Some(hint) = &check.hint {
            lines.push(format!("  hint: {hint}"));
        }
    }

    lines.join("\n")
}

pub fn render_json(report: &PreflightReport) -> Value {
    let summary = report.summary();
    let checks = report
        .checks
        .iter()
        .map(|check| {
            json!({
                "id": check.id,
                "label": check.label,
                "status": check.status.as_str(),
                "blocking": check.blocking,
                "message": check.message,
                "hint": check.hint,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "schema_version": 1,
        "ok": summary.ok,
        "command": "preflight",
        "result": {
            "strict": report.strict,
            "status": summary.status(),
            "summary": {
                "blocking_failures": summary.blocking_failures,
                "warnings": summary.warnings,
            },
            "checks": checks,
        }
    })
}

fn tool_check(
    id: &'static str,
    label: &'static str,
    path: Option<String>,
    blocking: bool,
    missing_hint: Option<&str>,
) -> CheckReport {
    match path {
        Some(path) => CheckReport {
            id,
            label,
            status: CheckStatus::Ok,
            blocking,
            message: format!("found at {path}"),
            hint: None,
        },
        None => CheckReport {
            id,
            label,
            status: CheckStatus::Fail,
            blocking,
            message: "not found in PATH".to_string(),
            hint: missing_hint.map(str::to_string),
        },
    }
}

fn permission_check(
    id: &'static str,
    label: &'static str,
    signal: PermissionSignal,
    blocking: bool,
    guidance: &'static str,
) -> CheckReport {
    match signal.state {
        PermissionState::Ready => CheckReport {
            id,
            label,
            status: CheckStatus::Ok,
            blocking,
            message: signal.detail,
            hint: None,
        },
        PermissionState::Blocked => CheckReport {
            id,
            label,
            status: CheckStatus::Fail,
            blocking,
            message: signal.detail,
            hint: Some(guidance.to_string()),
        },
        PermissionState::Unknown => CheckReport {
            id,
            label,
            status: CheckStatus::Warn,
            blocking,
            message: signal.detail,
            hint: Some(guidance.to_string()),
        },
    }
}

fn probe_accessibility() -> PermissionSignal {
    let output = run_osascript(ACCESSIBILITY_SCRIPT);
    if output.success {
        let value = output.stdout.trim().to_ascii_lowercase();
        return match value.as_str() {
            "true" => PermissionSignal::ready("System Events reports UI scripting is enabled."),
            "false" => PermissionSignal::blocked("System Events reports UI scripting is disabled."),
            _ => PermissionSignal::unknown(format!(
                "Accessibility probe returned unexpected value: {}",
                sanitize_probe_detail(&output.stdout)
            )),
        };
    }

    let normalized = output.normalized_detail();
    if looks_like_accessibility_blocked(&normalized) {
        PermissionSignal::blocked("Accessibility access is blocked for this terminal host.")
    } else if looks_like_automation_blocked(&normalized) {
        PermissionSignal::unknown(
            "Could not confirm Accessibility because Automation access to System Events is blocked.",
        )
    } else {
        PermissionSignal::unknown(format!(
            "Accessibility probe failed: {}",
            sanitize_probe_detail(&normalized)
        ))
    }
}

fn probe_automation() -> PermissionSignal {
    let output = run_osascript(AUTOMATION_SCRIPT);
    if output.success {
        return PermissionSignal::ready("Apple Events access to System Events is allowed.");
    }

    let normalized = output.normalized_detail();
    if looks_like_automation_blocked(&normalized) {
        PermissionSignal::blocked("Apple Events access to System Events is blocked.")
    } else {
        PermissionSignal::unknown(format!(
            "Automation probe failed: {}",
            sanitize_probe_detail(&normalized)
        ))
    }
}

fn find_in_path(bin: &str) -> Option<String> {
    if bin.contains(std::path::MAIN_SEPARATOR) {
        let path = PathBuf::from(bin);
        if path.is_file() {
            return Some(path.display().to_string());
        }
        return None;
    }

    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(bin))
        .find(|candidate| candidate.is_file())
        .map(|candidate| candidate.display().to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OsaScriptOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

impl OsaScriptOutput {
    fn normalized_detail(&self) -> String {
        let merged = format!("{} {}", self.stdout, self.stderr);
        sanitize_probe_detail(&merged).to_ascii_lowercase()
    }
}

fn run_osascript(script: &str) -> OsaScriptOutput {
    match Command::new("osascript").args(["-e", script]).output() {
        Ok(output) => OsaScriptOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        },
        Err(err) => OsaScriptOutput {
            success: false,
            stdout: String::new(),
            stderr: err.to_string(),
        },
    }
}

fn sanitize_probe_detail(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        "no detail available".to_string()
    } else {
        collapsed
    }
}

fn looks_like_automation_blocked(message: &str) -> bool {
    message.contains("-1743") || message.contains("not authorized to send apple events")
}

fn looks_like_accessibility_blocked(message: &str) -> bool {
    message.contains("-25211")
        || message.contains("assistive access")
        || message.contains("ui scripting")
            && (message.contains("not allowed") || message.contains("permission"))
}

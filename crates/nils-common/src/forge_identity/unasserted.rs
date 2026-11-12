//! Signal for non-interactive runs that reach a configured `asserted-only` policy without
//! asserting a principal, so they would otherwise silently use the host's default `gh` account.
use super::{Error, Result, append_audit};
use serde::Deserialize;
use std::io::IsTerminal;
use std::sync::atomic::{AtomicBool, Ordering};

/// Environment variables whose presence marks a run as automation regardless of TTYs.
const AUTOMATION_ENV: [&str; 3] = ["INVOCATION_ID", "GITHUB_ACTIONS", "AGENT_SESSION_ID"];
pub const FIX_TEXT: &str = "set FORGE_IDENTITY_PRINCIPAL to a configured principal, or set the identity policy key unasserted_noninteractive = \"allow\" to keep the shared default account on purpose";

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Allow,
    #[default]
    Warn,
    Deny,
}

/// A run is non-interactive without a TTY on stdin or stdout, or under a known automation marker.
pub fn is_noninteractive_with(
    stdin_tty: bool,
    stdout_tty: bool,
    env_set: impl Fn(&str) -> bool,
) -> bool {
    !stdin_tty || !stdout_tty || AUTOMATION_ENV.iter().any(|name| env_set(name))
}
pub fn is_noninteractive() -> bool {
    is_noninteractive_with(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
        |name| std::env::var_os(name).is_some(),
    )
}

static NOTIFIED: AtomicBool = AtomicBool::new(false);
/// Let a test process observe the once-per-process notice again.
#[doc(hidden)]
pub fn reset_notice_for_tests() {
    NOTIFIED.store(false, Ordering::SeqCst);
}

/// Bounded, secret-free process label: base name only, restricted charset.
fn label(raw: &str) -> String {
    let base = raw.rsplit('/').next().unwrap_or("");
    let clean: String = base
        .chars()
        .take(64)
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if clean.is_empty() {
        "unknown".into()
    } else {
        clean
    }
}
fn parent_name() -> String {
    // SAFETY: getppid has no preconditions and cannot fail.
    let ppid = unsafe { libc::getppid() };
    if let Ok(comm) = std::fs::read_to_string(format!("/proc/{ppid}/comm")) {
        return label(comm.trim());
    }
    std::process::Command::new("ps")
        .args(["-o", "comm=", "-p", &ppid.to_string()])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| label(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or_else(|| "unknown".into())
}
fn argv0() -> String {
    label(
        &std::env::args_os()
            .next()
            .map(|a| a.to_string_lossy().into_owned())
            .unwrap_or_default(),
    )
}

pub(super) fn gate(mode: Mode) -> Result<()> {
    if mode == Mode::Allow || !is_noninteractive() {
        return Ok(());
    }
    let outcome = if mode == Mode::Deny {
        "unasserted_noninteractive_denied"
    } else {
        "unasserted_noninteractive_warned"
    };
    if mode == Mode::Warn && NOTIFIED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let record = serde_json::json!({
        "schema_version": "forge.identity.audit.v1",
        "timestamp": jiff::Timestamp::now().to_string(),
        "principal": null,
        "target": null,
        "outcome": outcome,
        "caller": {"argv0": argv0(), "parent": parent_name()},
    });
    if mode == Mode::Deny {
        append_audit(&record)?;
        return Err(Error::with_detail(
            "identity_unasserted_noninteractive",
            FIX_TEXT.into(),
        ));
    }
    eprintln!(
        "warning: forge identity: no principal asserted in a non-interactive run; {FIX_TEXT}"
    );
    // A warning must not fail the run; an unwritable audit trail only loses the record.
    let _ = append_audit(&record);
    Ok(())
}

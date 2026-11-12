//! Signal for non-interactive runs that reach a configured `asserted-only` policy without
//! asserting a principal, so they would otherwise silently use the host's default `gh` account.
use super::{Error, Result, append_audit, state_home};
use serde::Deserialize;
use std::io::{IsTerminal, Write};
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

/// At most one audit record per caller (argv0 and parent) per window, so scheduled
/// automation that runs unasserted many times an hour writes a bounded number of lines.
const AUDIT_WINDOW: std::time::Duration = std::time::Duration::from_secs(3600);
fn stamp_name(argv0: &str, parent: &str) -> String {
    format!("{argv0}.{parent}")
}
fn within_window(age: std::time::Duration) -> bool {
    age < AUDIT_WINDOW
}
/// True when this caller has no record inside the window; claims the window when it does.
fn claim_audit_window(argv0: &str, parent: &str) -> bool {
    let Some(state) = state_home() else {
        return true;
    };
    claim_in(&state.join("forge-cli/unasserted-callers"), argv0, parent)
}
fn claim_in(dir: &std::path::Path, argv0: &str, parent: &str) -> bool {
    let stamp = dir.join(stamp_name(argv0, parent));
    let fresh = std::fs::metadata(&stamp)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(within_window);
    if fresh {
        return false;
    }
    let _ = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&stamp, b""));
    true
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
    let (argv0, parent) = (argv0(), parent_name());
    let audit = claim_audit_window(&argv0, &parent);
    let record = serde_json::json!({
        "schema_version": "forge.identity.audit.v1",
        "timestamp": jiff::Timestamp::now().to_string(),
        "principal": null,
        "target": null,
        "outcome": outcome,
        "caller": {"argv0": argv0, "parent": parent},
    });
    if mode == Mode::Deny {
        if audit {
            append_audit(&record)?;
        }
        return Err(Error::with_detail(
            "identity_unasserted_noninteractive",
            FIX_TEXT.into(),
        ));
    }
    // A closed stderr pipe must not fail (or panic) the run.
    let _ = writeln!(
        std::io::stderr(),
        "warning: forge identity: no principal asserted in a non-interactive run; {FIX_TEXT}"
    );
    // A warning must not fail the run; an unwritable audit trail only loses the record.
    if audit {
        let _ = append_audit(&record);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn audit_window_covers_one_hour() {
        assert!(within_window(Duration::from_secs(59 * 60)));
        assert!(!within_window(Duration::from_secs(61 * 60)));
    }
    #[test]
    fn each_caller_pair_has_its_own_stamp() {
        let all = [
            stamp_name("forge-cli", "systemd"),
            stamp_name("forge-cli", "bash"),
            stamp_name("git-cli", "systemd"),
        ];
        assert_eq!(
            all.iter().collect::<std::collections::BTreeSet<_>>().len(),
            3
        );
    }
    #[test]
    fn the_stamp_path_keeps_callers_apart_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        assert!(claim_in(dir.path(), "forge-cli", "bash"));
        assert!(!claim_in(dir.path(), "forge-cli", "bash"));
        assert!(claim_in(dir.path(), "forge-cli", "systemd"));
        assert!(claim_in(dir.path(), "git-cli", "bash"));
    }
}

//! Bounded tmux batches reduce cleanup subprocess round trips.
use crate::{PANE_OBSERVATION_MAX_OUTPUT_BYTES, run_output_with_timeout_and_strict_cap};
use std::{path::Path, process::Command, time::Duration};

pub(super) const IDENTITY_KEYS: [&str; 3] = [
    "AGENT_SESSION_ID",
    "AGENT_SESSION_STATE_DIR",
    "AGENT_SESSION_RUNTIME_ID",
];

pub(super) fn environment(tmux: &Path, session: &str, timeout: Duration) -> Option<[String; 3]> {
    let mut command = Command::new(tmux);
    for (i, key) in IDENTITY_KEYS.iter().enumerate() {
        if i > 0 {
            command.arg(";");
        }
        command.args(["show-environment", "-t", session, key]);
    }
    let output = run_output_with_timeout_and_strict_cap(command, timeout, 16384).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let rows: Vec<_> = text.lines().collect();
    if rows.len() != 3 {
        return None;
    }
    let mut values = Vec::new();
    for (row, key) in rows.into_iter().zip(IDENTITY_KEYS) {
        values.push(row.strip_prefix(&format!("{key}="))?.to_string());
    }
    values.try_into().ok()
}

pub(super) fn pane_and_cursor(
    tmux: &Path,
    target: &str,
    timeout: Duration,
) -> Option<(String, (usize, usize))> {
    let mut command = Command::new(tmux);
    command.args([
        "capture-pane",
        "-p",
        "-e",
        "-N",
        "-T",
        "-t",
        target,
        ";",
        "display-message",
        "-p",
        "-t",
        target,
        "#{cursor_x} #{cursor_y}",
    ]);
    let output =
        run_output_with_timeout_and_strict_cap(command, timeout, PANE_OBSERVATION_MAX_OUTPUT_BYTES)
            .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let (pane, cursor) = text.trim_end_matches('\n').rsplit_once('\n')?;
    let mut coordinates = cursor.split_whitespace();
    let x = coordinates.next()?.parse().ok()?;
    let y = coordinates.next()?.parse().ok()?;
    if coordinates.next().is_some() {
        return None;
    }
    Some((pane.to_string(), (x, y)))
}

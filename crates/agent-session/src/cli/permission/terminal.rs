//! Screen admission for external Claude versions outside the 2.1.317 hook audit.
use std::{fs::File, path::Path};

use crate::{CliContext, CliError, SessionRecord, SpecialKey, activity};
use serde_json::json;

/// Retain the ingress fence across the immediate write. The caller holds the
/// record lock; no provider version result or pane observation is cached.
pub(crate) fn input_admission(
    context: &CliContext,
    record: &SessionRecord,
    tmux: &Path,
    text: Option<&str>,
    keys: &[SpecialKey],
) -> Result<Option<File>, CliError> {
    let ingress = super::terminal_admission(context, record)?;
    if record.agent != "claude" || !activity::claude_attention::has_permission(context, record)? {
        return Ok(ingress);
    }
    // The version admission above permits only positively identified releases
    // outside Claude Code 2.1.317. A batch could outlive the dialog after its
    // first Enter; arbitrary text could edit tool input or save a rule.
    let key = input_key(text, keys)?;
    let target = format!("{}:0.0", record.tmux_session);
    // Share the existing strict timeout, byte-cap and UTF-8 capture owner.
    let pane = crate::send_guard::capture_styled(tmux, &target, crate::PANE_OBSERVATION_COMMAND_TIMEOUT)
    .filter(|pane| visible_dialog(pane))
    .ok_or_else(|| CliError::runtime(
        "permission-dialog-unobservable",
        "Claude permission dialog is not visible; inspect the attached terminal before retrying",
        None,
    ))?;
    // Keep the observation only in memory: it can contain tool arguments.
    drop(pane);
    crate::write_private_file(
        &crate::session_dir(context, &record.id).join("permission-terminal-admission.json"),
        &serde_json::to_vec(&json!({
            "schema_version": "agent-session.permission-terminal-admission.v1",
            "runtime_id": record.runtime.as_ref().map(|r| &r.launch_id),
            "reason": "permission-provider-version-unsupported",
            "key": key.as_str(),
            "checked_at": jiff::Timestamp::now().to_string(),
        }))
        .expect("terminal admission serializes"),
    )?;
    Ok(ingress)
}

pub(super) fn input_key(text: Option<&str>, keys: &[SpecialKey]) -> Result<SpecialKey, CliError> {
    match (text, keys) {
        (
            None,
            [key @ (SpecialKey::Enter | SpecialKey::Escape | SpecialKey::Up | SpecialKey::Down)],
        ) => Ok(*key),
        (Some(text), []) if crate::text_is_bare_newline(text) => Ok(SpecialKey::Enter),
        _ => Err(CliError::runtime(
            "permission-terminal-input-unsupported",
            "Claude permission input requires one Enter, Escape, Up or Down key, or a bare newline alone",
            None,
        )),
    }
}

fn visible_dialog(pane: &str) -> bool {
    let plain =
        nils_common::shell::strip_ansi(pane, nils_common::shell::AnsiStripMode::CsiAnyTerminator);
    let rows: Vec<_> = plain
        .lines()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .collect();
    let Some(start) = rows
        .iter()
        .rposition(|r| *r == "Do you want to proceed?" || *r == "Allow this action?")
    else {
        return false;
    };
    let dialog = &rows[start + 1..];
    // Claude Code 2.1.325 can draw the terminal title rule below the dialog.
    let dialog = if dialog.last().is_some_and(|r| r.starts_with('─')) {
        &dialog[..dialog.len() - 1]
    } else {
        dialog
    };
    // A historical permission frame above a composer is not a live dialog.
    let choices: Vec<_> = dialog
        .iter()
        .filter_map(|row| {
            let row = row.strip_prefix('❯').unwrap_or(row).trim();
            let (number, label) = row.split_once(". ")?;
            (!number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())).then_some(label)
        })
        .collect();
    dialog
        .last()
        .is_some_and(|r| r.starts_with("Esc to cancel"))
        && dialog.iter().filter(|r| r.starts_with('❯')).count() == 1
        && dialog.iter().filter(|r| r.starts_with('❯')).all(|r| {
            r.strip_prefix('❯')
                .unwrap()
                .trim()
                .split_once(". ")
                .is_some_and(|(number, label)| {
                    !number.is_empty()
                        && number.bytes().all(|b| b.is_ascii_digit())
                        && (label == "Yes" || label == "No" || label.starts_with("Yes, "))
                })
        })
        && !dialog.iter().any(|r| r.starts_with('─'))
        && choices.contains(&"Yes")
        && choices.contains(&"No")
}

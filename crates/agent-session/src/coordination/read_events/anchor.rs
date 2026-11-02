//! The ordering anchor: the exact provider transcript's byte length now.
use std::collections::BTreeSet;

use nils_provider_resume as resume;

/// Byte length of the one transcript bound to `session_id`, or `None` when it
/// cannot be resolved exactly (missing, ambiguous or a truncated scan).
/// `config_dir` is the recipient's persisted provider configuration root; only
/// a session without one falls back to the process environment.
pub(super) fn transcript_len(
    provider: &str,
    session_id: &str,
    config_dir: Option<&std::path::Path>,
) -> Option<u64> {
    let mut matches = BTreeSet::new();
    match provider {
        "claude" => {
            let mut budget = resume::ClaudeResumeScanBudget::from_env();
            resume::collect_claude_named_resume_matches(
                &config_dir
                    .map(|dir| dir.join("projects"))
                    .or_else(resume::claude_projects_root)?,
                session_id,
                &mut matches,
                &mut budget,
            );
            (!budget.truncated).then_some(())?;
        }
        "codex" => {
            let mut budget = resume::CodexResumeScanBudget::from_env();
            resume::collect_codex_provider_resume_matches(
                &config_dir
                    .map(|dir| dir.join("sessions"))
                    .or_else(resume::codex_sessions_root)?,
                0,
                session_id,
                &mut matches,
                &mut budget,
            );
            (!budget.truncated).then_some(())?;
        }
        _ => return None,
    }
    let mut paths = matches.into_iter();
    let path = paths.next()?.path;
    paths.next().is_none().then_some(())?;
    Some(std::fs::metadata(path).ok()?.len())
}

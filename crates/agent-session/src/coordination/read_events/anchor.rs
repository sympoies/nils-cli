//! The ordering anchor: the exact provider transcript's byte length now.
use std::collections::BTreeSet;

use nils_provider_resume as resume;

/// Byte length of the one transcript bound to `session_id`, or `None` when it
/// cannot be resolved exactly (missing, ambiguous or a truncated scan).
pub(super) fn transcript_len(provider: &str, session_id: &str) -> Option<u64> {
    let mut matches = BTreeSet::new();
    match provider {
        "claude" => {
            let mut budget = resume::ClaudeResumeScanBudget::from_env();
            resume::collect_claude_named_resume_matches(
                &resume::claude_projects_root()?,
                session_id,
                &mut matches,
                &mut budget,
            );
            (!budget.truncated).then_some(())?;
        }
        "codex" => {
            let mut budget = resume::CodexResumeScanBudget::from_env();
            resume::collect_codex_provider_resume_matches(
                &resume::codex_sessions_root()?,
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

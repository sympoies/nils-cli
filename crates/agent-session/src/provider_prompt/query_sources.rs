//! Request-local sharing of complete Codex history discovery.
use std::collections::BTreeMap;

use nils_provider_resume::{CodexHistorySnapshot, CodexResumeScanBudget};

use super::*;

#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Scope {
    root: PathBuf,
    effective_config: Option<PathBuf>,
    profile: Option<String>,
}

/// Owned by one CLI list call; never reused by another request or the daemon.
#[derive(Default)]
pub(crate) struct QuerySources {
    codex: BTreeMap<Scope, Option<CodexHistorySnapshot>>,
    #[cfg(test)]
    scans: usize,
}

impl QuerySources {
    pub(crate) fn codex_source(&mut self, record: &SessionRecord) -> Option<ProviderPromptSource> {
        let resume = record.provider_resume.as_ref()?;
        if record.agent != "codex"
            || resume.provider != "codex"
            || resume.session_id.trim().is_empty()
        {
            return None;
        }
        let scope = Scope {
            root: crate::session_provider_config_dir(record)
                .map(|dir| dir.join("sessions"))
                .or_else(codex_sessions_root)?,
            effective_config: session_effective_provider_config_dir(record),
            profile: crate::session_agent_profile(record).map(str::to_string),
        };
        let snapshot = self.codex.entry(scope).or_insert_with_key(|scope| {
            #[cfg(test)]
            {
                self.scans += 1;
            }
            CodexHistorySnapshot::scan(&scope.root, &mut CodexResumeScanBudget::from_env())
        });
        Some(ProviderPromptSource {
            provider: ProviderKind::Codex,
            session_id: resume.session_id.clone(),
            path: snapshot
                .as_ref()?
                .unique_path(&resume.session_id)?
                .to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests;

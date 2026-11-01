//! One bounded Codex metadata walk shared within a single query.
use std::collections::BTreeMap;

use super::*;

/// Complete, transient discovery of one configured Codex history root.
///
/// This is not a persistent source cache or mutation authority. Callers must
/// validate a selected source's current identity before consuming its body.
/// A truncated walk never supplies a snapshot, even for an already seen id.
pub struct CodexHistorySnapshot {
    matches: BTreeMap<String, BTreeSet<ProviderHistoryMatch>>,
}

impl CodexHistorySnapshot {
    /// Scan arbitrary JSONL filenames with the same guards and budget as the
    /// single-id collector. Invalid metadata is ignored by that existing contract.
    pub fn scan(root: &Path, budget: &mut CodexResumeScanBudget) -> Option<Self> {
        let mut matches: BTreeMap<String, BTreeSet<ProviderHistoryMatch>> = BTreeMap::new();
        visit_metadata(root, 0, budget, &mut |path, meta| {
            matches
                .entry(meta.session_id)
                .or_default()
                .insert(ProviderHistoryMatch {
                    path,
                    cwd: meta.cwd,
                });
        });
        (!budget.truncated).then_some(Self { matches })
    }

    /// A unique discovered path, or unknown for absent/ambiguous ids.
    /// This observation does not replace per-use source validation.
    pub fn unique_path(&self, session_id: &str) -> Option<&Path> {
        let matches = self.matches.get(session_id)?;
        (matches.len() == 1).then(|| matches.first().unwrap().path.as_path())
    }
}

pub(super) fn visit_metadata(
    dir: &Path,
    depth: usize,
    budget: &mut CodexResumeScanBudget,
    visit: &mut impl FnMut(PathBuf, CodexSessionMeta),
) {
    if depth > CODEX_RESUME_SCAN_MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !budget.visit_entry() {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            visit_metadata(&path, depth + 1, budget, visit);
            if budget.truncated {
                return;
            }
            continue;
        }
        if !file_type.is_file()
            || path.extension().and_then(|value| value.to_str()) != Some("jsonl")
        {
            continue;
        }
        if let Some(meta) = read_codex_resumable_session_meta(&path) {
            visit(path, meta);
        }
    }
}

#[cfg(test)]
mod tests;

//! Registry-bound Claude transcripts get the first share of the existing scan budget.
use super::*;
use std::collections::BTreeSet;

pub(super) fn scan_catalog(
    sources: &[HistorySource],
    dsh_sources: &[DshHistorySource],
    archives_root: &Path,
    stars_root: &Path,
    session_state_dir: Option<&Path>,
) -> CatalogSnapshot {
    let deadline = Instant::now() + SCAN_MAX_DURATION;
    let mut truncated = false;
    let mut visited = 0usize;
    let preferred = managed_claude_sessions(
        sources,
        session_state_dir,
        deadline,
        &mut visited,
        &mut truncated,
    );
    let archives = read_archives(archives_root, deadline, &mut truncated);
    let stars = read_stars(stars_root, deadline, &mut truncated);
    let mut sessions = Vec::new();
    let mut incremental_segments = BTreeMap::<String, Vec<HistorySession>>::new();
    let mut seen = HashSet::new();
    // Configured roots may overlap through symlinks (for example a Claude
    // account directory whose `projects/` links to the shared tree), so each
    // physical transcript is attributed to the first source that reaches it.
    let mut seen_transcripts = HashSet::new();
    let mut catalog_stamps = IncrementalCatalogStampCache::default();

    // Inspect all registry-bound transcripts before directory-generation stamps
    // or the broad scan can spend the shared deadline on an older source.
    for mut session in preferred {
        let physical = fs::canonicalize(&session.transcript_path)
            .unwrap_or_else(|_| session.transcript_path.clone());
        if !seen_transcripts.insert(physical) {
            continue;
        }
        session.incremental_catalog_stamp = incremental_catalog_stamp(
            &session.transcript_path,
            &mut catalog_stamps,
            deadline,
            &mut truncated,
        );
        overlay_saved_metadata(&mut session, &archives, &stars);
        incremental_segments
            .entry(session.id.clone())
            .or_default()
            .push(session.clone());
        if seen.insert(session.id.clone()) {
            sessions.push(session);
        }
    }

    for source in sources {
        let mut paths = Vec::new();
        collect_jsonl(
            &source.root,
            if source.provider == "codex" { 6 } else { 2 },
            0,
            &mut paths,
            &mut visited,
            deadline,
            &mut truncated,
        );
        paths.sort();
        for path in paths {
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
            let physical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if !seen_transcripts.insert(physical) {
                continue;
            }
            let Some(mut session) = inspect_history_file(
                source,
                &path,
                deadline,
                &mut catalog_stamps,
                &mut truncated,
                true,
            ) else {
                continue;
            };
            overlay_saved_metadata(&mut session, &archives, &stars);
            incremental_segments
                .entry(session.id.clone())
                .or_default()
                .push(session.clone());
            if !seen.insert(session.id.clone()) {
                continue;
            }
            sessions.push(session);
            if Instant::now() >= deadline {
                truncated = true;
                break;
            }
        }
        if truncated {
            break;
        }
    }

    for source in dsh_sources {
        let remaining = SCAN_MAX_ENTRIES.saturating_sub(visited);
        if remaining == 0 || Instant::now() >= deadline {
            truncated = true;
            break;
        }
        let Ok(mut listed) = list_dsh_sessions(source) else {
            truncated = true;
            continue;
        };
        if listed.len() > remaining {
            listed.truncate(remaining);
            truncated = true;
        }
        visited += listed.len();
        for item in listed {
            if !valid_dsh_catalog_item(&item) {
                truncated = true;
                continue;
            }
            let id = stable_history_id(
                "dsh",
                Some(&source.agent_profile),
                &item.provider_session_id,
            );
            if !seen.insert(id.clone()) {
                continue;
            }
            sessions.push(HistorySession {
                archived_title_mode: Default::default(),
                id: id.clone(),
                provider: "dsh".to_string(),
                provider_version: None,
                provider_session_id: item.provider_session_id,
                agent_profile: Some(source.agent_profile.clone()),
                title: None,
                prompt_preview: None,
                first_user_prompt_preview: None,
                last_user_prompt_preview: None,
                repo_name: repo_name(&item.cwd),
                cwd: item.cwd,
                created_at: item.created_at,
                updated_at: item.updated_at,
                archived_at: archives.get(&id).map(|archive| archive.archived_at.clone()),
                starred_at: stars.get(&id).cloned(),
                resumable: false,
                archived_session_id: None,
                archived_title_state: None,
                transcript_path: source.root.clone(),
                incremental_catalog_stamp: item.revision,
            });
        }
    }

    for segments in incremental_segments.values_mut() {
        segments.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.transcript_path.cmp(&right.transcript_path))
        });
        segments.dedup_by(|left, right| left.transcript_path == right.transcript_path);
    }

    CatalogSnapshot {
        sessions,
        incremental_segments,
        truncated,
    }
}

fn overlay_saved_metadata(
    session: &mut HistorySession,
    archives: &BTreeMap<String, ArchivedSession>,
    stars: &BTreeMap<String, String>,
) {
    let archive = archives.get(&session.id);
    if let Some(archive) = archive {
        session.id = archive.history_id.clone();
        session.agent_profile = archive.agent_profile.clone();
        session.title = archive.title.clone();
        session.archived_session_id = archive.session_id.clone();
        session.archived_title_state = archive.title_state.clone();
        session.archived_title_mode = archive.title_mode;
        session.cwd = archive.cwd.clone();
        session.repo_name = repo_name(&archive.cwd);
        session.archived_at = Some(archive.archived_at.clone());
    }
    session.starred_at = stars.get(&session.id).cloned();
}

// This pass shares SCAN_MAX_ENTRIES and the catalog deadline with discovery.
// It reads only bounded registry documents and named transcripts; it never
// enumerates the old transcript filenames in each project.
fn managed_claude_sessions(
    sources: &[HistorySource],
    state_dir: Option<&Path>,
    deadline: Instant,
    visited: &mut usize,
    truncated: &mut bool,
) -> Vec<HistorySession> {
    let mut sessions = Vec::new();
    if !sources.iter().any(|source| source.provider == "claude") {
        return sessions;
    }
    let Some(state_dir) = state_dir else {
        return sessions;
    };
    let Ok(entries) = fs::read_dir(state_dir.join("sessions")) else {
        return sessions;
    };
    let mut identities = BTreeSet::new();
    for entry in entries.flatten() {
        if !scan_step(visited, deadline, truncated) {
            break;
        }
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let directory = entry.path();
        // Use the same record reader (including identity-bound resume sidecars)
        // as the managed registry. Refuse links and oversized documents first.
        if !bounded_registry_document(&directory.join("session.json"), false)
            || !bounded_registry_document(&directory.join(crate::SESSION_RESUME_FILE), true)
        {
            continue;
        }
        let Ok(record) = crate::read_session_record(&directory.join("session.json")) else {
            continue;
        };
        if entry.file_name().to_str() != Some(record.id.as_str()) {
            continue;
        }
        let Some(resume) = record.provider_resume.as_ref() else {
            continue;
        };
        if resume.provider != "claude" {
            continue;
        }
        let Ok(id) = uuid::Uuid::parse_str(&resume.session_id) else {
            continue;
        };
        identities.insert(id.hyphenated().to_string());
    }
    if identities.is_empty() {
        return sessions;
    }
    let mut seen = HashSet::new();
    for source in sources.iter().filter(|source| source.provider == "claude") {
        let Ok(projects) = fs::read_dir(&source.root) else {
            continue;
        };
        for project in projects.flatten() {
            if !scan_step(visited, deadline, truncated) {
                return sessions;
            }
            if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            for id in &identities {
                if !scan_step(visited, deadline, truncated) {
                    return sessions;
                }
                let path = project.path().join(format!("{id}.jsonl"));
                if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
                    continue;
                }
                let physical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen.insert(physical) {
                    continue;
                }
                if let Some(session) = inspect_history_file(
                    source,
                    &path,
                    deadline,
                    &mut IncrementalCatalogStampCache::default(),
                    truncated,
                    false,
                ) {
                    sessions.push(session);
                }
            }
        }
    }
    sessions
}

fn bounded_registry_document(path: &Path, optional: bool) -> bool {
    match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_file() && metadata.len() <= 1024 * 1024,
        Err(error) => optional && error.kind() == std::io::ErrorKind::NotFound,
    }
}

fn scan_step(visited: &mut usize, deadline: Instant, truncated: &mut bool) -> bool {
    if *visited >= SCAN_MAX_ENTRIES || Instant::now() >= deadline {
        *truncated = true;
        return false;
    }
    *visited += 1;
    true
}

#[cfg(test)]
#[path = "catalog_scan_tests.rs"]
mod tests;

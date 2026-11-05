use super::*;

impl HistoryCatalog {
    pub(super) fn exact_claude_page(
        &self,
        machine: &str,
        query: Option<&str>,
        provider: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
        titles: &BTreeMap<String, String>,
    ) -> Result<Option<HistoryPage>, HistoryError> {
        let Some(id) = query.map(str::trim) else {
            return Ok(None);
        };
        if provider.is_some_and(|provider| provider != "claude") || id.len() != 36 {
            return Ok(None);
        }
        let Ok(id) = uuid::Uuid::parse_str(id) else {
            return Ok(None);
        };
        let id = id.hyphenated().to_string();
        let mut sessions = Vec::new();
        let mut seen = HashSet::new();
        let mut truncated = false;
        let mut stamps = IncrementalCatalogStampCache::default();
        for source in self
            .sources
            .iter()
            .filter(|source| source.provider == "claude")
        {
            let Ok(projects) = fs::read_dir(&source.root) else {
                continue;
            };
            for project in projects.flatten() {
                if !project.file_type().is_ok_and(|kind| kind.is_dir()) {
                    continue;
                }
                let path = project.path().join(format!("{id}.jsonl"));
                if !fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_file()) {
                    continue;
                }
                let physical = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if !seen.insert(physical) {
                    continue;
                }
                let deadline = Instant::now() + SCAN_MAX_DURATION;
                let Some(mut session) = inspect_exact_history_file(
                    source,
                    &path,
                    deadline,
                    &mut stamps,
                    &mut truncated,
                ) else {
                    continue;
                };
                if let Some(archive) = read_archive(&self.archives_root, &session.id) {
                    session.agent_profile = archive.agent_profile;
                    session.title = archive.title;
                    session.archived_session_id = archive.session_id;
                    session.archived_title_state = archive.title_state;
                    session.archived_title_mode = archive.title_mode;
                    session.cwd = archive.cwd;
                    session.repo_name = repo_name(&session.cwd);
                    session.archived_at = Some(archive.archived_at);
                }
                let star_path = self.stars_root.join(format!("{}.json", session.id));
                if fs::metadata(&star_path).is_ok_and(|meta| meta.len() <= MAX_STAR_BYTES)
                    && let Ok(bytes) = fs::read(star_path)
                    && let Ok(star) = serde_json::from_slice::<StarredSession>(&bytes)
                    && star.history_id == session.id
                {
                    session.starred_at = Some(star.starred_at);
                }
                sessions.push(session);
            }
        }
        if sessions.is_empty() && provider.is_none() && !truncated {
            return Ok(None);
        }
        {
            let mut cache = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if cache.direct_sessions.len() > 1024 {
                cache.direct_sessions.clear();
            }
            for session in &sessions {
                cache
                    .direct_sessions
                    .insert(session.id.clone(), session.clone());
            }
        }
        let exact_ids: HashSet<_> = sessions.iter().map(|session| session.id.clone()).collect();
        let snapshot = self.snapshot();
        sessions.extend(
            snapshot
                .sessions
                .iter()
                .filter(|session| !exact_ids.contains(&session.id))
                .cloned(),
        );
        apply_title_overrides(&mut sessions, titles);
        sessions.retain(|session| {
            provider.is_none_or(|value| value == session.provider)
                && matches_metadata(session, machine, query)
        });
        sessions.sort_by(|left, right| {
            exact_ids
                .contains(&right.id)
                .cmp(&exact_ids.contains(&left.id))
                .then_with(|| compare_list_keys(&list_key(left), &list_key(right)))
        });
        if let Some(cursor) = cursor.map(parse_list_cursor).transpose()? {
            let cursor_is_exact = exact_ids.contains(&cursor.id);
            sessions.retain(|session| {
                let is_exact = exact_ids.contains(&session.id);
                (cursor_is_exact && !is_exact)
                    || (cursor_is_exact == is_exact && session_is_after_cursor(session, &cursor))
            });
        }
        let limit = limit.clamp(1, 100);
        let has_more = sessions.len() > limit;
        sessions.truncate(limit);
        let next_cursor = has_more
            .then(|| sessions.last().map(encode_list_cursor))
            .flatten();
        let mut page = HistoryPage {
            sessions,
            next_cursor,
            truncated: snapshot.truncated,
        };
        enrich_latest_prompt_previews(&mut page.sessions, &mut HashMap::new());
        Ok(Some(page))
    }

    pub(super) fn direct_session(&self, id: &str) -> Option<HistorySession> {
        let session = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .direct_sessions
            .get(id)
            .cloned()?;
        let project = session.transcript_path.parent()?;
        let root = project.parent()?;
        if !self
            .sources
            .iter()
            .any(|source| source.provider == "claude" && source.root == root)
            || !fs::symlink_metadata(project).ok()?.file_type().is_dir()
            || !fs::symlink_metadata(&session.transcript_path)
                .ok()?
                .file_type()
                .is_file()
            || nils_provider_resume::read_claude_session_cwd(
                &session.transcript_path,
                &session.provider_session_id,
            )
            .is_none()
        {
            return None;
        }
        Some(session)
    }
}

pub(super) fn inspect_history_file(
    source: &HistorySource,
    path: &Path,
    deadline: Instant,
    catalog_stamps: &mut IncrementalCatalogStampCache,
    truncated: &mut bool,
    include_stamp: bool,
) -> Option<HistorySession> {
    if Instant::now() >= deadline {
        return None;
    }
    let metadata = fs::metadata(path).ok()?;
    let updated_at = format_system_time(metadata.modified().ok()?);
    let (provider_session_id, cwd, created_at, provider_version) = match source.provider.as_str() {
        "codex" => {
            let meta = nils_provider_resume::read_codex_resumable_session_meta(path)?;
            (
                meta.session_id,
                meta.cwd,
                format_system_time(meta.created_at),
                history_images::provider_version(path, "codex"),
            )
        }
        "claude" => {
            let id = path.file_stem()?.to_str()?.to_string();
            let cwd = nils_provider_resume::read_claude_session_cwd(path, &id)?;
            (id, cwd, updated_at.clone(), None)
        }
        _ => return None,
    };
    let id = stable_history_id(
        &source.provider,
        source.agent_profile.as_deref(),
        &provider_session_id,
    );
    let first_user_prompt_preview =
        first_prompt(path, &source.provider, &provider_session_id, deadline);
    Some(HistorySession {
        archived_title_mode: Default::default(),
        id,
        provider: source.provider.clone(),
        provider_version,
        provider_session_id,
        agent_profile: source.agent_profile.clone(),
        title: None,
        prompt_preview: first_user_prompt_preview.clone(),
        first_user_prompt_preview,
        last_user_prompt_preview: None,
        repo_name: repo_name(&cwd),
        cwd,
        created_at,
        updated_at,
        archived_at: None,
        starred_at: None,
        resumable: true,
        archived_session_id: None,
        archived_title_state: None,
        transcript_path: path.to_path_buf(),
        incremental_catalog_stamp: if include_stamp {
            incremental_catalog_stamp(path, catalog_stamps, deadline, truncated)
        } else {
            String::new()
        },
    })
}

fn inspect_exact_history_file(
    source: &HistorySource,
    path: &Path,
    deadline: Instant,
    stamps: &mut IncrementalCatalogStampCache,
    truncated: &mut bool,
) -> Option<HistorySession> {
    let session = inspect_history_file(source, path, deadline, stamps, truncated, false);
    if session.is_none() && Instant::now() >= deadline {
        *truncated = true;
    }
    session
}

#[cfg(test)]
mod deadline_tests {
    use super::*;

    #[test]
    fn expired_inspection_reports_truncation() {
        let tmp = tempfile::tempdir().unwrap();
        let source = HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().to_path_buf(),
        };
        let mut truncated = false;
        let result = inspect_exact_history_file(
            &source,
            &tmp.path().join("missing.jsonl"),
            Instant::now() - Duration::from_secs(1),
            &mut IncrementalCatalogStampCache::default(),
            &mut truncated,
        );
        assert!(result.is_none());
        assert!(truncated);
    }
}

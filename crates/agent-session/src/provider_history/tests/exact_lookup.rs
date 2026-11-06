use super::*;

#[test]
fn exact_claude_id_survives_catalog_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let crowded = tmp.path().join("crowded");
    let project = tmp.path().join("projects/repo");
    fs::create_dir_all(&crowded).unwrap();
    fs::create_dir_all(&project).unwrap();
    for n in 0..=SCAN_MAX_ENTRIES {
        fs::create_dir(crowded.join(format!("{n}"))).unwrap();
    }
    let id = "12345678-1234-1234-1234-123456789abc";
    fs::write(project.join(format!("{id}.jsonl")), format!("{{\"sessionId\":\"{id}\",\"cwd\":\"/work/repo\",\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}}}}\n")).unwrap();
    let sources = [
        HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: crowded,
        },
        HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("projects"),
        },
    ];
    let title_state: crate::SessionTitleState = serde_json::from_value(serde_json::json!({
        "topic": "Saved topic",
        "topic_source": "auto",
        "references": [],
        "activity": "Saved activity"
    }))
    .unwrap();
    write_archive(
        &tmp.path().join("archives"),
        &ArchivedSession {
            title_mode: crate::display_metadata::TitleMode::Pinned,
            schema_version: "agent-session.history-archive.v1".to_string(),
            history_id: stable_history_id("claude", None, id),
            provider: "claude".to_string(),
            provider_session_id: id.to_string(),
            agent_profile: None,
            session_id: Some("managed-abc".to_string()),
            title: Some("Saved topic - Saved activity".to_string()),
            title_state: Some(title_state.clone()),
            cwd: "/work/repo".to_string(),
            created_at: "2026-09-30T00:00:00Z".to_string(),
            updated_at: "2026-09-30T00:00:00Z".to_string(),
            archived_at: "2026-09-30T00:00:01Z".to_string(),
        },
    )
    .unwrap()
    .commit();
    let catalog = HistoryCatalog::new(
        sources.to_vec(),
        tmp.path().join("archives"),
        tmp.path().join("stars"),
    );
    let page = catalog.list("test", Some(id), None, None, 50).unwrap();
    assert_eq!(page.sessions.len(), 1);
    assert_eq!(page.sessions[0].provider_session_id, id);
    assert!(!page.truncated);
    let messages = catalog
        .messages(
            &page.sessions[0].id,
            None,
            10,
            HistoryMessageDirection::Forward,
        )
        .unwrap();
    assert_eq!(messages.messages.len(), 1);
    // The list advertises star and resume, so both must work for this row.
    let history_id = page.sessions[0].id.clone();
    assert!(page.sessions[0].resumable);
    let starred_at = "2026-01-01T00:00:00Z";
    assert_eq!(
        catalog
            .set_star(&history_id, Some(starred_at))
            .unwrap()
            .as_deref(),
        Some(starred_at)
    );
    let resumed = catalog.resolve_fresh(&history_id).unwrap();
    assert_eq!(resumed.provider_session_id, id);
    assert_eq!(resumed.archived_session_id.as_deref(), Some("managed-abc"));
    assert_eq!(
        resumed.title.as_deref(),
        Some("Saved topic - Saved activity")
    );
    assert_eq!(resumed.archived_title_state, Some(title_state));
    assert_eq!(
        resumed.archived_title_mode,
        crate::display_metadata::TitleMode::Pinned
    );
    fs::remove_file(project.join(format!("{id}.jsonl"))).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("outside.jsonl"),
        project.join(format!("{id}.jsonl")),
    )
    .unwrap();
    fs::write(
        tmp.path().join("outside.jsonl"),
        format!("{{\"sessionId\":\"{id}\",\"cwd\":\"/work\"}}\n"),
    )
    .unwrap();
    assert!(catalog.direct_session(&page.sessions[0].id).is_none());
    assert!(catalog.set_star(&history_id, Some(starred_at)).is_err());
    assert!(catalog.resolve_fresh(&history_id).is_err());
}

#[test]
fn exact_claude_lookup_with_many_transcripts() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("projects/repo");
    fs::create_dir_all(&project).unwrap();
    let id = "12345678-1234-1234-1234-123456789abc";
    fs::write(
        project.join(format!("{id}.jsonl")),
        format!("{{\"sessionId\":\"{id}\",\"cwd\":\"/work/repo\"}}\n"),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("projects"),
        }],
        tmp.path().join("archives"),
        tmp.path().join("stars"),
    );
    for n in 0..10_000 {
        fs::write(project.join(format!("fixture-{n}.jsonl")), "").unwrap();
        if n == 999 || n == 9_999 {
            let start = Instant::now();
            let page = catalog
                .list("test", Some(id), Some("claude"), None, 10)
                .unwrap();
            eprintln!("{} transcripts: {:?}", n + 2, start.elapsed());
            assert_eq!(page.sessions.len(), 1);
            assert!(!page.truncated);
        }
    }
}

#[test]
fn exact_claude_lookup_accepts_uppercase_with_provider_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("projects/repo");
    fs::create_dir_all(&project).unwrap();
    let id = "12345678-1234-1234-1234-123456789abc";
    fs::write(
        project.join(format!("{id}.jsonl")),
        format!("{{\"sessionId\":\"{id}\",\"cwd\":\"/work/repo\"}}\n"),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("projects"),
        }],
        tmp.path().join("archives"),
        tmp.path().join("stars"),
    );
    let page = catalog
        .list("test", Some(&id.to_uppercase()), Some("claude"), None, 50)
        .unwrap();
    assert_eq!(page.sessions.len(), 1);
}

#[test]
fn exact_claude_list_rejects_symlinked_project_and_transcript() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("projects");
    let outside = tmp.path().join("outside");
    fs::create_dir_all(root.join("real")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    let id = "12345678-1234-1234-1234-123456789abc";
    let target = outside.join(format!("{id}.jsonl"));
    fs::write(
        &target,
        format!("{{\"sessionId\":\"{id}\",\"cwd\":\"/work\"}}\n"),
    )
    .unwrap();
    std::os::unix::fs::symlink(&target, root.join("real").join(format!("{id}.jsonl"))).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root,
        }],
        tmp.path().join("archives"),
        tmp.path().join("stars"),
    );
    let page = catalog
        .list("test", Some(id), Some("claude"), None, 50)
        .unwrap();
    assert!(page.sessions.is_empty());
}

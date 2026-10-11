use pretty_assertions::assert_eq;

use super::*;

#[test]
fn resolver_requires_one_exact_provider_identity() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_root = tmp.path().join("codex");
    let claude_root = tmp.path().join("claude");
    let codex_file = codex_root.join("2026/07/session.jsonl");
    fs::create_dir_all(codex_file.parent().expect("parent")).expect("codex dirs");
    fs::write(&codex_file, codex_meta("codex-id", "/repo")).expect("codex transcript");
    let codex = record("codex", "codex-id");
    let source =
        resolve_provider_prompt_source_from_roots(&codex, Some(&codex_root), Some(&claude_root))
            .expect("codex source");
    assert_eq!(source.provider, ProviderKind::Codex);
    assert_eq!(source.path, codex_file);

    let duplicate = codex_root.join("other.jsonl");
    fs::write(&duplicate, codex_meta("codex-id", "/repo")).expect("duplicate");
    assert!(
        resolve_provider_prompt_source_from_roots(&codex, Some(&codex_root), Some(&claude_root),)
            .is_none()
    );

    let unresolved = record("claude", "missing-id");
    assert!(
        resolve_provider_prompt_source_from_roots(
            &unresolved,
            Some(&codex_root),
            Some(&claude_root),
        )
        .is_none()
    );
}

#[test]
fn profiled_claude_uses_its_persisted_config_root() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let profile_config = tmp.path().join(".claude-gpt");
    let fallback = tmp.path().join(".claude/projects");
    let mut record = record("claude", "claude-id");
    record.runtime = Some(crate::RuntimeInfo {
        kind: "tmux".to_string(),
        tmux_session: record.tmux_session.clone(),
        generation: 1,
        started_at: "2099-01-01T00:00:00Z".to_string(),
        launch_id: "runtime-1".to_string(),
        extra: BTreeMap::from([(
            "agent_profile_provider_config_dir".to_string(),
            json!(profile_config),
        )]),
    });

    assert_eq!(
        claude_projects_root_for_record(&record, Some(fallback)),
        Some(profile_config.join("projects"))
    );
}

#[test]
fn vscode_codex_prompt_source_opens_after_resolution() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_root = tmp.path().join("codex");
    let codex_file = codex_root.join("2026/07/session.jsonl");
    fs::create_dir_all(codex_file.parent().expect("parent")).expect("codex dirs");
    fs::write(
        &codex_file,
        format!(
            "{}\n",
            json!({
                "timestamp":"2099-01-01T00:00:00Z",
                "type":"session_meta",
                "payload":{
                    "id":"codex-id",
                    "session_id":"codex-id",
                    "cwd":"/repo",
                    "source":"vscode",
                    "originator":"codex-tui",
                    "timestamp":"2099-01-01T00:00:00Z"
                }
            })
        ),
    )
    .expect("codex transcript");
    let record = record("codex", "codex-id");
    let source = resolve_provider_prompt_source_from_roots(&record, Some(&codex_root), None)
        .expect("vscode prompt source");

    assert!(ProviderPromptTail::open_new_runtime_source(source).is_some());
}

#[cfg(unix)]
#[test]
fn resolver_ignores_symlinks_and_non_regular_transcripts() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_root = tmp.path().join("codex");
    let outside = tmp.path().join("outside.jsonl");
    fs::create_dir_all(&codex_root).expect("root");
    fs::write(&outside, codex_meta("codex-id", "/repo")).expect("outside transcript");
    symlink(&outside, codex_root.join("linked.jsonl")).expect("symlink");
    let _socket = UnixListener::bind(codex_root.join("socket.jsonl")).expect("unix socket");

    let record = record("codex", "codex-id");
    assert!(resolve_provider_prompt_source_from_roots(&record, Some(&codex_root), None).is_none());
}

#[test]
fn resolver_fails_soft_when_shared_history_budget_is_exhausted() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let codex_root = tmp.path().join("codex");
    let transcript = codex_root.join("nested/session.jsonl");
    fs::create_dir_all(transcript.parent().expect("parent")).expect("dirs");
    fs::write(&transcript, codex_meta("codex-id", "/repo")).expect("transcript");
    let _entries = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RESUME_SCAN_MAX_ENTRIES", "1");

    let record = record("codex", "codex-id");
    assert!(resolve_provider_prompt_source_from_roots(&record, Some(&codex_root), None).is_none());
}

fn claude_metadata(session_id: &str) -> String {
    // Real transcripts can carry several unrelated records before identity.
    format!(
        "{}{}\n",
        "{\"type\":\"queue-operation\"}\n".repeat(6),
        json!({"type":"user", "sessionId":session_id, "cwd":"/repo", "isSidechain":false})
    )
}

fn claude_history(root: &Path) -> PathBuf {
    let oversized = "x".repeat(nils_provider_resume::CLAUDE_SESSION_META_MAX_LINE_BYTES + 1);
    for project in 0..55 {
        let dir = root.join(format!("project-{project:02}"));
        fs::create_dir_all(&dir).expect("project dir");
        fs::write(dir.join("oversized.jsonl"), &oversized).expect("oversized unrelated metadata");
        for transcript in 0..7 {
            fs::write(
                dir.join(format!("unrelated-{transcript}.jsonl")),
                claude_metadata("other-conversation"),
            )
            .expect("unrelated transcript");
        }
    }
    let target = root.join("project-27/claude-id.jsonl");
    fs::write(&target, claude_metadata("claude-id")).expect("target transcript");
    target
}

#[test]
fn known_claude_source_resolves_without_reading_unrelated_history() {
    let lock = GlobalStateLock::new();
    let _entries = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_RESUME_SCAN_MAX_ENTRIES", "60");
    let _time = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_RESUME_SCAN_SLICE_MS", "60000");
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    let target = claude_history(&root);
    let source = resolve_provider_prompt_source_from_roots(
        &record("claude", "claude-id"),
        None,
        Some(&root),
    )
    .expect("known conversation must resolve even when global transcript discovery truncates");
    assert_eq!(source.path, target);
    assert!(ProviderPromptTail::open_source_at_eof(source).is_some());
}

#[test]
fn known_claude_source_rejects_ambiguous_and_mismatched_metadata() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    for project in ["first", "second"] {
        fs::create_dir_all(root.join(project)).expect("project");
        fs::write(
            root.join(project).join("claude-id.jsonl"),
            claude_metadata("claude-id"),
        )
        .expect("duplicate identity");
    }
    let recipient = record("claude", "claude-id");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
    fs::remove_file(root.join("second/claude-id.jsonl")).expect("remove duplicate");
    fs::write(
        root.join("first/claude-id.jsonl"),
        claude_metadata("other-id"),
    )
    .expect("mismatch");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
}

#[test]
fn known_claude_source_preserves_missing_oversized_and_bounded_failures() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    let recipient = record("claude", "claude-id");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
    let target = claude_history(&root);
    fs::write(
        &target,
        "x".repeat(nils_provider_resume::CLAUDE_SESSION_META_MAX_LINE_BYTES + 1),
    )
    .expect("oversized target");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
    fs::write(&target, claude_metadata("claude-id")).expect("restore identity");
    let _entries = EnvGuard::set(&lock, "AGENT_SESSION_CLAUDE_RESUME_SCAN_MAX_ENTRIES", "1");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
}

#[cfg(unix)]
#[test]
fn known_claude_source_rejects_symlinks_and_non_regular_files() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    fs::create_dir_all(root.join("project")).expect("project");
    let outside = tmp.path().join("outside.jsonl");
    fs::write(&outside, claude_metadata("claude-id")).expect("outside");
    let target = root.join("project/claude-id.jsonl");
    symlink(&outside, &target).expect("symlink");
    let recipient = record("claude", "claude-id");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
    fs::remove_file(&target).expect("unlink");
    let _socket = UnixListener::bind(&target).expect("socket");
    assert!(resolve_provider_prompt_source_from_roots(&recipient, None, Some(&root)).is_none());
}

#[test]
fn known_claude_source_retains_noncanonical_filename_discovery() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    fs::create_dir_all(root.join("project")).expect("project");
    let transcript = root.join("project/historical.jsonl");
    fs::write(&transcript, claude_metadata("claude-id")).expect("historical transcript");
    let source = resolve_provider_prompt_source_from_roots(
        &record("claude", "claude-id"),
        None,
        Some(&root),
    )
    .expect("unambiguous historical transcript");
    assert_eq!(source.path, transcript);
}

#[cfg(unix)]
#[test]
fn known_claude_source_rejects_unreadable_transcript() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let root = tmp.path().join("projects");
    fs::create_dir_all(root.join("project")).expect("project");
    let transcript = root.join("project/claude-id.jsonl");
    fs::write(&transcript, claude_metadata("claude-id")).expect("transcript");
    fs::set_permissions(&transcript, fs::Permissions::from_mode(0o000)).expect("unreadable");
    // Privileged runners may read mode-000 files; assert failure when the OS
    // actually refuses access rather than claiming chmod itself proves it.
    if File::open(&transcript).is_err() {
        assert!(
            resolve_provider_prompt_source_from_roots(
                &record("claude", "claude-id"),
                None,
                Some(&root),
            )
            .is_none()
        );
    }
    fs::set_permissions(&transcript, fs::Permissions::from_mode(0o600)).expect("restore");
}

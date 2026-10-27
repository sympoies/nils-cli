#[test]
fn worktree_lifecycle_blocks_provider_import_and_existing_resume() {
    use super::*;
    let tmp = tempfile::TempDir::new().unwrap();
    let checkout = tmp.path().join("checkout");
    fs::create_dir(&checkout).unwrap();
    assert!(
        ProcessCommand::new("git")
            .current_dir(&checkout)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success()
    );
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record = create_record(RecordRequest {
        context: &context,
        agent: AgentKind::Codex,
        mode: "interactive",
        coordination_mode: cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("existing"),
        cwd: &checkout,
        prompt: None,
        log_file_name: None,
        provider_resume: None,
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .unwrap()
    .record;
    let record_path = context.state_dir.join("sessions/existing/session.json");
    let before = fs::read(&record_path).unwrap();
    let _guard = nils_common::worktree_lifecycle::Guard::acquire(
        &crate::runtime_environment::worktree_lifecycle_state_home(&context).unwrap(),
        &checkout,
    )
    .unwrap();
    let args = ProviderResumeImportArgs {
        ordinary_start: None,
        launch_env: launch_env::LaunchEnv::new(),
        agent: AgentKind::Codex,
        title_mode: display_metadata::TitleMode::Auto,
        provider_resume_id: "provider-fixture".into(),
        title: None,
        title_state: None,
        id: Some("imported".into()),
        coordination_mode: cli::CoordinationMode::Advisory,
        tmux_bin: Some(tmp.path().join("never-launch")),
        agent_bin: None,
        agent_profile: None,
        provider_config_dir: None,
        profile_auto_resume_supported: None,
        profile_graceful_shutdown: None,
        codex_usage_account: None,
        agent_args: Vec::new(),
        initial_lineage: InitialLineage {
            origin: origin::Fields::default(),
            seed: lineage::LineageSeed::root("fixture", "user", "test"),
            work: None,
            role: None,
        },
        format: OutputFormat::Json,
    };
    let provider_resume = ProviderResume {
        provider: "codex".into(),
        session_id: "provider-fixture".into(),
        captured_at: "2030-01-01T00:00:00Z".into(),
        capture_method: "fixture".into(),
        resume_args: Vec::new(),
        extra: BTreeMap::new(),
    };
    let imported =
        start_resolved_provider_resume_session(&context, args, checkout, provider_resume)
            .unwrap_err();
    pretty_assertions::assert_eq!(imported.code(), "worktree-lifecycle-busy");
    assert!(!context.state_dir.join("sessions/imported").exists());
    let resumed = resume_session_locked(&context, record, &tmp.path().join("never-launch"))
        .err()
        .expect("resume must retain the barrier");
    pretty_assertions::assert_eq!(resumed.code(), "worktree-lifecycle-busy");
    pretty_assertions::assert_eq!(fs::read(record_path).unwrap(), before);
}

#[test]
fn padded_import_id_refuses_a_live_canonical_writer_before_creation() {
    use super::*;
    use pretty_assertions::assert_eq;
    let tmp = tempfile::tempdir().unwrap();
    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let id = "00000000-0000-4000-8000-000000000007";
    let root = tmp.path().join("account");
    fs::create_dir_all(root.join("projects/project")).unwrap();
    fs::write(
        root.join(format!("projects/project/{id}.jsonl")),
        format!(
            "{}\n",
            json!({"sessionId":id,"cwd":tmp.path(),"type":"user"})
        ),
    )
    .unwrap();
    let launched = tmp.path().join("launched");
    let tmux = tmp.path().join("tmux");
    fs::write(&tmux, format!("#!/bin/sh\nif [ \"$1\" = has-session ]; then exit 0; fi\nif [ \"$1\" = new-session ]; then : > {}; fi\nexit 42\n", shell_words::quote(launched.to_str().unwrap()))).unwrap();
    fs::set_permissions(&tmux, fs::Permissions::from_mode(0o700)).unwrap();
    let source = create_record(RecordRequest {
        context: &context,
        agent: AgentKind::Claude,
        mode: "interactive",
        coordination_mode: cli::CoordinationMode::Advisory,
        title: None,
        title_state: None,
        explicit_id: Some("source"),
        cwd: tmp.path(),
        prompt: None,
        log_file_name: None,
        provider_resume: Some(ProviderResume {
            provider: "claude".into(),
            session_id: id.into(),
            captured_at: "2026-01-01T00:00:00Z".into(),
            capture_method: "fixture".into(),
            resume_args: vec!["--resume".into(), id.into()],
            extra: BTreeMap::new(),
        }),
        agent_args: Vec::new(),
        agent_bin: None,
    })
    .unwrap()
    .record;
    let record_path = session_dir(&context, &source.id).join("session.json");
    let before = fs::read(&record_path).unwrap();
    let args = ProviderResumeImportArgs {
        ordinary_start: None,
        launch_env: launch_env::LaunchEnv::new(),
        agent: AgentKind::Claude,
        title_mode: display_metadata::TitleMode::Auto,
        provider_resume_id: format!(" {id} "),
        title: None,
        title_state: None,
        id: Some("replacement".into()),
        coordination_mode: cli::CoordinationMode::Advisory,
        tmux_bin: Some(tmux),
        agent_bin: None,
        agent_profile: None,
        provider_config_dir: Some(root),
        profile_auto_resume_supported: None,
        profile_graceful_shutdown: None,
        codex_usage_account: None,
        agent_args: Vec::new(),
        initial_lineage: InitialLineage {
            origin: origin::Fields::default(),
            seed: lineage::LineageSeed::root("fixture", "user", "test"),
            work: None,
            role: None,
        },
        format: OutputFormat::Json,
    };
    let error = start_provider_resume_session(&context, args).unwrap_err();
    assert_eq!(error.code(), "resume-source-live");
    assert!(!context.state_dir.join("sessions/replacement").exists());
    assert!(
        !launched.exists(),
        "the canonical writer must fence runtime launch"
    );
    assert_eq!(fs::read(record_path).unwrap(), before);
}

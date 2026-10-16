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
        &nils_common::worktree_lifecycle::state_home().unwrap(),
        &checkout,
    )
    .unwrap();
    let args = ProviderResumeImportArgs {
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

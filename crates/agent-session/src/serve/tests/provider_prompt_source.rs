use super::*;

#[tokio::test]
async fn provider_prompt_resolver_fails_closed_on_a_mismatched_exact_source() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let claude_config = tmp.path().join("claude-config");
    let _claude_config = EnvGuard::set(&lock, "CLAUDE_CONFIG_DIR", claude_config.to_str().unwrap());
    let record = provider_discovery_record(
        "mismatched-exact",
        "hs-mismatched-exact",
        "launch-mismatched-exact",
        2,
    );
    let provider_id = record
        .provider_resume
        .as_ref()
        .expect("provider resume")
        .session_id
        .clone();
    let transcript = |session_id: &str, content: &str| {
        format!(
            "{}\n",
            json!({
                "type":"user",
                "sessionId":session_id,
                "cwd":record.cwd.clone(),
                "message":{"role":"user","content":content}
            })
        )
    };
    let first = claude_config
        .join("projects/-first")
        .join(format!("{provider_id}.jsonl"));
    std::fs::create_dir_all(first.parent().expect("first parent")).expect("first dir");
    std::fs::write(&first, transcript(provider_id.as_str(), "first")).expect("first transcript");
    let registry = Arc::new(ProviderPromptDiscoveryRegistry::default());
    assert!(registry.resolve_source(&record).await.is_some());

    // The exact filename now carries another conversation's identity, while a
    // second exact file names this one: the conflict must not resolve.
    std::fs::write(&first, transcript("different-provider-id", "mismatch"))
        .expect("replace cached transcript");
    let second = claude_config
        .join("projects/-second")
        .join(format!("{provider_id}.jsonl"));
    std::fs::create_dir_all(second.parent().expect("second parent")).expect("second dir");
    std::fs::write(&second, transcript(provider_id.as_str(), "replacement"))
        .expect("second transcript");

    let context = CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let (_, tail) = resolve_provider_prompt_tail(
        context,
        registry.clone(),
        record.clone(),
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert!(
        tail.is_none(),
        "a still-present mismatched exact source must fail closed"
    );
}

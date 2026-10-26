use super::*;

#[tokio::test]
async fn launch_env_create_matches_cli_for_all_provider_profiles() {
    let lock = GlobalStateLock::new();
    let _policy = EnvGuard::remove(&lock, crate::launch_env::ALLOWLIST_ENV);
    let _runtime = EnvGuard::set(&lock, "AGENT_SESSION_CODEX_RUNTIME", "off");
    let _api = EnvGuard::remove(&lock, "ANTHROPIC_API_KEY");
    let _auth = EnvGuard::remove(&lock, "ANTHROPIC_AUTH_TOKEN");
    let _capacity = EnvGuard::set(&lock, "CLAUDE_CODE_MAX_CONTEXT_TOKENS", "131072");
    let _endpoint = EnvGuard::set(
        &lock,
        "ANTHROPIC_BASE_URL",
        "http://127.0.0.1:9/endpoint-canary",
    );
    let _codex_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    let _claude_broker = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    for agent in ["codex", "claude"] {
        let tmp = tempfile::TempDir::new().unwrap();
        let launcher = fake_agent(tmp.path(), "provider-fixture");
        let provider_root = tmp.path().join("profile-config");
        fs::create_dir_all(&provider_root).unwrap();
        let profiles = AgentLaunchProfiles::from_json(
            &json!([{
                "id":"local-fixture", "label":"Local fixture", "agent":agent,
                "agent_bin":launcher, "provider_config_dir":provider_root, "readiness_args":["--check"]
            }])
            .to_string(),
        )
        .unwrap();
        let log = tmp.path().join("tmux.log");
        let tmux = logging_tmux(tmp.path(), &log);
        let script = fs::read_to_string(&tmux).unwrap();
        fs::write(&tmux, script.replacen('\n', "\nfor arg; do case \"$arg\" in AGENT_SESSION_PRIVATE_PROVIDER_ENV=*) rm -f \"${arg#*=}\";; esac; done\n", 1)).unwrap();
        let mut st = state(tmp.path(), Some(TOKEN), tmux);
        Arc::get_mut(&mut st).unwrap().launch_profiles = profiles;
        let values = json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"});
        let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions",
                Some(TOKEN),
                json!({
                    "agent":agent, "agent_profile":"local-fixture", "id":"env-profile",
                    "cwd":tmp.path(), "launch_env":values
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["data"]["session"]["launch_env"], values);
        let record = load_session_record(&st.context, "env-profile").unwrap();
        assert_eq!(record.extra["launch_env"], values);
        if agent == "claude" {
            assert_eq!(
                crate::runtime_environment::provider_facts::claude_window(&record),
                Some(131072)
            );
            assert!(!crate::runtime_environment::provider_facts::account_applicable(&record));
            assert_eq!(
                body["data"]["session"]["claude_account"]["applicable"],
                false
            );
            assert!(!body.to_string().contains("endpoint-canary"));
        }
        assert!(
            fs::read_to_string(&log)
                .unwrap()
                .contains("-e AGENT_RUNTIME_SUPPRESS_MEMORY=1")
        );
        let projected =
            crate::board::project_record(&body["data"]["session"], "host-a", None, false).unwrap();
        assert_eq!(projected["launch_env"], values);
    }
}

#[tokio::test]
async fn launch_env_http_refuses_unapproved_keys_before_starting() {
    let tmp = tempfile::TempDir::new().unwrap();
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));
    let (status, body) = call(
        router(st),
        post_json(
            "/sessions",
            Some(TOKEN),
            json!({
                "agent":"codex", "launch_env":{"API_TOKEN":"private-fixture-value"}
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"]["code"], "launch-env-key-refused");
    assert!(!body.to_string().contains("private-fixture-value"));
    assert!(!tmp.path().join("sessions").exists());
    assert!(
        !fs::read_to_string(&log)
            .unwrap_or_default()
            .contains("new-session")
    );
}

#[tokio::test]
async fn codex_account_switch_routes_the_nickname_to_the_exact_runtime_control() {
    let lock = GlobalStateLock::new();
    let _policy = EnvGuard::remove(&lock, crate::launch_env::ALLOWLIST_ENV);
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = codex_listing_broker(&lock, tmp.path(), &["acct1", "omega"]);
    let launch_id = seed_codex_app_server_session(tmp.path(), "account-switch");
    let st = state(tmp.path(), Some(TOKEN), minimal_tmux(tmp.path()));
    let mut record = load_session_record(&st.context, "account-switch").unwrap();
    record.extra.insert(
        "launch_env".into(),
        json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"}),
    );
    crate::write_session_record(&st.context, &record).unwrap();
    crate::activity::activate_runtime(&st.context, &record).unwrap();
    for (event_id, kind) in [
        ("turn-start", "turn_started"),
        ("turn-done", "turn_completed"),
    ] {
        let event = serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": launch_id,
            "provider": "codex",
            "provider_turn_id": "turn-account-switch",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .unwrap();
        crate::activity::ingest_event(&st.context, &record.id, event).unwrap();
    }
    let (handle, mut commands) = codex_app_server::control_channel();
    st.codex_controls.lock().unwrap().insert(
        record.id.clone(),
        CodexControlEntry {
            launch_id: launch_id.clone(),
            handle,
        },
    );
    let responder_launch_id = launch_id.clone();
    let responder = tokio::spawn(async move {
        let Some(codex_app_server::ControlCommand::BindAccount {
            account,
            revision,
            response,
        }) = commands.recv().await
        else {
            panic!("account switch did not reach the Codex control plane");
        };
        assert_eq!(account, "acct1");
        assert_eq!(revision, 1);
        response
            .send(Ok(crate::codex_account::CodexAccountView {
                schema_version: crate::codex_account::VIEW_SCHEMA_VERSION,
                supported: true,
                applicable: true,
                state: "bound",
                selected_account: Some(account.clone()),
                effective_account: Some(account),
                selection_source: Some("explicit".to_string()),
                revision: 1,
                applied_runtime_id: Some(responder_launch_id),
                failure_reason: None,
                next: None,
            }))
            .unwrap();
    });

    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/account-switch/account",
            Some(TOKEN),
            json!({ "account": "acct1", "expected_session_incarnation": launch_id }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["codex_account"]["state"], "bound");
    assert_eq!(body["data"]["codex_account"]["selected_account"], "acct1");
    assert_eq!(body["data"]["codex_account"]["revision"], 1);
    responder.await.unwrap();
    assert_eq!(
        load_session_record(&st.context, "account-switch")
            .unwrap()
            .extra["launch_env"],
        json!({"AGENT_RUNTIME_SUPPRESS_MEMORY":"1"})
    );
}

//! Exercise consent through the origin receipt, relay wire and final target admission.
use super::*;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn target_consent_metadata_is_resume_only_and_never_weakens_account_binding() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let (state, broker, tmux) = fixture(temp.path());
    let _broker = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let app = router(crate::serve::tests::state(&state, Some(TOKEN), tmux));
    let before = snapshot(temp.path());
    for (route, body, code) in [
        (
            "/sessions/resume/v1",
            json!({"agent":"claude","resume":ID,"resume_account_explicit":null}),
            "resume-unsupported",
        ),
        (
            "/sessions/resume/v1",
            json!({"agent":"claude","resume":ID,"resume_account_explicit":"true"}),
            "resume-unsupported",
        ),
        (
            "/sessions/resume/v1",
            json!({"agent":"claude","resume":ID,"claude_account":"other","resume_account_explicit":false}),
            "resume-account-mismatch",
        ),
        (
            "/sessions",
            json!({"agent":"claude","resume_account_explicit":false}),
            "resume-unsupported",
        ),
    ] {
        let (_, result) = post(app.clone(), route, Some(TOKEN), body).await;
        assert_eq!(result["error"]["code"], code, "{route}: {result}");
        assert_eq!(result["error"]["details"]["session_created"], false);
        assert_eq!(snapshot(temp.path()), before);
    }
}

#[tokio::test]
async fn late_account_switch_requires_original_explicit_consent_across_console_hops() {
    let lock = GlobalStateLock::new();
    for explicit in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let target_root = temp.path().join("target");
        fs::create_dir_all(&target_root).unwrap();
        let (target_state, broker, _) = fixture(&target_root);
        let config = target_root.join("account");
        executable(
            &broker,
            &format!(
                "#!/bin/sh\ncase \"$1\" in\nlist) printf '%s\\n' '{{\"schema_version\":\"agent-session.account-broker.v2\",\"provider\":\"claude\",\"accounts\":[{{\"account\":\"original\"}},{{\"account\":\"other\"}}]}}' ;;\nmaterialize) printf '%s\\n' '{}' ;;\n*) exit 93 ;;\nesac\n",
                json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":config})
            ),
        );
        let _broker = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
            &json!([broker]).to_string(),
        );
        let provider = target_root.join("provider");
        executable(&provider, "#!/bin/sh\nexit 0\n");
        let _provider = EnvGuard::set(
            &lock,
            "AGENT_SESSION_CLAUDE_BIN",
            provider.to_str().unwrap(),
        );
        let log = target_root.join("launch.log");
        let tmux = crate::serve::tests::startup_fixtures::live_resume_tmux(&target_root, &log);
        let target = crate::serve::tests::state(&target_state, Some(TOKEN), tmux);
        let target_app = router(target.clone());
        let pending = Arc::new(StdMutex::new(None));
        let after_check = pending.clone();
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let seen = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(axum::routing::post(
            move |uri: axum::http::Uri, Json(body): Json<Value>| {
                let target = target.clone();
                let target_app = target_app.clone();
                let after_check = after_check.clone();
                let seen = seen.clone();
                async move {
                    seen.lock()
                        .unwrap()
                        .push((uri.path().to_string(), body.clone()));
                    let route = if uri.path() == "/api/coordination/resume-preflight/v1" {
                        "/resume-preflight/v1"
                    } else {
                        assert_eq!(uri.path(), "/api/coordination/sessions/v1");
                        "/sessions/resume/v1"
                    };
                    let (status, result) =
                        post(target_app, route, Some(TOKEN), body["session"].clone()).await;
                    if route == "/resume-preflight/v1" {
                        assert_eq!(status, StatusCode::OK, "{result}");
                        let mut source = load_session_record(&target.context, "source").unwrap();
                        crate::claude_account::queue_next(&mut source, "other").unwrap();
                        crate::write_session_record(&target.context, &source).unwrap();
                        *after_check.lock().unwrap() = Some(snapshot(&target.context.state_dir));
                    }
                    (status, Json(result))
                }
            },
        ));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let mut origin = crate::serve::tests::state(
            &temp.path().join("origin"),
            Some(TOKEN),
            PathBuf::from("/unavailable/tmux"),
        );
        let token = caller(&origin.context);
        Arc::get_mut(&mut origin).unwrap().federation = Some(crate::coordination::remote::Config {
            machine: "origin".into(),
            url: format!("http://{address}"),
            token: "synthetic-relay-token".into(),
            ingress_token: "synthetic-ingress-token".into(),
        });
        let mut request = json!({"operation_id":uuid::Uuid::now_v7().to_string(),"machine":"test-machine","no_parent":true,
            "session":{"agent":"claude","resume":ID,"id":"replacement"}});
        if explicit {
            request["session"]["claude_account"] = json!("original");
        }
        for _ in 0..2 {
            let (_, result) = post(
                router(origin.clone()),
                "/sessions/caller/console-resume/v1",
                Some(&token),
                request.clone(),
            )
            .await;
            if explicit {
                assert_eq!(result["session"]["id"], "replacement", "{result}");
            } else {
                assert_eq!(
                    result["error"]["code"], "resume-account-transition-pending",
                    "{result}"
                );
                assert_eq!(result["error"]["details"]["session_created"], false);
                assert_eq!(result["error"]["details"]["safe_to_retry"], false);
                assert_eq!(
                    snapshot(&target_state),
                    *pending.lock().unwrap().as_ref().unwrap()
                );
                assert!(!target_state.join("sessions/replacement").exists());
            }
        }
        server.abort();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "replay must not repeat either upstream hop"
        );
        assert_eq!(requests[1].1["session"]["claude_account"], "original");
        assert_eq!(
            requests[1].1["session"]["resume_account_explicit"],
            explicit
        );
        let launches = fs::read_to_string(&log)
            .unwrap_or_default()
            .matches("new-session")
            .count();
        assert_eq!(launches, usize::from(explicit));
        if explicit {
            let source = load_session_record(
                &CliContext {
                    state_dir: target_state,
                    host: None,
                },
                "source",
            )
            .unwrap();
            assert!(!source.extra.contains_key("claude_account_next"));
        }
    }
}

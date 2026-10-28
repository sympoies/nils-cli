//! Console transport must retain original authority through final admission.
use super::*;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
enum Case {
    Deleted(&'static str),
    Disappeared,
    PreparingSwitch(bool),
}

#[tokio::test]
async fn console_deleted_source_uses_explicit_account_and_shared_history_authority_once() {
    let lock = GlobalStateLock::new();
    for scope in ["account", "shared"] {
        exercise(&lock, Case::Deleted(scope)).await;
    }
}

#[tokio::test]
async fn console_inferred_pin_cannot_authorize_a_source_deleted_after_preflight() {
    let lock = GlobalStateLock::new();
    exercise(&lock, Case::Disappeared).await;
}

#[tokio::test]
async fn console_preparation_switch_requires_original_consent_at_final_admission() {
    let lock = GlobalStateLock::new();
    for explicit in [false, true] {
        exercise(&lock, Case::PreparingSwitch(explicit)).await;
    }
}

async fn exercise(lock: &GlobalStateLock, case: Case) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("target");
    fs::create_dir_all(&root).unwrap();
    let (state, broker, _) = fixture(&root);
    let config = root.join("account");
    let scope = if let Case::Deleted(scope) = case {
        scope
    } else {
        "account"
    };
    if scope == "shared" {
        let shared = root.join("shared-projects");
        fs::rename(config.join("projects"), &shared).unwrap();
        std::os::unix::fs::symlink(shared, config.join("projects")).unwrap();
    }
    let context = CliContext {
        state_dir: state.clone(),
        host: None,
    };
    let source_path = state.join("sessions/source/session.json");
    let prepared = root.join("pending.json");
    let hook = root.join("prepare-hook");
    executable(
        &hook,
        &if matches!(case, Case::PreparingSwitch(_)) {
            format!(
                "#!/bin/sh\ncp '{}' '{}'\n",
                prepared.display(),
                source_path.display()
            )
        } else {
            "#!/bin/sh\nexit 0\n".into()
        },
    );
    executable(
        &broker,
        &format!(
            "#!/bin/sh\ncase \"$1\" in\nlist) printf '%s\\n' '{{\"schema_version\":\"agent-session.account-broker.v2\",\"provider\":\"claude\",\"accounts\":[{{\"account\":\"original\"}},{{\"account\":\"other\"}}]}}' ;;\nhistory-root) printf '%s\\n' '{}' ;;\nmaterialize) '{}'; printf '%s\\n' '{}' ;;\n*) exit 93 ;;\nesac\n",
            json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":config,"history_scope":scope}),
            hook.display(),
            json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":config})
        ),
    );
    let _broker = EnvGuard::set(
        lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let mut pending = load_session_record(&context, "source").unwrap();
    crate::claude_account::queue_next(&mut pending, "other").unwrap();
    let pending_bytes = serde_json::to_vec(&pending).unwrap();
    fs::write(&prepared, &pending_bytes).unwrap();
    // Keep the omitted-account refusal independent of the real login history.
    let default = root.join("default");
    fs::create_dir_all(default.join("projects/project")).unwrap();
    fs::write(
        default.join(format!("projects/project/{ID}.jsonl")),
        format!("{}\n", json!({"sessionId":ID,"cwd":root,"type":"user"})),
    )
    .unwrap();
    let _default = EnvGuard::set(lock, "CLAUDE_CONFIG_DIR", default.to_str().unwrap());
    let provider = root.join("provider");
    executable(&provider, "#!/bin/sh\nexit 0\n");
    let _provider = EnvGuard::set(lock, "AGENT_SESSION_CLAUDE_BIN", provider.to_str().unwrap());
    let log = root.join("launch.log");
    let tmux = crate::serve::tests::startup_fixtures::live_resume_tmux(&root, &log);
    let target = router(crate::serve::tests::state(&state, Some(TOKEN), tmux));
    if matches!(case, Case::Deleted(_)) {
        fs::remove_dir_all(state.join("sessions/source")).unwrap();
        fs::remove_file(state.join("session-locks/source.lock")).unwrap();
    }
    let requests = Arc::new(StdMutex::new(Vec::new()));
    let seen = requests.clone();
    let source_dir = state.join("sessions/source");
    let source_lock = state.join("session-locks/source.lock");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let relay = Router::new().fallback(axum::routing::post(
        move |uri: axum::http::Uri, Json(body): Json<Value>| {
            let target = target.clone();
            let seen = seen.clone();
            let source_dir = source_dir.clone();
            let source_lock = source_lock.clone();
            async move {
                seen.lock()
                    .unwrap()
                    .push((uri.path().to_string(), body.clone()));
                let checking = uri.path() == "/api/coordination/resume-preflight/v1";
                let route = if checking {
                    "/resume-preflight/v1"
                } else {
                    assert_eq!(uri.path(), "/api/coordination/sessions/v1");
                    "/sessions/resume/v1"
                };
                let (status, result) =
                    post(target, route, Some(TOKEN), body["session"].clone()).await;
                if checking {
                    assert_eq!(status, StatusCode::OK, "{result}");
                    if let Case::Deleted(scope) = case {
                        assert_eq!(result["data"]["history_scope"], scope);
                    }
                    if matches!(case, Case::Disappeared) {
                        fs::remove_dir_all(source_dir).unwrap();
                        fs::remove_file(source_lock).unwrap();
                    }
                }
                (status, Json(result))
            }
        },
    ));
    let server = tokio::spawn(async move {
        axum::serve(listener, relay).await.unwrap();
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
    let explicit = matches!(case, Case::Deleted(_) | Case::PreparingSwitch(true));
    let mut request = json!({"operation_id":uuid::Uuid::now_v7().to_string(),"machine":"test-machine","no_parent":true,
        "session":{"agent":"claude","resume":ID,"id":"replacement","prompt":"Synthetic resume bootstrap","workspace":"example","title":"Resume fixture","title_mode":"pinned"}});
    if explicit {
        request["session"]["claude_account"] = json!("original");
    }
    let success = matches!(case, Case::Deleted(_) | Case::PreparingSwitch(true));
    for _ in 0..2 {
        let (_, result) = post(
            router(origin.clone()),
            "/sessions/caller/console-resume/v1",
            Some(&token),
            request.clone(),
        )
        .await;
        if success {
            assert_eq!(result["session"]["id"], "replacement", "{result}");
        } else {
            let code = if matches!(case, Case::Disappeared) {
                "resume-source-unknown"
            } else {
                "resume-account-transition-pending"
            };
            assert_eq!(result["error"]["code"], code, "{result}");
            assert_eq!(result["error"]["details"]["session_created"], false);
            assert_eq!(result["error"]["details"]["safe_to_retry"], false);
            assert!(!state.join("sessions/replacement").exists());
            if matches!(case, Case::PreparingSwitch(false)) {
                assert_eq!(
                    fs::read(&source_path).unwrap(),
                    pending_bytes,
                    "final refusal must preserve the newly pending source intent"
                );
            }
        }
    }
    server.abort();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2, "receipt replay must repeat neither hop");
    assert_eq!(requests[1].1["session"]["claude_account"], "original");
    assert_eq!(
        requests[1].1["session"]["resume_account_explicit"],
        explicit
    );
    assert_eq!(
        fs::read_to_string(&log)
            .unwrap_or_default()
            .matches("new-session")
            .count(),
        usize::from(success)
    );
    if success {
        let replacement = load_session_record(&context, "replacement").unwrap();
        assert_eq!(replacement.cwd, root.to_str().unwrap());
        assert_eq!(replacement.provider_resume.unwrap().session_id, ID);
        assert_eq!(
            replacement.extra["claude_account_binding"]["selected_account"],
            "original"
        );
        assert!(
            fs::read_to_string(replacement.prompt_file.unwrap())
                .unwrap()
                .contains("Synthetic resume bootstrap")
        );
        if matches!(case, Case::PreparingSwitch(true)) {
            assert!(
                !load_session_record(&context, "source")
                    .unwrap()
                    .extra
                    .contains_key("claude_account_next")
            );
        }
    }
}

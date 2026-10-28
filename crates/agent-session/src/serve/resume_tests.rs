use crate::serve::*;
use axum::http::Request;
use nils_test_support::{EnvGuard, GlobalStateLock};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;
use tower::ServiceExt;

const ID: &str = "00000000-0000-4000-8000-000000000007";
const TOKEN: &str = "fixture-daemon-token";

fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

fn fixture(root: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let state = root.join("state");
    let config = root.join("account");
    fs::create_dir_all(state.join("sessions/source")).unwrap();
    fs::create_dir_all(state.join("session-locks")).unwrap();
    fs::write(state.join("session-locks/source.lock"), "").unwrap();
    fs::create_dir_all(config.join("projects/project")).unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(config.join(".credentials.json"), "{}").unwrap();
    fs::write(
        config.join(format!("projects/project/{ID}.jsonl")),
        format!("{}\n", json!({"sessionId":ID,"cwd":root,"type":"user"})),
    )
    .unwrap();
    fs::write(state.join("sessions/source/session.json"), json!({
        "schema_version":"agent-session.session.v1","id":"source","agent":"claude","mode":"interactive",
        "cwd":root,"tmux_session":"source","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z",
        "provider_resume":{"provider":"claude","session_id":ID,"captured_at":"2026-01-01T00:00:00Z","capture_method":"fixture","resume_args":["--resume",ID]},
        "runtime":{"kind":"tmux","tmux_session":"source","generation":1,"started_at":"2026-01-01T00:00:00Z","launch_id":"source-runtime"},
        "tmux_runtime_never_launched":"source-runtime",
        "claude_account_binding":{"schema_version":"agent-session.claude-account-binding.v1","selected_account":"original","selection_source":"explicit","revision":1,"state":"bound","applied_runtime_id":"source-runtime","config_dir":config,"updated_at":"2026-01-01T00:00:00Z"}
    }).to_string()).unwrap();
    let broker = root.join("broker");
    executable(
        &broker,
        "#!/bin/sh\ncase \"$1\" in\nlist) printf '%s\\n' '{\"schema_version\":\"agent-session.account-broker.v2\",\"provider\":\"claude\",\"accounts\":[{\"account\":\"original\"}]}' ;;\n*) exit 93 ;;\nesac\n",
    );
    let tmux = root.join("tmux");
    executable(
        &tmux,
        "#!/bin/sh\nprintf 'no server running on fixture\\n' >&2\nexit 1\n",
    );
    (state, broker, tmux)
}

fn snapshot(path: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            out.extend(snapshot(&entry.unwrap().path()));
        }
    } else if path.is_file() {
        out.push((path.to_path_buf(), fs::read(path).unwrap()));
    }
    out.sort();
    out
}

async fn post(app: Router, route: &str, token: Option<&str>, value: Value) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method("POST")
        .uri(route)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .oneshot(req.body(Body::from(value.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn target_resume_preflight_is_authenticated_exact_and_observational() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let (state, broker, tmux) = fixture(temp.path());
    let _env = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let st = crate::serve::tests::state(&state, Some(TOKEN), tmux);
    let before = snapshot(temp.path());
    let body = json!({"agent":"claude","resume":ID});
    assert_eq!(
        post(
            router(st.clone()),
            "/resume-preflight/v1",
            None,
            body.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, result) = post(router(st), "/resume-preflight/v1", Some(TOKEN), body).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(
        result,
        json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
        "data":{"agent":"claude","provider_session_id":ID,"machine":"test-machine","account":"original"}})
    );
    assert_eq!(
        snapshot(temp.path()),
        before,
        "checks must not materialize credentials or create operations, locks, journals or records"
    );
}

#[tokio::test]
async fn target_resume_refusals_preserve_terminal_details_and_never_create() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let (state, broker, tmux) = fixture(temp.path());
    let _env = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let st = crate::serve::tests::state(&state, Some(TOKEN), tmux);
    let before = snapshot(temp.path());
    for (fields, code) in [
        (json!({"claude_account":"other"}), "resume-account-mismatch"),
        (json!({"agent_profile":"unsupported"}), "resume-unsupported"),
        (json!({"agent_args":["--resume",ID]}), "resume-unsupported"),
        (json!({"resume":"../bad"}), "resume-unsupported"),
        (json!({"agent":"dsh"}), "resume-unsupported"),
    ] {
        for route in ["/resume-preflight/v1", "/sessions/resume/v1"] {
            let mut body = json!({"agent":"claude","resume":ID});
            body.as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let (_, result) = post(router(st.clone()), route, Some(TOKEN), body).await;
            assert_eq!(result["ok"], false, "{route}: {result}");
            assert_eq!(result["error"]["code"], code, "{route}: {result}");
            assert_eq!(result["error"]["details"]["session_created"], false);
            assert_eq!(result["error"]["details"]["safe_to_retry"], false);
        }
    }
    assert_eq!(snapshot(temp.path()), before);
}

#[tokio::test]
async fn console_resume_operation_replay_never_rechecks_or_resubmits_and_keeps_terminal_refusals() {
    for refused in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut st = crate::serve::tests::state(
            temp.path(),
            Some(TOKEN),
            PathBuf::from("/unavailable/tmux"),
        );
        let token = caller(&st.context);
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let seen = requests.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(axum::routing::post(move |uri: axum::http::Uri, Json(body): Json<Value>| {
            let seen = seen.clone();
            async move {
                seen.lock().unwrap().push((uri.path().to_string(),body));
                if uri.path() == "/api/coordination/resume-preflight/v1" {
                    return (StatusCode::OK,Json(json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
                        "data":{"agent":"claude","provider_session_id":ID,"machine":"target","account":"original"}})));
                }
                if refused {
                    return (StatusCode::UNPROCESSABLE_ENTITY,Json(json!({"ok":false,"error":{"code":"resume-source-live",
                        "message":"fixture refusal","details":{"session_created":false,"safe_to_retry":false}}})));
                }
                (StatusCode::OK,Json(json!({"ok":true,"data":{"session":{"id":"new-child","provider_resume":{"session_id":ID}}}})))
            }
        }));
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Arc::get_mut(&mut st).unwrap().federation = Some(crate::coordination::remote::Config {
            machine: "origin".into(),
            url: format!("http://{address}"),
            token: "synthetic-relay-token".into(),
            ingress_token: "synthetic-ingress-token".into(),
        });
        let operation = uuid::Uuid::now_v7().to_string();
        let request = json!({"operation_id":operation,"machine":"target","no_parent":true,"role":"coordinator",
            "work":{"issues":[{"provider":"github","repository":"example/project","number":7}],"inherit":false},
            "session":{"agent":"claude","resume":ID,"prompt":"fixture bootstrap","workspace":"example","title":"Resume fixture","title_mode":"pinned","agent_args":["--model","fixture-model"]}});
        for _ in 0..2 {
            let (_, result) = post(
                router(st.clone()),
                "/sessions/caller/console-resume/v1",
                Some(&token),
                request.clone(),
            )
            .await;
            if refused {
                assert_eq!(result["error"]["code"], "resume-source-live", "{result}");
                assert_eq!(result["error"]["details"]["safe_to_retry"], false);
                assert_eq!(result["error"]["details"]["session_created"], false);
            } else {
                assert_eq!(result["session"]["id"], "new-child", "{result}");
            }
        }
        server.abort();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "an admitted operation must never perform another preflight or create on replay"
        );
        let create = &requests[1].1["session"];
        assert_eq!(create["resume"], ID);
        assert_eq!(create["claude_account"], "original");
        assert_eq!(create["prompt"], "fixture bootstrap");
        assert_eq!(create["role"], "coordinator");
        assert_eq!(create["workspace"], "example");
        assert_eq!(create["work"]["issues"][0]["number"], 7);
        assert_eq!(create["origin"]["parent"]["session_id"], "caller");
    }
}

#[tokio::test]
async fn target_create_rechecks_liveness_and_history_after_a_successful_check() {
    let lock = GlobalStateLock::new();
    let temp = tempfile::tempdir().unwrap();
    let (state, broker, tmux) = fixture(temp.path());
    let _env = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        &json!([broker]).to_string(),
    );
    let st = crate::serve::tests::state(&state, Some(TOKEN), tmux.clone());
    let body =
        json!({"agent":"claude","resume":ID,"id":"must-not-exist","prompt":"fixture bootstrap"});
    assert_eq!(
        post(
            router(st.clone()),
            "/resume-preflight/v1",
            Some(TOKEN),
            json!({"agent":"claude","resume":ID})
        )
        .await
        .0,
        StatusCode::OK
    );
    executable(&tmux, "#!/bin/sh\nexit 0\n");
    let (_, result) = post(
        router(st.clone()),
        "/sessions/resume/v1",
        Some(TOKEN),
        body.clone(),
    )
    .await;
    assert_eq!(result["error"]["code"], "resume-source-live");
    executable(
        &tmux,
        "#!/bin/sh\nprintf 'no server running on fixture\\n' >&2\nexit 1\n",
    );
    let history = temp
        .path()
        .join(format!("account/projects/project/{ID}.jsonl"));
    let script = fs::read_to_string(&broker).unwrap().replace("*) exit 93 ;;", &format!("materialize) rm -- {}; printf '%s\\n' '{}' ;;\n*) exit 93 ;;",
        shell_words::quote(history.to_str().unwrap()),json!({"schema_version":"agent-session.account-broker.v2","provider":"claude","account":"original","config_dir":temp.path().join("account")})));
    executable(&broker, &script);
    let (_, result) = post(router(st), "/sessions/resume/v1", Some(TOKEN), body).await;
    assert_eq!(
        result["error"]["code"], "resume-history-missing",
        "{result}"
    );
    assert_eq!(result["error"]["details"]["session_created"], false);
    assert!(!state.join("sessions/must-not-exist").exists());
    assert_eq!(fs::read_dir(state.join("sessions")).unwrap().count(), 1);
}

#[tokio::test]
async fn legacy_create_cannot_ignore_a_dedicated_resume_request() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let st = crate::serve::tests::state(&state, Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    let (_, result) = post(
        router(st),
        "/sessions",
        Some(TOKEN),
        json!({"agent":"claude","resume":ID,"prompt":"bootstrap"}),
    )
    .await;
    assert_eq!(result["error"]["code"], "resume-unsupported");
    assert_eq!(result["error"]["details"]["session_created"], false);
    assert!(
        !state.exists(),
        "misrouted resume must refuse before any create activity"
    );
}

fn caller(context: &CliContext) -> String {
    use crate::coordination::{capability_path, digest_bytes, lock_registry, now_epoch};
    use nils_common::fs::{SECRET_FILE_MODE, write_atomic};
    let token = "synthetic-runtime-capability-000000000000001";
    let directory = context.state_dir.join("sessions/caller/coordination");
    fs::create_dir_all(&directory).unwrap();
    for dir in [
        context.state_dir.join("sessions"),
        directory.parent().unwrap().to_owned(),
        directory.clone(),
    ] {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let record = json!({"schema_version":"agent-session.session.v1","id":"caller","agent":"codex","mode":"interactive","cwd":".",
        "tmux_session":"caller","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z",
        "runtime":{"kind":"tmux","tmux_session":"caller","generation":1,"started_at":"2026-01-01T00:00:00Z","launch_id":"current"}});
    write_atomic(
        &directory.parent().unwrap().join("session.json"),
        record.to_string().as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    write_atomic(
        &capability_path(context, "caller", "current"),
        token.as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    write_atomic(
        &directory.join("heartbeat"),
        format!("current:{}\n", now_epoch()).as_bytes(),
        SECRET_FILE_MODE,
    )
    .unwrap();
    let mut registry = lock_registry(context).unwrap();
    let mut data = serde_json::to_value(&registry.registry).unwrap();
    data["brokers"]["caller"] = json!({"session_id":"caller","incarnation":"current",
        "capability_digest":digest_bytes(token.as_bytes()),"generation":1,"state":"ready","heartbeat_at":"2026-01-01T00:00:00Z","heartbeat_epoch":now_epoch()});
    registry.registry = serde_json::from_value(data).unwrap();
    registry.save().unwrap();
    token.into()
}

#[tokio::test]
async fn console_preflight_binds_identity_and_target_without_operation_or_auth_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut st =
        crate::serve::tests::state(temp.path(), Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    let token = caller(&st.context);
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let requests = seen.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new().fallback(axum::routing::post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let requests = requests.clone();
        async move {
            assert_eq!(headers["authorization"], "Bearer synthetic-relay-token");
            requests.lock().unwrap().push(body);
            Json(json!({"schema_version":"cli.agent-session.resume-preflight.v1","ok":true,
                "data":{"agent":"claude","provider_session_id":ID,"machine":"target","account":"original"}}))
        }
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Arc::get_mut(&mut st).unwrap().federation = Some(crate::coordination::remote::Config {
        machine: "origin".into(),
        url: format!("http://{address}"),
        token: "synthetic-relay-token".into(),
        ingress_token: "synthetic-ingress-token".into(),
    });
    let before = snapshot(temp.path());
    let request = json!({"machine":"target","session":{"agent":"claude","resume":ID,"claude_account":"original"}});
    let (status, result) = post(
        router(st),
        "/sessions/caller/resume-preflight/v1",
        Some(&token),
        request,
    )
    .await;
    server.abort();
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["data"]["machine"], "target");
    assert_eq!(
        *seen.lock().unwrap(),
        vec![
            json!({"source_session_id":"caller","source_incarnation":"current","machine":"target",
        "session":{"agent":"claude","resume":ID,"claude_account":"original"}})
        ]
    );
    assert_eq!(snapshot(temp.path()), before);
}

#[tokio::test]
async fn console_resume_route_requires_a_resume_before_any_start_activity() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let st = crate::serve::tests::state(&state, Some(TOKEN), PathBuf::from("/unavailable/tmux"));
    let (_, result) = post(
        router(st),
        "/sessions/caller/console-resume/v1",
        Some(TOKEN),
        json!({"session":{"agent":"claude"}}),
    )
    .await;
    assert_eq!(result["error"]["code"], "resume-unsupported");
    assert_eq!(result["error"]["details"]["session_created"], false);
    assert!(!state.exists());
}

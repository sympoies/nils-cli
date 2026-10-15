use super::*;
use pretty_assertions::{assert_eq, assert_ne};

#[tokio::test]
async fn claude_account_switch_queues_while_busy_and_restarts_only_at_idle() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let running = tmp.path().join("running");
    let pane_pid = tmp.path().join("pane.pid");
    let tmux = claude_live_tmux(tmp.path(), &log, &running, &pane_pid);
    let st = state(tmp.path(), Some(TOKEN), tmux);
    let first_pane = TestProcessGroup::spawn();
    fs::write(&pane_pid, first_pane.pid().to_string()).unwrap();
    let _first_pane = first_pane.into_background_reaper();

    let (status, body) = call(
            router(st.clone()),
            post_json(
                "/sessions",
                Some(TOKEN),
                json!({"agent":"claude","id":"claude-live","cwd":cwd,"claude_account":"alpha", "workspace":"example-workspace", "origin":{"parent":{"session_id":"origin-parent","machine":"worker-host"}}}),
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let record = load_session_record(&st.context, "claude-live").unwrap();
    let origin = crate::origin::Fields::of(&record);
    let first_launch = record.runtime.as_ref().unwrap().launch_id.clone();
    let provider_session_id = record.provider_resume.as_ref().unwrap().session_id.clone();

    // A busy turn keeps the switch queued without restarting the session.
    let turn = |event_id: &str, kind: &str| {
        serde_json::from_value(json!({
            "schema_version": crate::activity::TURN_EVENT_VERSION,
            "event_id": event_id,
            "runtime_id": first_launch,
            "provider": "claude",
            "provider_session_id": provider_session_id,
            "provider_turn_id": "turn-claude-switch",
            "kind": kind,
            "confidence": "authoritative"
        }))
        .unwrap()
    };
    crate::activity::ingest_event(&st.context, &record.id, turn("turn-start", "turn_started"))
        .unwrap();
    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-live/account",
            Some(TOKEN),
            json!({"account":"beta","expected_session_incarnation":first_launch}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["data"]["session_incarnation"], first_launch.as_str());
    assert_eq!(body["data"]["claude_account"]["selected_account"], "alpha");
    assert_eq!(body["data"]["claude_account"]["next"]["account"], "beta");
    let launches = |log: &Path| {
        fs::read_to_string(log)
            .unwrap()
            .lines()
            .filter(|line| line.starts_with("new-session"))
            .map(str::to_string)
            .collect::<Vec<_>>()
    };
    assert_eq!(launches(&log).len(), 1, "a busy turn must not restart");

    // A nickname the broker does not list is refused before queueing.
    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-live/account",
            Some(TOKEN),
            json!({"account":"delta","expected_session_incarnation":first_launch}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-unknown");

    crate::activity::ingest_event(&st.context, &record.id, turn("turn-done", "turn_completed"))
        .unwrap();

    // An idle switch materializes the next account before stopping
    // anything. A broker refusal leaves the running session and its
    // durable account state untouched.
    let before_refusal = session_record_json(tmp.path(), "claude-live");
    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-live/account",
            Some(TOKEN),
            json!({"account":"gamma","expected_session_incarnation":first_launch}),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    assert_eq!(body["error"]["code"], "claude-account-switch-refused");
    assert_eq!(
        body["error"]["details"]["cause"],
        "claude-account-broker-rejected"
    );
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        !calls.contains("kill-session") && !calls.contains("if-shell"),
        "a refused preflight must never stop the runtime: {calls:?}"
    );
    assert!(running.exists(), "the session must still be running");
    assert_eq!(launches(&log).len(), 1);
    let after_refusal = session_record_json(tmp.path(), "claude-live");
    assert_eq!(
        after_refusal["claude_account_binding"],
        before_refusal["claude_account_binding"]
    );
    assert_eq!(
        after_refusal["claude_account_next"], before_refusal["claude_account_next"],
        "a refused preflight keeps the previously queued intent unchanged"
    );

    // At the idle boundary the switch restarts the runtime. This fixture
    // pane has no dedicated control group, so the verified stop refuses
    // and the switch fails safe: nothing relaunches, the applied account
    // is unchanged, and the intent stays queued for the next resume.
    let (status, body) = call(
        router(st.clone()),
        put_json(
            "/sessions/claude-live/account",
            Some(TOKEN),
            json!({"account":"beta","expected_session_incarnation":first_launch}),
        ),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["error"]["code"], "session-termination-failed");
    assert_eq!(launches(&log).len(), 1, "a refused stop must not relaunch");
    let persisted = load_session_record(&st.context, "claude-live").unwrap();
    let account = crate::claude_account::view_for_record(&persisted).unwrap();
    assert_eq!(account.selected_account.as_deref(), Some("alpha"));
    assert!(crate::claude_account::has_queued_next(&persisted));
    assert_eq!(
        broker
            .calls()
            .matches("materialize --provider claude --account beta")
            .count(),
        1,
        "the idle switch preflights the next account once, before the stop"
    );
    let preserved = load_session_record(&st.context, "claude-live").unwrap();
    assert_eq!(crate::origin::Fields::of(&preserved), origin);
}

#[tokio::test]
async fn origin_serve_create_and_projection_include_nullable_defaults_and_workspace() {
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    let log = tmp.path().join("tmux.log");
    let st = state(tmp.path(), Some(TOKEN), logging_tmux(tmp.path(), &log));
    for (id, fields) in [
        ("unassigned", json!({})),
        (
            "assigned",
            json!({"workspace":"example-workspace", "origin":{"parent":{"session_id":"remote-parent","machine":"other-host"}}}),
        ),
    ] {
        let mut request = json!({"agent":"claude", "id":id, "cwd":tmp.path()});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let (status, body) = call(
            router(st.clone()),
            post_json("/sessions", Some(TOKEN), request),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let expected_origin = fields
            .get("origin")
            .cloned()
            .unwrap_or(json!({"parent":null}));
        let expected_workspace = fields.get("workspace").cloned().unwrap_or(Value::Null);
        assert_eq!(
            body["data"]["session"].get("origin"),
            Some(&expected_origin)
        );
        assert_eq!(
            body["data"]["session"].get("workspace"),
            Some(&expected_workspace)
        );
        let record = session_record_json(tmp.path(), id);
        assert_eq!(record["origin"], expected_origin);
        assert_eq!(record["workspace"], expected_workspace);
        let (status, list) = call(router(st.clone()), get("/sessions")).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let entry = list["data"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap();
        assert_eq!(entry["origin"], expected_origin);
        assert_eq!(entry["workspace"], expected_workspace);
    }
    for fields in [
        json!({"workspace":"bad/path"}),
        json!({"origin":{"parent":{"session_id":"parent","machine":"bad/path"}}}),
    ] {
        let mut request = json!({"agent":"claude", "id":"invalid", "cwd":tmp.path()});
        request
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        let (status, body) = call(
            router(st.clone()),
            post_json("/sessions", Some(TOKEN), request),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(!tmp.path().join("sessions/invalid/session.json").exists());
    }
}

#[test]
fn claude_legacy_switch_retires_the_verified_stop_and_applies_next_account() {
    use crate::coordination::broker::test_support;
    let lock = GlobalStateLock::new();
    let tmp = tempfile::TempDir::new().unwrap();
    let cwd = tmp.path().join("repo");
    fs::create_dir_all(&cwd).unwrap();
    let _broker = claude_broker_fixture(&lock, tmp.path());
    let context = CliContext {
        state_dir: tmp.path().to_path_buf(),
        host: None,
    };
    seed_bound_claude_session(tmp.path(), "prior-switch", &cwd, "alpha");
    let mut record = load_session_record(&context, "prior-switch").unwrap();
    let origin = crate::origin::Fields {
        origin: crate::origin::Origin {
            parent: Some(crate::origin::Parent {
                session_id: "origin-parent".into(),
                machine: "worker-host".into(),
            }),
        },
        workspace: Some("example-workspace".into()),
    };
    origin.store(&mut record);
    let old = record.runtime.as_ref().unwrap().launch_id.clone();
    let mut identity = test_support::process_group_identity(test_support::exited_process_group());
    identity["launch_id"] = json!(old);
    record.extra.remove("tmux_runtime_never_launched");
    record
        .extra
        .insert("delete_tmux_identity".to_string(), identity.clone());
    crate::claude_account::queue_next(&mut record, "beta").unwrap();
    crate::write_session_record(&context, &record).unwrap();
    test_support::seed_live_broker(&context, &record.id, &old, identity);
    test_support::make_legacy_broker(&context, &record.id, true);
    let tmux = resume_tmux(tmp.path(), &tmp.path().join("tmux.log"));
    let script = fs::read_to_string(&tmux).unwrap().replace(
            r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then"#,
            r#"if [ -n "$heartbeat" ] && [ -n "$incarnation" ]; then gate="$(dirname "$heartbeat")/broker-provisioned"; (i=0; while [ ! -f "$gate" ] && [ "$i" -lt 1000 ]; do i=$((i + 1)); sleep 0.01; done; printf '%s:%s\n' "$incarnation" "$(date +%s)" > "$heartbeat"; chmod 600 "$heartbeat") >/dev/null 2>&1 & fi; if false; then"#,
        );
    fs::write(&tmux, script).unwrap();
    let (incarnation, _) =
        crate::session_account::resume_after_switch_stop(&context, &record.id, &tmux)
            .expect("verified older runtime stop must resume under the queued account");
    assert_ne!(incarnation.as_deref(), Some(old.as_str()));
    let resumed = load_session_record(&context, &record.id).unwrap();
    let view = serde_json::to_value(crate::claude_account::view_for_record(&resumed)).unwrap();
    assert_eq!(view["selected_account"], "beta");
    assert_eq!(crate::origin::Fields::of(&resumed), origin);
    assert!(view.get("next").is_none());
}

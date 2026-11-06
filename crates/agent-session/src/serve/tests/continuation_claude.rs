//! Synthetic native-module receiver at the generic scheduler boundary.
use super::*;
use crate::send::claude_notice::{runtime, sha256};
use pretty_assertions::assert_eq;
use std::io::Read;

#[test]
fn generic_claude_notice_transfers_lock_without_cancelling_its_recovery() {
    let global = GlobalStateLock::new();
    let _writers = EnvGuard::set(&global, crate::notice::WRITERS_ENV, "1");
    let tmp = tempfile::tempdir_in("/tmp").unwrap();
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let id = "claude-source";
    let launch = seed_claude_prompt_session(tmp.path(), id);
    let tmux = guard_tmux(tmp.path(), id);
    let st = state(tmp.path(), Some(TOKEN), tmux.clone());
    let mut raw = serde_json::to_value(load_session_record(&st.context, id).unwrap()).unwrap();
    raw["provider_resume"] = json!({"provider":"claude","session_id":"provider-one","resume_args":[],"capture_method":"fixture","captured_at":"2026-01-01T00:00:00Z"});
    raw["runtime"]["agent_profile_provider_config_dir"] = json!(tmp.path().join("provider"));
    let record: SessionRecord = serde_json::from_value(raw).unwrap();
    crate::write_session_record(&st.context, &record).unwrap();
    let transcript = tmp
        .path()
        .join("provider/projects/example/provider-one.jsonl");
    fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({"type":"summary","sessionId":"provider-one","cwd":record.cwd})
        ),
    )
    .unwrap();
    let dir = runtime::module_dir(tmp.path(), &record).unwrap();
    fs::create_dir_all(dir.join("hooks")).unwrap();
    for path in [&dir, &dir.join("hooks")] {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let socket_parent = tmp.path().join("agent-session");
    fs::create_dir(&socket_parent).unwrap();
    fs::set_permissions(&socket_parent, fs::Permissions::from_mode(0o700)).unwrap();
    let namespace = sha256(format!("{}\0{id}\0{launch}", tmp.path().display()).as_bytes());
    let socket = socket_parent.join(format!("cn-{}.sock", &namespace[..16]));
    let launch_binding = crate::send::claude_notice::Launch::from_record(&record).unwrap();
    let module = sha256(runtime::MODULE.as_bytes());
    let nonce = "a".repeat(64);
    let binding = json!({"launch":launch_binding,"provider":"claude","provider_session_id":"provider-one",
        "provider_version":"2.1.296","module_sha256":module});
    for (name, value) in [
        (
            "bootstrap.json",
            json!({"schema_version":"agent-session.claude-notice-runtime.v1","launch":launch_binding,
            "module_sha256":module,"provider_version":"2.1.296","socket_path":socket,"nonce":nonce}),
        ),
        (
            "ready.json",
            json!({"schema_version":"agent-session.claude-notice-ready.v1","binding":binding,"nonce":nonce,"terminal":true,"timer_started":true}),
        ),
        (
            "offer.json",
            json!({"schema_version":"agent-session.claude-notice-offer.v1","state":"idle"}),
        ),
    ] {
        crate::write_private_file(&dir.join(name), value.to_string().as_bytes()).unwrap();
    }
    crate::write_private_file(&dir.join("hooks/register.js"), runtime::MODULE.as_bytes()).unwrap();
    assert!(runtime::readiness(&st.context, &record).is_some());
    activity::activate_runtime(&st.context, &record).unwrap();
    auto_resume::set_enabled(&st.context, id, true, "2020-01-01T00:00:00Z").unwrap();
    let failed=activity::ingest_event(&st.context,id,serde_json::from_value(json!({
        "schema_version":activity::TURN_EVENT_VERSION,"event_id":"claude-failed","runtime_id":launch,
        "provider":"claude","provider_session_id":"provider-one","provider_turn_id":"interrupted-turn",
        "kind":"turn_failed","failure_reason":"usage_exhausted","confidence":"authoritative"})).unwrap()).unwrap();
    auto_resume::arm_usage_exhaustion(
        &st.context,
        id,
        failed
            .turn_state
            .last_turn
            .unwrap()
            .provider_turn_id
            .unwrap(),
        failed.turn_state.revision,
        "2020-01-01T00:00:01Z",
    )
    .unwrap();
    auto_resume::tick(
        &st.context,
        id,
        1_577_836_801,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: true,
            exhausted_reset_epochs: vec![1_577_836_810],
            soonest_reset_epoch: None,
        },
        |_, _lock| panic!("must schedule first"),
    )
    .unwrap();
    let context = st.context.clone();
    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let offer = loop {
            let offer: Value =
                serde_json::from_slice(&fs::read(dir.join("offer.json")).unwrap()).unwrap();
            if offer["op"].is_string() {
                break offer;
            }
            assert!(
                Instant::now() < deadline,
                "native notice offer did not arrive"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let authorized = post(
            &socket,
            "/authorize",
            &json!({"schema_version":"agent-session.claude-notice-authorize.v1",
            "op":offer["op"],"binding":binding,"nonce":nonce,"terminal":true,"box_empty":true,"cursor":0}),
        );
        let text = authorized["text"].as_str().unwrap();
        let parsed = crate::notice::parse_notice(text).unwrap();
        assert_eq!(parsed.metadata.kind(), "runtime.resume");
        assert_eq!(parsed.body, auto_resume::CONTINUATION_MESSAGE);
        let guard = crate::try_acquire_session_record_lock(&context, id)
            .unwrap()
            .expect("native ingress must acquire the transferred record lock");
        let mut current = load_session_record(&context, id).unwrap();
        current.extra.insert("synthetic_hook".into(), json!(true));
        crate::write_session_record(&context, &current).unwrap();
        drop(guard);
        writeln!(fs::OpenOptions::new().append(true).open(transcript).unwrap(),"{}",json!({"type":"user","sessionId":"provider-one",
            "uuid":"11111111-1111-4111-8111-111111111111","version":"2.1.296","promptSource":"system","isSidechain":false,
            "origin":{"kind":"plugin","name":"agent-session-claude-notice","asUser":true},"message":{"role":"user","content":text}})).unwrap();
        post(
            &socket,
            "/result",
            &json!({"schema_version":"agent-session.claude-notice-result.v1","op":offer["op"],"binding":binding,
            "nonce":nonce,"outcome":"accepted","after_empty":true,"returned_text":text,
            "origin":{"kind":"plugin","name":"agent-session-claude-notice","as_user":true}}),
        );
        text.to_owned()
    });
    let outcome = auto_resume::tick(
        &st.context,
        id,
        1_577_837_801,
        &UsageSnapshot {
            authoritative: true,
            has_exhausted_windows: false,
            exhausted_reset_epochs: vec![],
            soonest_reset_epoch: None,
        },
        |record, lock| {
            crate::send_auto_resume_input(
                &st.context,
                record,
                lock,
                auto_resume::CONTINUATION_MESSAGE,
                &tmux,
            )
        },
    )
    .unwrap();
    let sent = peer.join().unwrap();
    assert_eq!(outcome, auto_resume::TickOutcome::Resumed);
    assert!(auto_resume::view_for_record(&st.context, &record).enabled);
    assert_eq!(
        load_session_record(&st.context, id).unwrap().extra["synthetic_hook"],
        true
    );
    let history = provider_history::stable_history_id("claude", None, "provider-one");
    let catalog = HistoryCatalog::new(
        vec![HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root: tmp.path().join("provider/projects"),
        }],
        provider_history::archive_root(tmp.path()),
        provider_history::star_root(tmp.path()),
    )
    .with_session_state_dir(tmp.path().to_owned());
    let page = catalog
        .messages_with_parts(&history, None, 10, HistoryMessageDirection::Forward, true)
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].role, "notice");
    assert_eq!(page.messages[0].text, sent);
    assert!(
        !tmp.path().join("submitted").exists(),
        "the continuation must not press physical Enter"
    );
}

fn post(socket: &Path, path: &str, value: &Value) -> Value {
    let mut stream = std::os::unix::net::UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let body = value.to_string();
    write!(stream,"POST {path} HTTP/1.1\r\nHost: notice.invalid\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    let mut response = String::new();
    stream
        .take(128 * 1024)
        .read_to_string(&mut response)
        .unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200 "), "{head}");
    serde_json::from_str(body).unwrap()
}

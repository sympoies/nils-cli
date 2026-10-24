use super::*;
use pretty_assertions::assert_eq;
use std::fs;

fn local_backend_record(agent: &str) -> crate::SessionRecord {
    serde_json::from_value(serde_json::json!({
        "schema_version":"agent-session.session.v1", "id":"local-fixture", "agent":agent,
        "mode":"interactive", "title":null, "cwd":"/workspace/project", "tmux_session":"fixture",
        "prompt_file":null, "log_file":null, "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
        "runtime":{"kind":"codex_app_server", "tmux_session":"fixture", "generation":1,
          "started_at":"2026-01-01T00:00:00Z", "launch_id":"local-fixture-runtime", "codex_app_server_protocol":"v2",
          "codex_app_server_socket":"/fixture/socket", "codex_app_server_proxy":"/fixture/proxy",
          "codex_app_server_thread_handoff":"/fixture/handoff", "codex_app_server_thread_attached":"/fixture/attached"}
    })).unwrap()
}

#[test]
fn configured_local_claude_window_reaches_public_usage() {
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let root = tmp.path().join("provider");
    fs::create_dir_all(root.join("projects/project")).unwrap();
    let mut row: Value = serde_json::from_str(fixture("claude").lines().next().unwrap()).unwrap();
    row["message"]["model"] = serde_json::json!("custom-local-model");
    fs::write(
        root.join("projects/project/provider-session.jsonl"),
        format!("{row}\n"),
    )
    .unwrap();
    let mut record = local_backend_record("claude");
    record.runtime.as_mut().unwrap().extra.insert(
        "agent_profile_provider_config_dir".into(),
        serde_json::json!(root),
    );
    record.provider_resume = Some(serde_json::from_value(serde_json::json!({
        "provider":"claude", "session_id":"provider-session", "captured_at":"2026-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]
    })).unwrap());
    record.agent_args = vec!["--settings".into(), serde_json::json!({"env":{
        "CLAUDE_CODE_MAX_CONTEXT_TOKENS":"131072", "ANTHROPIC_BASE_URL":"http://127.0.0.1:9/private-canary"
    }}).to_string()];
    let usage = require_usage(projection(&context, &record));
    assert_eq!(usage.used_tokens, Some(130));
    assert_eq!(usage.window_tokens, Some(131072));
    let view = serde_json::to_value(crate::session_view(
        &context,
        &record,
        Some("stopped".into()),
        None,
    ))
    .unwrap();
    assert_eq!(view["context_usage"]["window_tokens"], 131072);
    let board = crate::board::project_record(&view, "machine", None, false).unwrap();
    assert_eq!(board["context_usage"], view["context_usage"]);
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("private-canary")
    );
    for invalid in [
        serde_json::json!("0"),
        serde_json::json!("-1"),
        serde_json::json!("not-a-capacity"),
        serde_json::json!(131072),
    ] {
        record.agent_args = vec![
            "--settings".into(),
            serde_json::json!({"env":{"CLAUDE_CODE_MAX_CONTEXT_TOKENS":invalid}}).to_string(),
        ];
        assert_eq!(
            require_usage(projection(&context, &record)).window_tokens,
            None
        );
    }
    row["message"]["model"] = serde_json::json!("claude-haiku-4-5-20251001");
    fs::write(
        root.join("projects/project/provider-session.jsonl"),
        format!("{row}\n"),
    )
    .unwrap();
    assert_eq!(
        require_usage(projection(&context, &record)).window_tokens,
        Some(200000),
        "invalid override keeps the known-model fallback"
    );
}

#[test]
fn local_backend_accounts_are_explicitly_inapplicable_for_both_providers() {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let _claude = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    let _codex = EnvGuard::set(
        &lock,
        "AGENT_SESSION_CODEX_ACCOUNT_BROKER",
        r#"["/configured/broker"]"#,
    );
    for agent in ["claude", "codex"] {
        let mut record = local_backend_record(agent);
        record.agent_args = if agent == "claude" {
            vec!["--settings".into(), serde_json::json!({"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9/private-canary"}}).to_string()]
        } else {
            [
                "model_provider=\"fixture_local\"",
                "model_providers.fixture_local.requires_openai_auth=false",
                "model_providers.fixture_local.base_url=\"http://127.0.0.1:9/private-canary\"",
            ]
            .into_iter()
            .flat_map(|value| ["-c".to_string(), value.to_string()])
            .collect()
        };
        let view = if agent == "claude" {
            serde_json::to_value(crate::claude_account::view_for_record(&record).unwrap()).unwrap()
        } else {
            serde_json::to_value(crate::codex_account::view_for_record(&record)).unwrap()
        };
        assert_eq!(view["applicable"], false, "{agent}");
        assert_eq!(view["supported"], false, "{agent}");
        assert_eq!(view["state"], "unsupported", "{agent}");
        assert!(!view.to_string().contains("private-canary"));
        record.agent_args.clear();
        let normal = if agent == "claude" {
            serde_json::to_value(crate::claude_account::view_for_record(&record).unwrap()).unwrap()
        } else {
            serde_json::to_value(crate::codex_account::view_for_record(&record)).unwrap()
        };
        assert_eq!(normal["applicable"], true, "{agent}");
        assert_eq!(normal["supported"], true, "{agent}");
    }
}

fn fixture(provider: &str) -> &'static str {
    match provider {
        "claude" => include_str!("../tests/fixtures/context-usage/claude.jsonl"),
        "codex" => include_str!("../tests/fixtures/context-usage/codex.jsonl"),
        _ => unreachable!(),
    }
}

fn require_usage(usage: Option<ContextUsage>) -> ContextUsage {
    assert!(usage.is_some(), "context usage missing");
    usage.unwrap()
}

#[test]
fn provider_fixtures_normal_compacted_unknown_and_stale() {
    for (provider, count, model, window) in [
        ("claude", 1, "claude-haiku-4-5-20251001", 200000),
        ("codex", 3, "example-model", 272000),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("provider.jsonl");
        let rows: Vec<_> = fixture(provider).split_inclusive('\n').collect();
        fs::write(&path, rows[..count].concat()).unwrap();
        let mut reader = Reader::default();
        let normal = require_usage(reader.read(&path, provider, "provider-session"));
        assert_eq!(normal.used_tokens, Some(130));
        assert_eq!(normal.window_tokens, Some(window));
        assert_eq!(normal.model.as_deref(), Some(model));
        // Provider timestamps are historical observations, never poll time.
        assert_eq!(normal.observed_at.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(
            normal.source.as_deref(),
            Some(if provider == "claude" {
                "claude-transcript"
            } else {
                "codex-rollout"
            })
        );
        let bytes = reader.bytes_read;
        assert_eq!(
            reader.read(&path, provider, "provider-session"),
            Some(normal)
        );
        assert_eq!(reader.bytes_read, bytes);
        fs::write(&path, rows[..count + 1].concat()).unwrap();
        assert_eq!(reader.read(&path, provider, "provider-session"), None);
        fs::write(&path, fixture(provider)).unwrap();
        let compacted = require_usage(reader.read(&path, provider, "provider-session"));
        assert_eq!(compacted.used_tokens, Some(17));
        assert_eq!(compacted.window_tokens, None);
        assert_eq!(
            compacted.observed_at.as_deref(),
            Some("2026-01-01T00:02:00Z")
        );
        fs::remove_file(&path).unwrap();
        assert_eq!(reader.read(&path, provider, "provider-session"), None);
    }
}

#[test]
fn missing_transcripts_are_optional() {
    let tmp = tempfile::tempdir().unwrap();
    for provider in ["claude", "codex"] {
        assert_eq!(
            Reader::default().read(
                &tmp.path().join("missing.jsonl"),
                provider,
                "provider-session"
            ),
            None
        );
    }
}

#[test]
fn shared_views_and_board_expose_only_context_facts() {
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let mut record: crate::SessionRecord = serde_json::from_value(serde_json::json!({
        "schema_version":"agent-session.session.v1", "id":"session", "agent":"claude", "mode":"oneshot", "title":null,
        "cwd":"/workspace/project", "tmux_session":"session", "prompt_file":null, "log_file":null,
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z"
    })).unwrap();
    let view = crate::session_view(&context, &record, Some("stopped".into()), None);
    let mut view = serde_json::to_value(view).unwrap();
    assert!(view.get("context_usage").is_none());
    let provider_root = tmp.path().join("provider-config");
    let projects = provider_root.join("projects/project");
    fs::create_dir_all(&projects).unwrap();
    let transcript = projects.join("provider-session.jsonl");
    fs::write(
        &transcript,
        fixture("claude").split_inclusive('\n').next().unwrap(),
    )
    .unwrap();
    record.provider_resume = Some(serde_json::from_value(serde_json::json!({
        "provider":"claude", "session_id":"provider-session", "captured_at":"2026-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]
    })).unwrap());
    record.runtime = Some(serde_json::from_value(serde_json::json!({
        "kind":"tmux", "tmux_session":"session", "generation":1, "started_at":"2026-01-01T00:00:00Z", "launch_id":"fixture-runtime", "agent_profile_provider_config_dir":provider_root
    })).unwrap());
    let actual = crate::session_view(&context, &record, Some("stopped".into()), None);
    let actual = serde_json::to_value(actual).unwrap();
    assert!(actual["context_usage"].is_object(), "context usage missing");
    assert_eq!(actual["context_usage"]["used_tokens"], 130);
    let list = serde_json::to_value(crate::board::list_records(
        &[crate::session_view(
            &context,
            &record,
            Some("stopped".into()),
            None,
        )],
        "machine",
    ))
    .unwrap();
    assert_eq!(list[0]["context_usage"], actual["context_usage"]);
    let board_actual = crate::board::project_record(&actual, "machine", None, false).unwrap();
    assert_eq!(board_actual["context_usage"], actual["context_usage"]);
    let closed =
        crate::board::closed_record(&context, &record, crate::board::CloseReason::Deleted).unwrap();
    assert_eq!(closed["context_usage"], actual["context_usage"]);
    fs::remove_file(&transcript).unwrap();
    let missing = serde_json::to_value(crate::session_view(
        &context,
        &record,
        Some("stopped".into()),
        None,
    ))
    .unwrap();
    assert!(missing.get("context_usage").is_none());
    view["context_usage"] = serde_json::json!({"used_tokens":130, "window_tokens":200000, "source":"claude-transcript", "observed_at":null, "model":null, "private_text":"DO NOT PROJECT"});
    let board = crate::board::project_record(&view, "machine", None, false).unwrap();
    assert_eq!(
        board["context_usage"],
        serde_json::json!({"used_tokens":130, "window_tokens":200000, "source":"claude-transcript", "observed_at":null, "model":null})
    );
}

#[test]
fn bounded_tail_torn_lines_and_replacement_do_not_reuse_usage() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider.jsonl");
    let row = fixture("claude").split_inclusive('\n').next().unwrap();
    let mut reader = Reader::default();
    let mut body = "x".repeat(300000);
    body.push('\n');
    body.push_str(row);
    fs::write(&path, &body).unwrap();
    assert_eq!(
        require_usage(reader.read(&path, "claude", "provider-session")).used_tokens,
        Some(130)
    );
    assert!(reader.bytes_read <= 256 * 1024);
    // A partial event at EOF must not count until its newline arrives.
    fs::write(&path, row.trim_end()).unwrap();
    assert_eq!(reader.read(&path, "claude", "provider-session"), None);
    fs::write(&path, row).unwrap();
    assert!(reader.read(&path, "claude", "provider-session").is_some());
    let replacement = tmp.path().join("replacement.jsonl");
    fs::write(&replacement, "{}\n").unwrap();
    fs::rename(replacement, &path).unwrap();
    assert_eq!(reader.read(&path, "claude", "provider-session"), None);
    // Matching usage outside the bounded tail is unknown, not stale cache.
    fs::write(&path, format!("{row}{}\n", "x".repeat(300000))).unwrap();
    assert_eq!(reader.read(&path, "claude", "provider-session"), None);
}

#[test]
fn foreign_and_sidechain_claude_usage_is_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider.jsonl");
    let row = fixture("claude").split_inclusive('\n').next().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(row).unwrap();
    value["sessionId"] = serde_json::json!("different-session");
    fs::write(&path, format!("{value}\n")).unwrap();
    assert_eq!(
        Reader::default().read(&path, "claude", "provider-session"),
        None
    );
    value["sessionId"] = serde_json::json!("provider-session");
    value["isSidechain"] = serde_json::json!(true);
    fs::write(&path, format!("{value}\n")).unwrap();
    assert_eq!(
        Reader::default().read(&path, "claude", "provider-session"),
        None
    );
}

#[test]
fn status_line_cross_checks_use_context_instead_of_billable_or_lifetime_totals() {
    // Small conformance examples: Claude input 10 + cache read 100 + write 20;
    // Codex last turn input 110 (100 cached) + output 20 (10 reasoning).
    // Both status lines therefore show 130 used; output/cached/reasoning fields
    // must not be double-counted, nor replaced by lifetime usage.
    for (provider, lines, capacity) in [("claude", 1, 200000), ("codex", 3, 272000)] {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("provider.jsonl");
        fs::write(
            &path,
            fixture(provider)
                .split_inclusive('\n')
                .take(lines)
                .collect::<String>(),
        )
        .unwrap();
        let usage = require_usage(Reader::default().read(&path, provider, "provider-session"));
        assert_eq!(
            (usage.used_tokens, usage.window_tokens),
            (Some(130), Some(capacity))
        );
    }
}

#[test]
fn claude_capacity_table_does_not_guess_ambiguous_variants() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider.jsonl");
    let mut row: serde_json::Value =
        serde_json::from_str(fixture("claude").lines().next().unwrap()).unwrap();
    for (model, window) in [
        ("claude-opus-4-6", None),
        ("claude-sonnet-4-5-20250929", None),
        ("claude-opus-4-6[1m]", Some(1000000)),
        ("claude-sonnet-4-5-20250929[1m]", Some(1000000)),
        ("claude-fable-5", Some(1000000)),
        ("claude-fable-5-1", Some(1000000)),
        ("claude-fable-5-20260101", Some(1000000)),
        ("claude-fable-5-1-20260101", Some(1000000)),
        ("claude-fable-5[1m]", Some(1000000)),
        ("claude-fable-5-1[1m]", Some(1000000)),
        ("claude-fable-5-20260101[1m]", Some(1000000)),
        ("claude-fable-5-1-20260101[1m]", Some(1000000)),
        ("custom-model[1m]", None),
    ] {
        row["message"]["model"] = serde_json::json!(model);
        fs::write(&path, format!("{row}\n")).unwrap();
        assert_eq!(
            require_usage(Reader::default().read(&path, "claude", "provider-session"))
                .window_tokens,
            window
        );
    }
}

#[test]
fn nullable_and_overflow_facts_remain_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("provider.jsonl");
    let mut row: serde_json::Value =
        serde_json::from_str(fixture("claude").lines().next().unwrap()).unwrap();
    row["message"]["model"] = serde_json::Value::Null;
    row["timestamp"] = serde_json::json!("invalid");
    row["message"]["usage"]["input_tokens"] = serde_json::json!(u64::MAX);
    fs::write(&path, format!("{row}\n")).unwrap();
    let usage = require_usage(Reader::default().read(&path, "claude", "provider-session"));
    assert_eq!(usage.used_tokens, None);
    assert_eq!(usage.window_tokens, None);
    assert_eq!(usage.model, None);
    assert_eq!(usage.observed_at, None);
}

#[test]
fn managed_codex_profile_uses_its_configured_rollout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("provider-config");
    let sessions = root.join("sessions/2026/01/01");
    fs::create_dir_all(&sessions).unwrap();
    fs::write(
        sessions.join("rollout.jsonl"),
        fixture("codex")
            .split_inclusive('\n')
            .take(3)
            .collect::<String>(),
    )
    .unwrap();
    let context = crate::CliContext {
        state_dir: tmp.path().join("state"),
        host: None,
    };
    let record: crate::SessionRecord = serde_json::from_value(serde_json::json!({
        "schema_version":"agent-session.session.v1", "id":"session", "agent":"codex", "mode":"oneshot", "title":null,
        "cwd":"/workspace/project", "tmux_session":"session", "prompt_file":null, "log_file":null,
        "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-01T00:00:00Z",
        "provider_resume":{"provider":"codex", "session_id":"provider-session", "captured_at":"2026-01-01T00:00:00Z", "capture_method":"fixture", "resume_args":[]},
        "runtime":{"kind":"tmux", "tmux_session":"session", "generation":1, "started_at":"2026-01-01T00:00:00Z", "launch_id":"fixture-runtime", "agent_profile_provider_config_dir":root}
    })).unwrap();
    let view = serde_json::to_value(crate::session_view(
        &context,
        &record,
        Some("stopped".into()),
        None,
    ))
    .unwrap();
    assert!(view["context_usage"].is_object(), "context usage missing");
    assert_eq!(view["context_usage"]["used_tokens"], 130);
    assert_eq!(view["context_usage"]["window_tokens"], 272000);
    assert_eq!(view["context_usage"]["model"], "example-model");
}

#[test]
fn managed_binding_applicability_survives_current_broker_unavailability() {
    use nils_test_support::{EnvGuard, GlobalStateLock};
    let lock = GlobalStateLock::new();
    let _claude = EnvGuard::remove(&lock, "AGENT_SESSION_CLAUDE_ACCOUNT_BROKER");
    let _codex = EnvGuard::remove(&lock, "AGENT_SESSION_CODEX_ACCOUNT_BROKER");
    for agent in ["claude", "codex"] {
        let mut record = local_backend_record(agent);
        record.extra.insert(
            format!("{agent}_account_binding"),
            serde_json::json!({
                "schema_version":format!("agent-session.{agent}-account-binding.v1"),
                "selected_account":"fixture-account", "revision":1, "state":"bound",
                "applied_runtime_id":"local-fixture-runtime", "updated_at":"2030-01-01T00:00:00Z"
            }),
        );
        let view = if agent == "claude" {
            serde_json::to_value(crate::claude_account::view_for_record(&record).unwrap()).unwrap()
        } else {
            serde_json::to_value(crate::codex_account::view_for_record(&record)).unwrap()
        };
        assert_eq!(view["supported"], false);
        assert_eq!(view["state"], "unsupported");
        assert_eq!(
            view["applicable"], true,
            "{agent}: backend applicability is independent from current broker support"
        );
        assert_eq!(view["selected_account"], "fixture-account");
    }
}

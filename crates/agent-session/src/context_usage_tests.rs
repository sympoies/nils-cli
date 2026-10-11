use super::*;
use pretty_assertions::assert_eq;
use std::fs;

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

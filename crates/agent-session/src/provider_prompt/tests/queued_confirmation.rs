use super::*;
use pretty_assertions::assert_eq;

fn queued(prompt: Value) -> Value {
    json!({"type":"attachment", "sessionId":"claude-id", "cwd":"/repo",
        "timestamp":"2099-01-01T00:00:02Z",
        "attachment":{"type":"queued_command", "commandMode":"prompt",
            "origin":{"kind":"human"}, "prompt":prompt}})
}

#[test]
fn queued_confirmation_uses_history_text_and_preserves_string_prompts() {
    for content in [
        json!("Review this image\nwith these instructions"),
        json!([{"type":"text", "text":"Review this image"},
            {"type":"image", "source":{"type":"base64", "media_type":"image/png", "data":"fixture"}},
            {"type":"text", "text":"with these instructions"}]),
    ] {
        let row = queued(content);
        let expected = "Review this image\nwith these instructions";
        let prompt = parse_claude_enqueued_prompt_value(&row, "claude-id").unwrap();
        assert_eq!(prompt.prompt, expected);
        assert_eq!(prompt.submitted_at.as_deref(), Some("2099-01-01T00:00:02Z"));
        assert_eq!(
            parse_history_user_prompt("claude", "claude-id", &row.to_string())
                .unwrap()
                .text,
            expected
        );
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("claude.jsonl");
        fs::write(&path, format!("{row}\n")).unwrap();
        let mut tail = ProviderPromptTail::open_path(
            ProviderKind::Claude,
            "claude-id",
            path.clone(),
            Duration::ZERO,
        )
        .unwrap();
        assert!(
            tail.poll_submitted_prompts().unwrap().is_empty(),
            "old rows are not new delivery evidence"
        );
        append(&path, format!("{row}\n").as_bytes());
        assert_eq!(
            tail.poll_submitted_prompts().unwrap(),
            vec![expected.to_string()]
        );
        let source = ProviderPromptSource::test_path(ProviderKind::Claude, "claude-id", path);
        assert_eq!(
            prompt_observed_after(&source, expected, &"2099-01-01T00:00:01Z".parse().unwrap()),
            Some(true)
        );
        assert_eq!(
            prompt_observed_after(&source, expected, &"2099-01-01T00:00:03Z".parse().unwrap()),
            Some(false)
        );
    }
}

#[test]
fn queued_confirmation_rejects_foreign_and_nonhuman_attachments() {
    let base = queued(json!([{"type":"text", "text":"Review this image"}]));
    for (pointer, replacement) in [
        ("/sessionId", json!("foreign-id")),
        ("/attachment/commandMode", json!("task-notification")),
        ("/attachment/origin/kind", json!("peer")),
        ("/attachment/type", json!("other")),
    ] {
        let mut row = base.clone();
        *row.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            parse_claude_enqueued_prompt_value(&row, "claude-id").is_none(),
            "{pointer}"
        );
    }
    let mut sidechain = base.clone();
    sidechain["isSidechain"] = json!(true);
    assert!(parse_claude_enqueued_prompt_value(&sidechain, "claude-id").is_none());
    let mut meta = base;
    meta["attachment"]["isMeta"] = json!(true);
    assert!(parse_claude_enqueued_prompt_value(&meta, "claude-id").is_none());
    let image_only =
        queued(json!([{"type":"image", "source":{"type":"base64", "data":"fixture"}}]));
    assert!(parse_claude_enqueued_prompt_value(&image_only, "claude-id").is_none());
}

#[test]
fn queued_confirmation_skips_inline_image_bytes_across_polls() {
    let row = queued(json!([
        {"type":"text", "text":"Review this image"},
        {"type":"image", "source":{"type":"base64", "media_type":"image/png", "data":"A".repeat(MAX_PROVIDER_LINE_BYTES * 2)}}
    ]));
    let tmp = tempfile::TempDir::new().unwrap();
    let path = tmp.path().join("claude.jsonl");
    fs::write(&path, "").unwrap();
    let mut tail = ProviderPromptTail::open_path(
        ProviderKind::Claude,
        "claude-id",
        path.clone(),
        Duration::ZERO,
    )
    .unwrap();
    append(&path, format!("{row}\n").as_bytes());
    let mut observed = Vec::new();
    for _ in 0..16 {
        observed.extend(tail.poll_submitted_prompts().unwrap());
    }
    assert_eq!(observed, vec!["Review this image".to_string()]);
}

use super::*;
use pretty_assertions::assert_eq;

#[test]
fn image_tool_result_keeps_the_prompt_answer_excerpt() {
    let temp = tempfile::TempDir::new().unwrap();
    let root = temp.path().join("projects");
    std::fs::create_dir_all(root.join("repo")).unwrap();
    let row = |role: &str, content: Value| json!({"sessionId":"image-tool-result","cwd":temp.path(),"type":role,"promptSource":"typed","message":{"role":role,"content":content}});
    let rows = [
        row(
            "user",
            json!([{"type":"text","text":"Inspect the screenshot"}]),
        ),
        row(
            "assistant",
            json!([{"type":"tool_use","id":"tool-one","name":"inspect","input":{}}]),
        ),
        row(
            "user",
            json!([{"type":"tool_result","tool_use_id":"tool-one","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}}]}]),
        ),
        row(
            "assistant",
            json!([{"type":"text","text":"The screenshot shows the expected result"}]),
        ),
    ];
    std::fs::write(
        root.join("repo/image-tool-result.jsonl"),
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    let catalog = HistoryCatalog::new(
        vec![crate::provider_history::HistorySource {
            provider: "claude".into(),
            agent_profile: None,
            root,
        }],
        crate::provider_history::archive_root(temp.path()),
        crate::provider_history::star_root(temp.path()),
    );
    let id = crate::provider_history::stable_history_id("claude", None, "image-tool-result");
    let page = catalog
        .messages(&id, None, 50, HistoryMessageDirection::Latest)
        .unwrap();
    assert_eq!(page.messages.len(), 3);
    assert!(!page.messages[1].human_prompt);
    assert_eq!(page.messages[1].images.len(), 1);
    let turns = pair_turns(page.messages);
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].user_prompt, "Inspect the screenshot");
    assert_eq!(
        turns[0].assistant_excerpt.as_deref(),
        Some("The screenshot shows the expected result")
    );
}

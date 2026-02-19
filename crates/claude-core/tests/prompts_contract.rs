use claude_core::prompts::render_execute_prompt;

#[test]
fn prompt_uses_input_when_provided() {
    assert_eq!(
        render_execute_prompt("fallback", Some("prompt: hello")),
        "hello"
    );
}

#[test]
fn advice_prefix_expands_into_template() {
    let rendered = render_execute_prompt("advice: how to improve tests?", None);
    assert!(rendered.contains("senior software engineer"));
    assert!(rendered.contains("how to improve tests?"));
}

#[test]
fn knowledge_prefix_expands_into_template() {
    let rendered = render_execute_prompt("knowledge: eventual consistency", None);
    assert!(rendered.contains("Explain the following concept clearly"));
    assert!(rendered.contains("eventual consistency"));
}

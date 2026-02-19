#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptIntent {
    Prompt,
    Advice,
    Knowledge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptRenderError {
    MissingTask,
}

pub fn render_execute_prompt(task: &str, input: Option<&str>) -> Result<String, PromptRenderError> {
    let source = select_prompt_source(task, input);
    if source.is_empty() {
        return Err(PromptRenderError::MissingTask);
    }

    let (intent, body) = detect_intent(source)?;
    let prompt = match intent {
        PromptIntent::Prompt => body.to_string(),
        PromptIntent::Advice => format!(
            "You are a senior software engineer. Provide concise, actionable engineering advice.\n\nQuestion:\n{body}\n\nResponse format:\n1. Recommendation\n2. Why\n3. Risks\n4. Validation"
        ),
        PromptIntent::Knowledge => format!(
            "Explain the following concept clearly for an engineer.\n\nConcept:\n{body}\n\nResponse format:\n1. Definition\n2. How it works\n3. Practical example\n4. Common pitfalls"
        ),
    };
    Ok(prompt)
}

fn select_prompt_source<'a>(task: &'a str, input: Option<&'a str>) -> &'a str {
    input
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })
        .unwrap_or_else(|| task.trim())
}

fn detect_intent(source: &str) -> Result<(PromptIntent, &str), PromptRenderError> {
    if let Some(body) = strip_intent_prefix(source, "advice:") {
        return non_empty_intent(PromptIntent::Advice, body);
    }
    if let Some(body) = strip_intent_prefix(source, "knowledge:") {
        return non_empty_intent(PromptIntent::Knowledge, body);
    }
    if let Some(body) = strip_intent_prefix(source, "prompt:") {
        return non_empty_intent(PromptIntent::Prompt, body);
    }

    if let Some(body) = strip_intent_keyword(source, "advice") {
        return non_empty_intent(PromptIntent::Advice, body);
    }
    if let Some(body) = strip_intent_keyword(source, "knowledge") {
        return non_empty_intent(PromptIntent::Knowledge, body);
    }
    if is_bare_intent_keyword(source, "prompt")
        || is_bare_intent_keyword(source, "advice")
        || is_bare_intent_keyword(source, "knowledge")
    {
        return Err(PromptRenderError::MissingTask);
    }

    Ok((PromptIntent::Prompt, source))
}

fn strip_intent_prefix<'a>(source: &'a str, prefix: &str) -> Option<&'a str> {
    let head = source.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    source.get(prefix.len()..).map(str::trim)
}

fn strip_intent_keyword<'a>(source: &'a str, keyword: &str) -> Option<&'a str> {
    let head = source.get(..keyword.len())?;
    if !head.eq_ignore_ascii_case(keyword) {
        return None;
    }
    let tail = source.get(keyword.len()..)?;
    let mut chars = tail.chars();
    if !matches!(chars.next(), Some(ch) if ch.is_whitespace()) {
        return None;
    }
    Some(tail.trim())
}

fn non_empty_intent(
    intent: PromptIntent,
    body: &str,
) -> Result<(PromptIntent, &str), PromptRenderError> {
    if body.is_empty() {
        return Err(PromptRenderError::MissingTask);
    }
    Ok((intent, body))
}

fn is_bare_intent_keyword(source: &str, keyword: &str) -> bool {
    source.eq_ignore_ascii_case(keyword)
}

#[cfg(test)]
mod tests {
    use super::{PromptRenderError, render_execute_prompt};
    use pretty_assertions::assert_eq;

    #[test]
    fn prompt_uses_input_when_provided() {
        assert_eq!(
            render_execute_prompt("fallback", Some("prompt: hello")),
            Ok("hello".to_string())
        );
    }

    #[test]
    fn advice_prefix_expands_into_template() {
        let rendered =
            render_execute_prompt("advice: how to improve tests?", None).expect("rendered prompt");
        assert!(rendered.contains("senior software engineer"));
        assert!(rendered.contains("how to improve tests?"));
    }

    #[test]
    fn knowledge_prefix_expands_into_template() {
        let rendered = render_execute_prompt("knowledge: eventual consistency", None)
            .expect("rendered prompt");
        assert!(rendered.contains("Explain the following concept clearly"));
        assert!(rendered.contains("eventual consistency"));
    }

    #[test]
    fn empty_source_returns_missing_task_error() {
        assert_eq!(
            render_execute_prompt("   ", Some("   ")),
            Err(PromptRenderError::MissingTask)
        );
    }

    #[test]
    fn empty_prefixed_intent_body_returns_missing_task_error() {
        assert_eq!(
            render_execute_prompt("prompt:", None),
            Err(PromptRenderError::MissingTask)
        );
        assert_eq!(
            render_execute_prompt("advice:   ", None),
            Err(PromptRenderError::MissingTask)
        );
        assert_eq!(
            render_execute_prompt("knowledge:  ", None),
            Err(PromptRenderError::MissingTask)
        );
        assert_eq!(
            render_execute_prompt("advice   ", None),
            Err(PromptRenderError::MissingTask)
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptIntent {
    Prompt,
    Advice,
    Knowledge,
}

pub fn render_execute_prompt(task: &str, input: Option<&str>) -> String {
    let source = input
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(task)
        .trim();
    if source.is_empty() {
        return String::new();
    }

    let (intent, body) = detect_intent(source);
    match intent {
        PromptIntent::Prompt => body.to_string(),
        PromptIntent::Advice => format!(
            "You are a senior software engineer. Provide concise, actionable engineering advice.\n\nQuestion:\n{body}\n\nResponse format:\n1. Recommendation\n2. Why\n3. Risks\n4. Validation"
        ),
        PromptIntent::Knowledge => format!(
            "Explain the following concept clearly for an engineer.\n\nConcept:\n{body}\n\nResponse format:\n1. Definition\n2. How it works\n3. Practical example\n4. Common pitfalls"
        ),
    }
}

fn detect_intent(source: &str) -> (PromptIntent, &str) {
    let lower = source.to_ascii_lowercase();
    if let Some(rest) = lower
        .strip_prefix("advice:")
        .and_then(|_| source.split_once(':').map(|(_, tail)| tail.trim()))
        .filter(|value| !value.is_empty())
    {
        return (PromptIntent::Advice, rest);
    }
    if let Some(rest) = lower
        .strip_prefix("knowledge:")
        .and_then(|_| source.split_once(':').map(|(_, tail)| tail.trim()))
        .filter(|value| !value.is_empty())
    {
        return (PromptIntent::Knowledge, rest);
    }
    if let Some(rest) = lower
        .strip_prefix("prompt:")
        .and_then(|_| source.split_once(':').map(|(_, tail)| tail.trim()))
        .filter(|value| !value.is_empty())
    {
        return (PromptIntent::Prompt, rest);
    }

    if lower.starts_with("advice ") {
        return (PromptIntent::Advice, source[7..].trim());
    }
    if lower.starts_with("knowledge ") {
        return (PromptIntent::Knowledge, source[10..].trim());
    }

    (PromptIntent::Prompt, source)
}

#[cfg(test)]
mod tests {
    use super::render_execute_prompt;
    use pretty_assertions::assert_eq;

    #[test]
    fn prompt_uses_input_when_provided() {
        assert_eq!(
            render_execute_prompt("fallback", Some("prompt: hello")),
            "hello".to_string()
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
}

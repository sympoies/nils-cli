use super::isolated::GeneratedCommitMessage;

pub(super) fn validate_generated_message(message: &GeneratedCommitMessage) -> Result<(), String> {
    const TYPES: [&str; 11] = [
        "build", "chore", "ci", "docs", "feat", "fix", "perf", "refactor", "revert", "style",
        "test",
    ];
    if !TYPES.contains(&message.commit_type.as_str()) {
        return Err("codex-cli agent commit: invalid commit type".to_string());
    }
    if let Some(scope) = &message.scope
        && (scope.trim().is_empty()
            || scope.len() > 64
            || scope.chars().any(|ch| {
                !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
            }))
    {
        return Err("codex-cli agent commit: invalid commit scope".to_string());
    }
    let subject = message.subject.trim();
    if subject.is_empty() || subject != message.subject || subject.chars().any(char::is_control) {
        return Err("codex-cli agent commit: invalid commit subject".to_string());
    }
    let header_len = message.commit_type.len()
        + message
            .scope
            .as_ref()
            .map_or(0, |scope| scope.chars().count() + 2)
        + 2
        + subject.chars().count();
    let max_width = match std::env::var("SEMANTIC_COMMIT_HEADER_WIDTH") {
        Ok(value) => value
            .parse::<usize>()
            .ok()
            .filter(|width| *width > 0)
            .ok_or_else(|| {
                "codex-cli agent commit: SEMANTIC_COMMIT_HEADER_WIDTH must be a positive integer"
                    .to_string()
            })?,
        Err(std::env::VarError::NotPresent) => 100,
        Err(_) => {
            return Err(
                "codex-cli agent commit: SEMANTIC_COMMIT_HEADER_WIDTH must be valid UTF-8"
                    .to_string(),
            );
        }
    };
    if header_len > max_width {
        return Err(format!(
            "codex-cli agent commit: commit header exceeds {max_width} characters"
        ));
    }
    if message.body_bullets.len() > 20
        || message.body_bullets.iter().any(|bullet| {
            bullet.trim().is_empty()
                || bullet != bullet.trim()
                || bullet.len() > 500
                || bullet.chars().any(char::is_control)
        })
    {
        return Err("codex-cli agent commit: invalid body bullet".to_string());
    }
    Ok(())
}

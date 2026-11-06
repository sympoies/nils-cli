use super::{
    CODEX_NOTIFY_FORWARD_FLAG, CodexNotifyMode, CodexOwnedNotify, SESSION_BINARY,
    codex_notify_argv_is_bounded,
};

pub(super) fn migration_activity_command(command: &str, product: super::Product) -> bool {
    command
        == format!(
            "{} activity hook --agent {}",
            SESSION_BINARY,
            product.as_str()
        )
        || (product == super::Product::Codex
            && command
                == format!(
                    "sh -c 'if [ \"${{AGENT_SESSION_ATTENTION_AUTHORITY:-hook}}\" = protocol ]; then exit 0; fi; exec {SESSION_BINARY} activity hook --agent codex'"
                ))
}

pub(super) fn codex_forward_argv_is_safe(argv: &[String]) -> bool {
    codex_notify_argv_is_bounded(argv)
        && !argv.iter().any(|value| value == CODEX_NOTIFY_FORWARD_FLAG)
        && !argv.starts_with(&codex_notify_argv())
        && !(argv.len() >= 5
            && argv_matches(
                &argv[..5],
                &[SESSION_BINARY, "activity", "notify", "--agent", "codex"],
            ))
}

pub(super) fn is_migration_notify(
    document: &toml_edit::DocumentMut,
    mode: &CodexNotifyMode,
) -> bool {
    document
        .get("notify")
        .and_then(toml_edit::Item::as_array)
        .and_then(|a| a.get(0))
        .and_then(toml_edit::Value::as_str)
        == Some(SESSION_BINARY)
        && matches!(mode, CodexNotifyMode::Owned | CodexNotifyMode::Composed(_))
}

pub(super) fn codex_notify_argv() -> Vec<String> {
    nils_common::provider_entrypoint::argv(false, &["activity", "notify", "--agent", "codex"])
}

pub(super) fn migrate_wrapped(
    base: &[String],
    forwarded: Option<&[String]>,
) -> Option<Vec<String>> {
    let mut owned = codex_notify_argv();
    if let Some(forwarded) = forwarded {
        owned.push(CODEX_NOTIFY_FORWARD_FLAG.to_string());
        owned.push(serde_json::to_string(forwarded).ok()?);
    }
    let mut normalized = base.to_vec();
    normalized.push(super::CODEX_COMPUTER_USE_PREVIOUS_NOTIFY_FLAG.to_string());
    normalized.push(serde_json::to_string(&owned).ok()?);
    Some(normalized)
}

pub(super) fn count_codex_dispatches(raw: &str) -> usize {
    let Ok(document) = raw.parse::<toml_edit::DocumentMut>() else {
        return 0;
    };
    let Some(hooks) = document.get("hooks").and_then(toml_edit::Item::as_table) else {
        return 0;
    };
    let current =
        nils_common::provider_entrypoint::command(true, &["dispatch", "--product", "codex"]);
    hooks
        .iter()
        .filter_map(|(_, groups)| groups.as_array_of_tables())
        .flat_map(|groups| groups.iter())
        .filter_map(|group| {
            group
                .get("hooks")
                .and_then(toml_edit::Item::as_array_of_tables)
        })
        .flat_map(|handlers| handlers.iter())
        .filter(|handler| {
            handler.get("type").and_then(toml_edit::Item::as_str) == Some("command")
                && handler
                    .get("command")
                    .and_then(toml_edit::Item::as_str)
                    .is_some_and(|command| {
                        command == current || command == "agent-hook dispatch --product codex"
                    })
        })
        .count()
}

pub(super) fn argv_matches(argv: &[String], expected: &[&str]) -> bool {
    argv.len() == expected.len()
        && argv
            .iter()
            .zip(expected)
            .all(|(value, expected)| value == expected)
}

pub(super) fn codex_owned_notify(argv: &[String]) -> Option<CodexOwnedNotify> {
    let legacy = [SESSION_BINARY, "activity", "notify", "--agent", "codex"];
    let prefix = if argv.len() >= legacy.len() && argv_matches(&argv[..legacy.len()], &legacy) {
        legacy.map(str::to_string).to_vec()
    } else {
        codex_notify_argv()
    };
    if argv == prefix {
        return Some(CodexOwnedNotify::Owned);
    }
    if argv.len() == prefix.len() + 2
        && argv[..prefix.len()] == prefix
        && argv[prefix.len()] == CODEX_NOTIFY_FORWARD_FLAG
        && let Ok(forwarded) = serde_json::from_str::<Vec<String>>(&argv[prefix.len() + 1])
        && codex_forward_argv_is_safe(&forwarded)
    {
        return Some(CodexOwnedNotify::Composed(forwarded));
    }
    None
}

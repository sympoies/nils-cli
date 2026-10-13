//! Codex permission ingress shares one immutable runtime authority across
//! legacy raw hooks and normalized dispatcher events. Exact app-server events
//! retain their separate private protocol admission.
use super::*;

pub(super) fn ingest_normalized(
    context: &CliContext,
    id: &str,
    event: TurnEvent,
    lock_mode: ActivityLockMode,
) -> Result<ActivityResult, CliError> {
    if event.provider == "codex"
        && event.schema_version == TURN_EVENT_VERSION
        && event.source_kind == SourceKind::ProviderHook
        && event.kind == TurnEventKind::AttentionRequested
        && event.attention_kind.as_deref() == Some("approval")
    {
        validate_event(&event, EventAdmission::Generic)?;
        let record = load_session_record(context, id)?;
        if record.agent == "codex"
            && record
                .runtime
                .as_ref()
                .is_some_and(|runtime| runtime.launch_id == event.runtime_id)
        {
            let authority = std::env::var(crate::codex_app_server::ATTENTION_AUTHORITY_ENV).ok();
            match codex_hook_attention_disposition(&record, authority.as_deref()) {
                CodexHookAttentionDisposition::Accept => {}
                CodexHookAttentionDisposition::Suppress => {
                    let mut result = activity_status_for_record(context, &record)?;
                    result.duplicate = true;
                    return Ok(result);
                }
                CodexHookAttentionDisposition::Breach => {
                    mark_runtime_unhealthy(
                        context,
                        id,
                        &event.runtime_id,
                        "codex_attention_authority_mismatch",
                    )?;
                    return Err(CliError::data(
                        "codex-attention-authority-breach",
                        "Codex permission event authority did not match the immutable runtime selection",
                        Some(json!({ "id": record.id })),
                    ));
                }
            }
        }
    }
    ingest_event_with_lock(context, id, event, lock_mode, EventAdmission::Generic)
}

pub(crate) fn codex_protocol_attention_source_guard_configured() -> bool {
    let Ok(json_path) = provider_config_path(AgentKind::Codex) else {
        return false;
    };
    let Ok(config_path) = codex_notification_config_path() else {
        return false;
    };
    let Ok(status) = codex_hook_status(&json_path, &config_path) else {
        return false;
    };
    if status.conflict || status.ownership_conflict {
        return false;
    }
    match status.representation {
        CodexHookRepresentation::Json => codex_json_permission_source_guard(&json_path),
        CodexHookRepresentation::InlineToml => codex_toml_permission_source_guard(&config_path),
    }
}

pub(super) fn codex_json_permission_source_guard(path: &Path) -> bool {
    let Ok(bytes) = fs::read(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let expected = owned_command(AgentKind::Codex, Some("PermissionRequest"));
    let Some(groups) = value
        .get("hooks")
        .and_then(|hooks| hooks.get("PermissionRequest"))
        .and_then(Value::as_array)
    else {
        return false;
    };
    let mut guarded = 0_usize;
    let mut dispatched = 0_usize;
    for group in groups {
        let matcher_is_absent = group.get("matcher").is_none();
        let Some(handlers) = group.get("hooks").and_then(Value::as_array) else {
            return false;
        };
        for handler in handlers {
            if handler.get("type").and_then(Value::as_str) != Some("command") {
                continue;
            }
            let Some(command) = handler.get("command").and_then(Value::as_str) else {
                return false;
            };
            if !codex_permission_reporter_command(command) {
                continue;
            }
            if matcher_is_absent
                && command == expected
                && json_codex_handler_is_owned("PermissionRequest", handler)
            {
                guarded += 1;
            } else if matcher_is_absent
                && command == "agent-hook dispatch --product codex"
                && handler.as_object().is_some_and(|object| object.len() == 3)
                && handler.get("timeout").and_then(Value::as_u64) == Some(60)
            {
                dispatched += 1;
            } else {
                // A second direct reporter can bypass the runtime authority
                // guard even when the owned handler is also installed.
                return false;
            }
        }
    }
    (guarded == 1 && dispatched == 0)
        || (guarded == 0 && dispatched == 1 && agent_hook_codex_configured() == Ok(true))
}

pub(super) fn codex_toml_permission_source_guard(path: &Path) -> bool {
    let Ok(raw) = fs::read_to_string(path) else {
        return false;
    };
    let Ok(document) = parse_codex_notification_config(path, &raw) else {
        return false;
    };
    let expected = owned_command(AgentKind::Codex, Some("PermissionRequest"));
    let Some(groups) = document
        .get("hooks")
        .and_then(TomlItem::as_table)
        .and_then(|hooks| hooks.get("PermissionRequest"))
        .and_then(TomlItem::as_array_of_tables)
    else {
        return false;
    };
    let mut guarded = 0_usize;
    let mut dispatched = 0_usize;
    for group in groups.iter() {
        let Some(handlers) = group.get("hooks").and_then(TomlItem::as_array_of_tables) else {
            return false;
        };
        for handler in handlers.iter() {
            if handler.get("type").and_then(TomlItem::as_str) != Some("command") {
                continue;
            }
            let Some(command) = handler.get("command").and_then(TomlItem::as_str) else {
                return false;
            };
            if !codex_permission_reporter_command(command) {
                continue;
            }
            if toml_hook_matcher_is_exact(
                group,
                ProviderSpec {
                    event: "PermissionRequest",
                    matcher: None,
                },
            ) && command == expected
                && toml_handler_command_is_owned("PermissionRequest", handler)
            {
                guarded += 1;
            } else if group.get("matcher").is_none()
                && command == "agent-hook dispatch --product codex"
                && handler.len() == 3
                && handler.get("timeout").and_then(TomlItem::as_integer) == Some(60)
            {
                dispatched += 1;
            } else {
                return false;
            }
        }
    }
    (guarded == 1 && dispatched == 0)
        || (guarded == 0 && dispatched == 1 && agent_hook_codex_configured() == Ok(true))
}

fn codex_permission_reporter_command(command: &str) -> bool {
    let words = command
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        })
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    words
        .windows(3)
        .any(|words| words == ["agent-session", "activity", "hook"])
        || words
            .windows(2)
            .any(|words| words == ["agent-hook", "dispatch"])
}

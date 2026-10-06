//! Bounded provider settings only: never infer from an account or title model.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{CliContext, CliError, SessionRecord};

const KEY: &str = "model_settings";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct ModelSettings {
    pub(crate) model: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
}

pub(crate) use agent_hook::session_metadata::{effort_label, model_label};

impl ModelSettings {
    pub(crate) fn from_launch(agent: &str, args: &[String]) -> Self {
        if !matches!(agent, "codex" | "claude" | "dsh") {
            return Self::default();
        }
        let mut settings = Self::default();
        let mut config_model = None;
        let mut explicit_model = false;
        let mut iter = args.iter();
        while let Some(arg) = iter.next() {
            if arg == "--" {
                break;
            }
            let (flag, inline) = arg
                .split_once('=')
                .map_or((arg.as_str(), None), |(key, value)| (key, Some(value)));
            match flag {
                "--model" | "-m" => {
                    explicit_model = true;
                    settings.model = inline
                        .or_else(|| iter.next().map(String::as_str))
                        .and_then(model_label);
                }
                "--effort" if agent != "codex" => {
                    settings.reasoning_effort = inline
                        .or_else(|| iter.next().map(String::as_str))
                        .and_then(effort_label);
                }
                "-c" | "--config" if agent == "codex" => {
                    if let Some(value) = inline.or_else(|| iter.next().map(String::as_str)) {
                        parse_config(value, &mut config_model, &mut settings.reasoning_effort);
                    }
                }
                _ if agent == "codex" && arg.starts_with("-c") && arg.len() > 2 => {
                    parse_config(&arg[2..], &mut config_model, &mut settings.reasoning_effort);
                }
                _ if agent == "codex" && arg.starts_with("-m") && arg.len() > 2 => {
                    explicit_model = true;
                    settings.model = model_label(&arg[2..]);
                }
                _ => {}
            }
        }
        if !explicit_model {
            settings.model = config_model;
        }
        settings
    }

    fn sanitized(self) -> Self {
        Self {
            model: self.model.as_deref().and_then(model_label),
            reasoning_effort: self.reasoning_effort.as_deref().and_then(effort_label),
        }
    }

    pub(crate) fn for_record(record: &SessionRecord) -> Self {
        let stored = record.extra.get(KEY);
        let launch_id = record
            .runtime
            .as_ref()
            .map(|runtime| runtime.launch_id.as_str());
        if let Some(value) = stored
            && value.get("runtime_id").and_then(Value::as_str) == launch_id
            && value
                .get("provider_session_id")
                .and_then(Value::as_str)
                .is_none_or(|id| {
                    record
                        .provider_resume
                        .as_ref()
                        .is_none_or(|_| provider_session_matches(record, id))
                })
            && let Ok(settings) = serde_json::from_value::<Self>(value.clone())
        {
            return settings.sanitized();
        }
        Self::from_launch(&record.agent, &record.agent_args)
    }

    pub(crate) fn store_launch(record: &mut SessionRecord) {
        let settings = Self::from_launch(&record.agent, &record.agent_args);
        record.extra.insert(
            KEY.to_string(),
            json!({
                "model": settings.model,
                "reasoning_effort": settings.reasoning_effort,
                "runtime_id": record.runtime.as_ref().map(|runtime| &runtime.launch_id),
            }),
        );
    }
}

fn parse_config(value: &str, model: &mut Option<String>, effort: &mut Option<String>) {
    let Some((key, value)) = value.split_once('=') else {
        return;
    };
    let value = value.trim();
    // Codex accepts both TOML strings and unquoted scalar overrides.
    let parsed = format!("value = {value}")
        .parse::<toml_edit::DocumentMut>()
        .ok()
        .and_then(|table| {
            table
                .get("value")
                .and_then(toml_edit::Item::as_str)
                .map(str::to_string)
        });
    let value = parsed.as_deref().unwrap_or(value);
    match key.trim() {
        "model" => *model = model_label(value),
        "model_reasoning_effort" => *effort = effort_label(value),
        _ => {}
    }
}

/// Called only with provider-owned metadata, after the owning ingress has
/// authenticated the runtime. Check its exact session identity again under lock.
pub(crate) fn observe(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    agent: &str,
    provider_session_id: &str,
    raw: &Value,
) -> Result<(), CliError> {
    observe_inner(
        context,
        id,
        runtime_id,
        agent,
        provider_session_id,
        raw,
        false,
    )
}

/// The proxy has acknowledged this primary thread before forwarding its start
/// response; durable resume capture may follow later on the control connection.
pub(crate) fn observe_primary_codex_thread(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    provider_session_id: &str,
    raw: &Value,
) -> Result<(), CliError> {
    observe_inner(
        context,
        id,
        runtime_id,
        "codex",
        provider_session_id,
        raw,
        true,
    )
}

pub(crate) fn observe_claude_hook(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    provider_session_id: &str,
    raw: &Value,
    session_start: bool,
) -> Result<(), CliError> {
    observe_inner(
        context,
        id,
        runtime_id,
        "claude",
        provider_session_id,
        raw,
        session_start,
    )
}

fn provider_session_matches(record: &SessionRecord, session_id: &str) -> bool {
    let Some(resume) = record.provider_resume.as_ref() else {
        return false;
    };
    resume.provider == record.agent
        && (resume.session_id == session_id
            || (record
                .runtime
                .as_ref()
                .zip(crate::AgentKind::from_name(&record.agent))
                .and_then(|(runtime, agent)| {
                    crate::activity::projected_provider_identifier(
                        &runtime.launch_id,
                        agent,
                        "session",
                        &resume.session_id,
                    )
                    .ok()
                })
                .as_deref()
                == Some(session_id)))
}

fn observation_matches(
    record: &SessionRecord,
    runtime_id: &str,
    agent: &str,
    session_id: &str,
    allow_unbound: bool,
) -> bool {
    record.agent == agent
        && record
            .runtime
            .as_ref()
            .is_some_and(|runtime| runtime.launch_id == runtime_id)
        && if record.provider_resume.is_some() {
            provider_session_matches(record, session_id)
        } else {
            allow_unbound
                && record
                    .extra
                    .get(KEY)
                    .filter(|value| {
                        value.get("runtime_id").and_then(Value::as_str) == Some(runtime_id)
                    })
                    .and_then(|value| value.get("provider_session_id"))
                    .and_then(Value::as_str)
                    .is_none_or(|previous| previous == session_id)
        }
}

fn observe_inner(
    context: &CliContext,
    id: &str,
    runtime_id: &str,
    agent: &str,
    provider_session_id: &str,
    raw: &Value,
    allow_unbound: bool,
) -> Result<(), CliError> {
    let model_value = raw.get("model").or_else(|| raw.get("to_model"));
    let mut model_observed = model_value.is_some();
    let mut model = model_value.and_then(Value::as_str).and_then(model_label);
    let effort_value = raw
        .get("reasoning_effort")
        .or_else(|| raw.get("reasoningEffort"))
        .or_else(|| raw.get("effort"));
    let effort_observed = effort_value.is_some();
    let effort = effort_value
        .and_then(|effort| {
            effort
                .as_str()
                .or_else(|| effort.get("level").and_then(Value::as_str))
        })
        .and_then(effort_label);
    let observed = crate::load_session_record(context, id)?;
    if !observation_matches(
        &observed,
        runtime_id,
        agent,
        provider_session_id,
        allow_unbound,
    ) {
        return Ok(());
    }
    if !model_observed
        && agent == "claude"
        && let Some(evidence) = crate::provider_prompt::latest_claude_model(&observed)
    {
        model_observed = true;
        model = evidence;
    }
    if !model_observed && !effort_observed {
        return Ok(());
    }
    let current = ModelSettings::for_record(&observed);
    if (!model_observed || current.model == model)
        && (!effort_observed || current.reasoning_effort == effort)
    {
        return Ok(());
    }
    // Codex calls this on an ordered worker; retry short lifecycle contention.
    let _lock =
        crate::acquire_session_record_lock_timed(context, id, std::time::Duration::from_secs(2))?;
    let mut record = crate::load_session_record(context, id)?;
    crate::ensure_same_session_identity(&observed, &record)?;
    (|| {
        if !observation_matches(
            &record,
            runtime_id,
            agent,
            provider_session_id,
            allow_unbound,
        ) {
            return Ok(());
        }
        let mut settings = ModelSettings::for_record(&record);
        if model_observed {
            if settings.model != model {
                settings.reasoning_effort = None;
            }
            settings.model = model;
        }
        if effort_observed {
            settings.reasoning_effort = effort;
        }
        record.extra.insert(KEY.to_string(), json!({
            "model": settings.model, "reasoning_effort": settings.reasoning_effort, "runtime_id": runtime_id,
            "provider_session_id": provider_session_id,
        }));
        crate::write_session_document(context, &record)
    })()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn launch_parsing_obeys_provider_flags_and_unknowns() {
        for (provider, args, model, effort) in [
            (
                "codex",
                vec![
                    "-m",
                    "example-model",
                    "--config=model_reasoning_effort='high'",
                ],
                Some("example-model"),
                Some("high"),
            ),
            (
                "codex",
                vec![
                    "-mfirst",
                    "-cmodel='config-model'",
                    "--model=last",
                    "-cmodel_reasoning_effort=low",
                    "-c",
                    "model_reasoning_effort=medium",
                ],
                Some("last"),
                Some("medium"),
            ),
            (
                "codex",
                vec!["--config", "model='configured'", "--oss"],
                Some("configured"),
                None,
            ),
            (
                "claude",
                vec!["--model", "opus", "--effort=medium"],
                Some("opus"),
                Some("medium"),
            ),
            (
                "claude",
                vec!["--model=local-model", "--effort", "high"],
                Some("local-model"),
                Some("high"),
            ),
            (
                "dsh",
                vec!["--model=local-model"],
                Some("local-model"),
                None,
            ),
            ("claude", vec!["--", "--model", "prompt-text"], None, None),
            (
                "codex",
                vec![
                    "--model",
                    "/private/model",
                    "-c",
                    "model_reasoning_effort=invalid",
                ],
                None,
                None,
            ),
            ("codex", vec!["--model", "sk-test-canary"], None, None),
            ("claude", vec!["--model"], None, None),
            ("unsupported", vec!["--model=example-model"], None, None),
        ] {
            let args = args.into_iter().map(str::to_string).collect::<Vec<_>>();
            assert_eq!(
                ModelSettings::from_launch(provider, &args),
                ModelSettings {
                    model: model.map(str::to_string),
                    reasoning_effort: effort.map(str::to_string),
                }
            );
        }
    }
    #[test]
    fn session_model_launch_rejects_credential_shaped_labels() {
        for value in [
            "gho_example",
            "ghu_example",
            "ghs_example",
            "ghr_example",
            "xoxp-example",
            "xoxa-example",
        ] {
            assert_eq!(
                ModelSettings::from_launch("codex", &["--model".into(), value.into()]),
                ModelSettings::default()
            );
        }
    }
    #[test]
    fn provider_observations_are_persisted_and_fenced_to_the_runtime_and_session() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let mut record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1", "id": "settings-test",
            "agent": "claude", "mode": "interactive", "cwd": "/work/example",
            "tmux_session": "settings-test", "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "agent_args": ["--model", "opus", "--effort", "medium"],
            "runtime": {"kind": "tmux", "tmux_session": "settings-test", "generation": 1, "started_at": "2030-01-01T00:00:00Z", "launch_id": "runtime-one"},
            "provider_resume": {"provider": "claude", "session_id": "provider-one", "captured_at": "2030-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []}
        })).unwrap();
        ModelSettings::store_launch(&mut record);
        crate::write_session_record(&context, &record).unwrap();
        let persisted = crate::load_session_record(&context, &record.id).unwrap();
        assert_eq!(persisted.extra[KEY]["model"], "opus");
        assert_eq!(persisted.extra[KEY]["reasoning_effort"], "medium");
        for (runtime, session) in [
            ("stale-runtime", "provider-one"),
            ("runtime-one", "auxiliary-session"),
        ] {
            observe(
                &context,
                &record.id,
                runtime,
                "claude",
                session,
                &json!({"model": "sonnet", "effort": "high"}),
            )
            .unwrap();
            assert_eq!(
                ModelSettings::for_record(
                    &crate::load_session_record(&context, &record.id).unwrap()
                )
                .model
                .as_deref(),
                Some("opus")
            );
        }
        observe(
            &context,
            &record.id,
            "runtime-one",
            "claude",
            "provider-one",
            &json!({"model": "sonnet"}),
        )
        .unwrap();
        let current = crate::load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            ModelSettings::for_record(&current),
            ModelSettings {
                model: Some("sonnet".into()),
                reasoning_effort: None
            }
        );
        observe(
            &context,
            &record.id,
            "runtime-one",
            "claude",
            "provider-one",
            &json!({"effort": "high", "model": "sk-secret-canary"}),
        )
        .unwrap();
        assert_eq!(
            ModelSettings::for_record(&crate::load_session_record(&context, &record.id).unwrap()),
            ModelSettings {
                model: None,
                reasoning_effort: Some("high".into())
            }
        );
        let mut resumed = current;
        resumed.runtime.as_mut().unwrap().launch_id = "runtime-two".into();
        assert_eq!(
            ModelSettings::for_record(&resumed).model.as_deref(),
            Some("opus")
        );
    }
    #[test]
    fn fresh_primary_codex_settings_survive_resume_capture() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let mut record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1", "id": "fresh-model-test", "agent": "codex", "mode": "interactive", "cwd": "/work/example", "tmux_session": "fresh-model-test",
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {"kind": "tmux", "tmux_session": "fresh-model-test", "generation": 1, "started_at": "2030-01-01T00:00:00Z", "launch_id": "fresh-runtime"}
        })).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        observe_primary_codex_thread(
            &context,
            &record.id,
            "fresh-runtime",
            "primary-thread",
            &json!({"model": "resolved-model", "reasoningEffort": "medium"}),
        )
        .unwrap();
        record = crate::load_session_record(&context, &record.id).unwrap();
        assert_eq!(
            ModelSettings::for_record(&record).model.as_deref(),
            Some("resolved-model")
        );
        record.provider_resume = Some(serde_json::from_value(json!({"provider": "codex", "session_id": "primary-thread", "captured_at": "2030-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []})).unwrap());
        assert_eq!(
            ModelSettings::for_record(&record)
                .reasoning_effort
                .as_deref(),
            Some("medium")
        );
        record.provider_resume.as_mut().unwrap().session_id = "other-thread".into();
        assert_eq!(ModelSettings::for_record(&record), ModelSettings::default());
        // A prior runtime's primary identity must not fence a fresh launch.
        record.provider_resume = None;
        record.runtime.as_mut().unwrap().launch_id = "replacement-runtime".into();
        crate::write_session_record(&context, &record).unwrap();
        observe_primary_codex_thread(
            &context,
            &record.id,
            "replacement-runtime",
            "replacement-thread",
            &json!({"model": "replacement-model"}),
        )
        .unwrap();
        assert_eq!(
            ModelSettings::for_record(&crate::load_session_record(&context, &record.id).unwrap())
                .model
                .as_deref(),
            Some("replacement-model")
        );
    }
    #[test]
    fn session_model_claude_start_hook_ignores_auxiliary_metadata() {
        let tmp = tempfile::TempDir::new().unwrap();
        let context = CliContext {
            state_dir: tmp.path().to_path_buf(),
            host: None,
        };
        let record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1", "id": "claude-hook-model", "agent": "claude", "mode": "interactive", "cwd": "/work/example", "tmux_session": "claude-hook-model",
            "created_at": "2030-01-01T00:00:00Z", "updated_at": "2030-01-01T00:00:00Z",
            "runtime": {"kind": "tmux", "tmux_session": "claude-hook-model", "generation": 1, "started_at": "2030-01-01T00:00:00Z", "launch_id": "hook-runtime"},
            "provider_resume": {"provider": "claude", "session_id": "primary-session", "captured_at": "2030-01-01T00:00:00Z", "capture_method": "fixture", "resume_args": []}
        })).unwrap();
        crate::write_session_record(&context, &record).unwrap();
        for (auxiliary, model, accepted) in [(false, "opus", true), (true, "sonnet", false)] {
            let mut raw = json!({"hook_event_name": "SessionStart", "session_id": "primary-session", "model": model, "effort": {"level": "medium"}});
            if auxiliary {
                raw["agent_id"] = json!("auxiliary-agent");
            }
            let bytes = serde_json::to_vec(&raw).unwrap();
            assert_eq!(
                crate::activity::ingest_provider_hook_input(
                    &context,
                    crate::AgentKind::Claude,
                    None,
                    crate::activity::ProviderHookInput {
                        id: &record.id,
                        runtime_id: "hook-runtime",
                        payload: &bytes,
                        attention_authority: None,
                    }
                )
                .unwrap(),
                accepted
            );
            let persisted = crate::load_session_record(&context, &record.id).unwrap();
            let view = crate::session_view(&context, &persisted, Some("stopped".into()), None);
            let json = serde_json::to_value(view).unwrap();
            assert_eq!(json["model"], "opus");
            assert_eq!(json["reasoning_effort"], "medium");
        }
    }
}

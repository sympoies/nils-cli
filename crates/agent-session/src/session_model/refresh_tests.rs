use super::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::fs;

const PROVIDER_SESSION: &str = "provider-session";

struct Fixture {
    _tmp: tempfile::TempDir,
    context: CliContext,
    root: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let context = CliContext {
            state_dir: tmp.path().join("state"),
            host: None,
        };
        let root = tmp.path().join("provider");
        Self {
            _tmp: tmp,
            context,
            root,
        }
    }

    fn seed(&self, agent: &str, agent_args: &[&str], extra: Value) -> SessionRecord {
        let mut record: SessionRecord = serde_json::from_value(json!({
            "schema_version": "agent-session.session.v1",
            "id": "refresh-fixture",
            "agent": agent,
            "mode": "interactive",
            "title": null,
            "cwd": "/workspace/project",
            "tmux_session": "refresh-fixture",
            "prompt_file": null,
            "log_file": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "provider_resume": {
                "provider": agent,
                "session_id": PROVIDER_SESSION,
                "captured_at": "2026-01-01T00:00:00Z",
                "capture_method": "fixture",
                "resume_args": []
            },
            "runtime": {
                "kind": "tmux",
                "tmux_session": "refresh-fixture",
                "generation": 1,
                "started_at": "2026-01-01T00:00:00Z",
                "launch_id": "refresh-runtime",
                "agent_profile_provider_config_dir": self.root,
            },
        }))
        .unwrap();
        record.agent_args = agent_args.iter().map(|arg| arg.to_string()).collect();
        for (key, value) in extra.as_object().into_iter().flatten() {
            let mut document = serde_json::to_value(&record).unwrap();
            document[key] = value.clone();
            record = serde_json::from_value(document).unwrap();
        }
        ModelSettings::store_launch(&mut record);
        crate::write_session_record(&self.context, &record).unwrap();
        record
    }

    fn claude_transcript(&self, model: &str) {
        let dir = self.root.join("projects/project");
        fs::create_dir_all(&dir).unwrap();
        let row = json!({
            "type": "assistant", "sessionId": PROVIDER_SESSION, "cwd": "/workspace/project",
            "timestamp": "2026-01-01T00:00:00Z", "message": {"model": model},
        });
        fs::write(
            dir.join(format!("{PROVIDER_SESSION}.jsonl")),
            format!("{row}\n"),
        )
        .unwrap();
    }

    fn codex_rollout(&self, model: &str, effort: Option<&str>) {
        let dir = self.root.join("sessions/2026/01/01");
        fs::create_dir_all(&dir).unwrap();
        let rows = [
            json!({"type": "session_meta", "payload": {
                "id": PROVIDER_SESSION, "cwd": "/workspace/project", "source": "cli",
                "timestamp": "2026-01-01T00:00:00Z"}}),
            json!({"type": "turn_context", "payload": {"model": model, "effort": effort}}),
        ];
        let body = rows
            .iter()
            .map(|row| format!("{row}\n"))
            .collect::<String>();
        fs::write(dir.join("rollout.jsonl"), body).unwrap();
    }

    fn stored(&self) -> SessionRecord {
        crate::load_session_record(&self.context, "refresh-fixture").unwrap()
    }
}

fn changed(result: &Value) -> Vec<&str> {
    result["changed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect()
}

fn unavailable(result: &Value) -> Vec<&str> {
    result["unavailable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|field| field.as_str().unwrap())
        .collect()
}

#[test]
fn claude_refresh_adopts_the_transcript_model_and_drops_the_unprovable_effort() {
    let fixture = Fixture::new();
    fixture.seed(
        "claude",
        &["--model", "old-model", "--effort", "high"],
        json!({}),
    );
    fixture.claude_transcript("claude-opus-5-5");

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();

    assert_eq!(changed(&result), vec!["model", "reasoning_effort"]);
    assert_eq!(
        ModelSettings::for_record(&fixture.stored()),
        ModelSettings {
            model: Some("claude-opus-5-5".into()),
            reasoning_effort: None,
        }
    );
    assert_eq!(result["session"]["model"], "claude-opus-5-5");
    assert_eq!(unavailable(&result), vec!["reasoning_effort", "account"]);
}

#[test]
fn codex_refresh_adopts_the_rollout_model_and_effort() {
    let fixture = Fixture::new();
    fixture.seed(
        "codex",
        &["-m", "old-model", "-c", "model_reasoning_effort=low"],
        json!({}),
    );
    fixture.codex_rollout("example-model", Some("high"));

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();

    assert_eq!(changed(&result), vec!["model", "reasoning_effort"]);
    assert_eq!(
        ModelSettings::for_record(&fixture.stored()),
        ModelSettings {
            model: Some("example-model".into()),
            reasoning_effort: Some("high".into()),
        }
    );
    assert_eq!(unavailable(&result), vec!["account"]);
}

#[test]
fn refresh_that_finds_nothing_new_leaves_the_record_untouched() {
    let fixture = Fixture::new();
    fixture.seed(
        "codex",
        &["-m", "example-model", "-c", "model_reasoning_effort=high"],
        json!({}),
    );
    fixture.codex_rollout("example-model", Some("high"));

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();

    assert_eq!(changed(&result), Vec::<&str>::new());
    assert_eq!(fixture.stored().updated_at, "2026-01-01T00:00:00Z");
}

#[test]
fn refresh_without_provider_evidence_keeps_the_recorded_settings() {
    let fixture = Fixture::new();
    fixture.seed("claude", &["--model", "recorded-model"], json!({}));

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();

    assert_eq!(changed(&result), Vec::<&str>::new());
    assert_eq!(
        unavailable(&result),
        vec!["model", "reasoning_effort", "account"]
    );
    assert_eq!(
        ModelSettings::for_record(&fixture.stored())
            .model
            .as_deref(),
        Some("recorded-model")
    );
}

#[test]
fn refresh_rejects_agents_without_a_provider_transcript() {
    let fixture = Fixture::new();
    fixture.seed("dsh", &[], json!({}));

    let error = refresh(&fixture.context, "refresh-fixture").unwrap_err();

    assert_eq!(error.code(), "refresh-unsupported-agent");
}

fn titled(topic: &str, references: &[&str]) -> Value {
    let title = std::iter::once(topic.to_string())
        .chain(references.iter().map(|reference| reference.to_string()))
        .collect::<Vec<_>>()
        .join(" ");
    json!({
        "title": title,
        "title_state": {
            "topic": topic, "topic_source": "auto",
            "references": references, "activity": null,
        },
        "work": {
            "program": null,
            "issues": [{"provider": "github", "repository": "sympoies/nils-cli", "number": 3022}],
            "inherited": false, "revision": 1,
        },
    })
}

#[test]
fn refresh_regenerates_a_structured_title_from_the_current_work_refs() {
    let fixture = Fixture::new();
    fixture.seed("codex", &[], titled("Fix parser", &["agent-console #1117"]));

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();

    assert_eq!(changed(&result), vec!["title"]);
    let stored = fixture.stored();
    assert_eq!(stored.title.as_deref(), Some("Fix parser nils-cli #3022"));
    assert_eq!(stored.title_revision, 1);
    assert_eq!(result["session"]["title"], "Fix parser nils-cli #3022");
}

#[test]
fn refresh_leaves_pinned_and_plain_titles_alone() {
    let fixture = Fixture::new();
    let mut pinned = titled("Fix parser", &["agent-console #1117"]);
    pinned["title_mode"] = json!("pinned");
    let mut record = fixture.seed("codex", &[], pinned);
    record.extra.insert("title_mode".into(), json!("pinned"));
    crate::write_session_record(&fixture.context, &record).unwrap();

    let result = refresh(&fixture.context, "refresh-fixture").unwrap();
    assert_eq!(changed(&result), Vec::<&str>::new());
    assert_eq!(
        fixture.stored().title.as_deref(),
        Some("Fix parser agent-console #1117")
    );

    let mut plain = titled("Fix parser", &[]);
    plain["title_state"] = Value::Null;
    plain["title"] = json!("laoda#480 Fix parser");
    fixture.seed("codex", &[], plain);
    let result = refresh(&fixture.context, "refresh-fixture").unwrap();
    assert_eq!(changed(&result), Vec::<&str>::new());
    assert_eq!(
        fixture.stored().title.as_deref(),
        Some("laoda#480 Fix parser")
    );
}

//! Modeled primary transcripts; no provider runtime is executed.
use super::native_notice::*;
use crate::send::claude_notice::{Binding, Launch};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::time::{Duration, Instant};

fn fixture(root: &std::path::Path) -> (crate::SessionRecord, Binding, std::path::PathBuf, String) {
    let path = root.join("projects/synthetic/provider-one.jsonl");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{}\n",
            json!({"sessionId":"provider-one","cwd":"/work/example"})
        ),
    )
    .unwrap();
    let record = serde_json::from_value(json!({
        "schema_version":"agent-session.session.v1", "id":"synthetic", "agent":"claude", "mode":"interactive",
        "cwd":"/work/example", "tmux_session":"modeled", "created_at":"2000-01-01T00:00:00Z","updated_at":"2000-01-01T00:00:00Z",
        "runtime":{"kind":"tmux","tmux_session":"modeled","launch_id":"runtime-one","generation":1,
            "started_at":"2000-01-01T00:00:00Z","agent_profile_provider_config_dir":root},
        "provider_resume":{"provider":"claude","session_id":"provider-one","captured_at":"2000-01-01T00:00:00Z","capture_method":"fixture","resume_args":[]}
    })).unwrap();
    let binding = Binding {
        launch: Launch {
            session_id: "synthetic".into(),
            incarnation: "runtime-one".into(),
            generation: 1,
            profile: None,
        },
        provider: "claude".into(),
        provider_session_id: "provider-one".into(),
        provider_version: "2.1.296".into(),
        module_sha256: "a".repeat(64),
    };
    let text =
        "<!-- agent-notice:v1 kind=mailbox source=synthetic -->\n\nAlpha  \nBeta \n".to_string();
    (record, binding, path, text)
}

fn native_row(text: &str, uuid: &str) -> serde_json::Value {
    json!({"type":"user","sessionId":"provider-one","version":"2.1.296","uuid":uuid,
        "promptSource":"system","isSidechain":false,
        "origin":{"kind":"plugin","name":"agent-session-claude-notice","asUser":true},
        "message":{"role":"user","content":[{"type":"text","text":text}]}})
}

#[test]
fn native_notice_observer_requires_a_unique_complete_new_row() {
    for duplicate in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let (record, binding, path, text) = fixture(temp.path());
        let mut baseline = Baseline::capture(&record).unwrap();
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(
            file,
            "{}",
            native_row(&text, "11111111-1111-4111-8111-111111111111")
        )
        .unwrap();
        if duplicate {
            writeln!(
                file,
                "{}",
                native_row(&text, "22222222-2222-4222-8222-222222222222")
            )
            .unwrap();
        }
        let proof = baseline.confirm(&binding, &text, Instant::now() + Duration::from_secs(1));
        assert_eq!(proof.is_none(), duplicate);
        if let Some(proof) = proof {
            assert_eq!(proof.row.text, text);
        }
    }
}

#[test]
fn native_notice_observer_refuses_partial_and_replaced_sources() {
    for replace in [false, true] {
        let temp = tempfile::TempDir::new().unwrap();
        let (record, binding, path, text) = fixture(temp.path());
        let mut baseline = Baseline::capture(&record).unwrap();
        let value = native_row(&text, "11111111-1111-4111-8111-111111111111");
        if replace {
            let replacement = path.with_extension("replacement");
            fs::write(&replacement, format!("{value}\n")).unwrap();
            fs::rename(replacement, &path).unwrap();
        } else {
            write!(
                fs::OpenOptions::new().append(true).open(path).unwrap(),
                "{value}"
            )
            .unwrap();
        }
        assert!(
            baseline
                .confirm(&binding, &text, Instant::now() + Duration::from_millis(5))
                .is_none()
        );
    }
}

//! Public notice CLI boundary: metadata must fail before loading a session or body.
use nils_test_support::cmd::{CmdOptions, run_resolved};
use pretty_assertions::assert_eq;

#[test]
fn invalid_notice_metadata_has_no_side_effect() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("absent-state");
    let body = temp.path().join("absent-body");
    for (kind, source, id) in [
        ("Bad", "scheduler", "valid"),
        ("mailbox", "source-->injected", "valid"),
        ("mailbox", "scheduler", "bad\nid"),
    ] {
        let output = run_resolved(
            "agent-session",
            &[
                "--state-dir",
                state.to_str().unwrap(),
                "notice",
                "missing",
                "--kind",
                kind,
                "--source",
                source,
                "--id",
                id,
                "--body-file",
                body.to_str().unwrap(),
                "--format",
                "json",
            ],
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        assert_eq!(output.code, 64);
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "invalid-notice-field"
        );
        assert!(
            !state.exists(),
            "invalid fields must not create session state"
        );
    }
}

#[test]
fn notice_writers_are_disabled_by_default() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("absent-state");
    let body = temp.path().join("absent-body");
    for input in [
        vec![
            "notice",
            "missing",
            "--kind",
            "mailbox",
            "--source",
            "scheduler",
            "--body-file",
            body.to_str().unwrap(),
        ],
        vec![
            "start",
            "--agent",
            "claude",
            "--notice-kind",
            "dispatch.bootstrap",
            "--notice-source",
            "scheduler",
            "--prompt-file",
            body.to_str().unwrap(),
        ],
    ] {
        let mut args = vec!["--state-dir", state.to_str().unwrap()];
        args.extend(input);
        args.extend(["--format", "json"]);
        let output = run_resolved(
            "agent-session",
            &args,
            &CmdOptions::new().with_env_remove_prefix("AGENT_SESSION_"),
        );
        assert_eq!(output.code, 1, "{}", output.stderr_text());
        assert_eq!(
            output.stdout_json()["error"]["code"],
            "notice-writers-disabled"
        );
        assert!(!state.exists());
    }
}

#[test]
fn shared_notice_corpus_matches_parser_and_encoder() {
    use agent_session::notice::{NoticeEnvelope, parse_notice};
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/session-notices-v1.json")).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let actual = parse_notice(input).map(|notice| {
            serde_json::json!({
                "kind":notice.metadata.kind(), "source":notice.metadata.source(),
                "id":notice.metadata.id(), "body":notice.body,
            })
        });
        assert_eq!(
            actual.unwrap_or(serde_json::Value::Null),
            case["notice"],
            "{}",
            case["name"]
        );
        if let Some(body) = case["notice"]["body"].as_str() {
            let metadata = NoticeEnvelope::new(
                case["notice"]["kind"].as_str().unwrap(),
                case["notice"]["source"].as_str().unwrap(),
                case["notice"]["id"].as_str(),
            )
            .unwrap();
            let encoded = metadata.encode(body).unwrap();
            assert_eq!(
                parse_notice(&encoded).unwrap().body.as_bytes(),
                body.as_bytes()
            );
        }
    }
}

#[test]
fn encoder_refuses_invalid_metadata_and_ambiguous_bodies() {
    use agent_session::notice::{NoticeEnvelope, NoticeError};
    for token in [
        "", "UPPER", "a--b", "a..b", "a_", "a/b", "a b", "a\nb", "a-->b", "é",
    ] {
        assert!(
            NoticeEnvelope::new(token, "scheduler", None).is_err(),
            "{token}"
        );
        assert!(
            NoticeEnvelope::new("mailbox", token, None).is_err(),
            "{token}"
        );
    }
    for id in ["", "a--b", "a/", "a\nb", "a b", "a-->"] {
        assert!(NoticeEnvelope::new("mailbox", "scheduler", Some(id)).is_err());
    }
    let metadata = NoticeEnvelope::new("mailbox", "scheduler", Some("1Aa-2.b_c")).unwrap();
    assert_eq!(metadata.encode(""), Err(NoticeError::EmptyBody));
    assert_eq!(
        metadata.encode("<!-- agent-notice:v1 kind=mailbox source=scheduler -->\n\nBody"),
        Err(NoticeError::DoubleHeader)
    );
}

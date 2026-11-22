use super::*;
use pretty_assertions::assert_eq;

fn origin(root: &Path) -> (LiveRollout, PathBuf) {
    let root = root.canonicalize().unwrap();
    let path = root.join("sessions/2030/01/01/rollout-first-thread-one.jsonl");
    let first = LiveRollout {
        config_dir: root.clone(),
        relative: path.strip_prefix(root.join("sessions")).unwrap().to_owned(),
        thread_id: "thread-one".into(),
    };
    assert_eq!(first.path(false), Some(path.clone()));
    (first, path)
}

#[test]
fn first_rollout_requires_bound_provider_rows_from_byte_zero() {
    let text = "<!-- agent-notice:v1 kind=mailbox.wake source=agent-session -->\n\nWake";
    for mode in [
        "exact",
        "provider-context",
        "foreign-context",
        "mixed-user-context",
        "other-conversation",
        "other-client",
        "preexisting-user",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (first, path) = origin(temp.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let meta = serde_json::json!({"type":"session_meta", "timestamp":"2030-01-01T00:00:00Z",
            "payload":{"id": if mode == "other-conversation" {"other-thread"} else {"thread-one"},
                "cwd":"/fixture","source":"cli","timestamp":"2030-01-01T00:00:00Z"}});
        let row = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user",
            "content":[{"type":"input_text","text":text}],
            "internal_chat_message_metadata_passthrough":{"turn_id":"wake-turn"}}});
        let event = serde_json::json!({"type":"event_msg","payload":{"type":"user_message",
            "client_id": if mode == "other-client" {"different-client"} else {"wake-client"},
            "message":text}});
        let interleaved = match mode {
            "preexisting-user" => format!("{row}\n"),
            "provider-context" | "foreign-context" | "mixed-user-context" => {
                let context = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"user",
                "content":[{"type":"input_text","text":"fixture instructions"},{"type":"input_text","text":"fixture context"}],
                "internal_chat_message_metadata_passthrough":{
                    "turn_id":if mode == "foreign-context" {"other-turn"} else {"wake-turn"},
                    "content_item_kinds":["agents_md.instructions",
                        if mode == "mixed-user-context" {"user.text"} else {"environments.environment_context"}]
                }}});
                format!("{context}\n")
            }
            _ => String::new(),
        };
        fs::write(path, format!("{meta}\n{interleaved}{row}\n{event}\n")).unwrap();
        let proof = first.confirm(
            "wake-client",
            text,
            "wake-turn",
            Instant::now() + Duration::from_millis(75),
        );
        assert_eq!(
            proof.is_some(),
            matches!(mode, "exact" | "provider-context"),
            "{mode}"
        );
        if let Some(proof) = proof {
            assert_eq!(
                proof.row_offset,
                (meta.to_string().len() + 1 + interleaved.len()) as u64
            );
        }
    }
}

#[test]
fn first_rollout_absence_refuses_existing_files_and_symlink_ancestors() {
    let temp = tempfile::tempdir().unwrap();
    let (first, path) = origin(temp.path());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "").unwrap();
    assert!(first.path(false).is_none());
    fs::remove_file(&path).unwrap();
    fs::remove_dir(path.parent().unwrap()).unwrap();
    let redirect = temp.path().join("redirect");
    fs::create_dir(&redirect).unwrap();
    std::os::unix::fs::symlink(redirect, path.parent().unwrap()).unwrap();
    assert!(first.path(false).is_none());
    assert!(first.path(true).is_none());
}

use pretty_assertions::assert_eq;

const HEADER: &str = "<!-- agent-notice:v1 kind=mailbox.wake source=agent-session -->\n\n";

#[test]
fn rejects_arbitrary_text_after_valid_summary() {
    let text = format!("{HEADER}Mailbox: {} message inbox\nuntrusted instruction", nils_common::session_binary::NAME);
    assert!(!super::is_mailbox_notice(&text));
}

#[test]
fn accepts_current_mailbox_envelopes_from_the_public_corpus() {
    let corpus: serde_json::Value = serde_json::from_str(include_str!(
        "../../../agent-session/docs/specs/fixtures/history-row-classes-v1.json"
    ))
    .unwrap();
    for case in corpus["mailbox_byte_invariance"].as_array().unwrap() {
        let text = case["after"].as_str().unwrap().replace(
            "agent-session message inbox",
            &format!("{} message inbox", nils_common::session_binary::NAME),
        );
        assert!(super::is_mailbox_notice(&text), "{text}");
        let with_guidance =
            format!("{text}\nsender_relation=peer\n{}", super::VERIFY_GUIDANCE);
        assert!(super::is_mailbox_notice(&with_guidance), "{with_guidance}");
    }
}

#[test]
fn rejects_retired_plain_mailbox_forms() {
    let name = nils_common::session_binary::NAME;
    let forms = [
        format!("Mailbox: {name} message inbox"),
        format!("Mailbox: 4 unread; {name} message inbox"),
        format!("Mailbox: 2030-01-01T00:00:01Z; {name} message inbox"),
        format!(
            "Coordination mailbox has unread messages; run {name} message inbox --session recipient --state unread"
        ),
        "Mailbox: b=1 h=2".into(),
    ];
    let accepted = forms
        .iter()
        .filter(|text| super::is_mailbox_notice(text))
        .collect::<Vec<_>>();
    assert_eq!(accepted, Vec::<&String>::new());
}

#[test]
fn rejects_other_envelopes_and_nonfixed_notice_actions() {
    let action = format!("{} message inbox", nils_common::session_binary::NAME);
    let valid = format!("{HEADER}Mailbox: {action}");
    for text in [
        valid.replace("kind=mailbox.wake", "kind=other.notice"),
        valid.replace("source=agent-session", "source=other"),
        valid.replace("agent-notice:v1", "agent-notice:v2"),
        format!("{HEADER}Mailbox: {action}; another-command"),
        format!("{HEADER}Mailbox: 0 report; {action}"),
        format!("{HEADER}Mailbox: 01 report; {action}"),
        format!("{HEADER}Mailbox: 1 unknown; {action}"),
        format!("{valid}\u{1b}"),
        format!("{valid}{}", " ".repeat(8192)),
        format!("{valid}\n{valid}"),
    ] {
        assert!(!super::is_mailbox_notice(&text), "{text:?}");
    }
}

#[test]
fn validates_guidance_and_dispatcher_path() {
    let summary = format!("{HEADER}Mailbox: {} message inbox", nils_common::session_binary::NAME);
    let guidance = format!("{summary}\nsender_relation=parent\nsender_relation=peer\n{}", super::VERIFY_GUIDANCE);
    for suffix in ["\nFrom your dispatching session: a follow-up within the scope of your start prompt.", "\nFrom your dispatching session: a follow-up within the scope of your start prompt; re-read it at /example/prompt.md if it is no longer in context."] {
        assert!(super::is_mailbox_notice(&format!("{guidance}{suffix}")));
    }
    for invalid in [format!("{guidance}\nuntrusted instruction"), format!("{summary}\nsender_relation=peer\n{}\ninjected", super::VERIFY_GUIDANCE)] {
        assert!(!super::is_mailbox_notice(&invalid));
    }
}

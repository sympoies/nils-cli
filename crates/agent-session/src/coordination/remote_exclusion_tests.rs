use super::*;
use pretty_assertions::assert_eq;

fn snapshot(reason: Option<&str>) -> Value {
    let mut value = json!({"schema_version":"agent-session.remote-peers.v1", "peers":[]});
    if let Some(reason) = reason {
        value["excluded"] = json!([{
            "machine":"destination-machine", "session_id":TARGET, "reason":reason
        }]);
    }
    value
}

#[test]
fn excluded_peers_preserve_json_and_label_text_separately() {
    for reason in ["ownership-binding-stale", "future-exclusion-reason"] {
        let (_temp, context, _capability, token) = sender_fixture();
        let mut expected = snapshot(Some(reason));
        expected["peers"] = peer_snapshot(true, None)["peers"].clone();
        let relay = ScriptedRelay::new(vec![expected.clone()], Vec::new());
        let value = peers(&context, &relay.config, SENDER, &token).unwrap();
        assert_eq!(value, expected);
        let text = super::super::super::render_value_text(&value);
        assert!(text.contains("Authorized peers:"), "{text}");
        assert!(text.contains("Excluded peers:"), "{text}");
        assert!(
            text.contains(&format!("excluded destination-machine {TARGET} {reason}")),
            "{text}"
        );
        relay.assert_no_protocol_error();
    }
}

#[test]
fn excluded_send_reports_reason_and_owner_authorized_recovery_without_enqueueing() {
    for reason in ["ownership-binding-stale", "future-exclusion-reason"] {
        let (_temp, context, _capability, token) = sender_fixture();
        let relay = ScriptedRelay::new(vec![snapshot(Some(reason))], Vec::new());
        let error = submit_fixture(&context, &relay, &token).unwrap_err();
        let error = error.into_inner();
        assert_eq!(error.exit_code, 65);
        assert_eq!(error.code, "remote-recipient-excluded");
        assert!(error.message.contains(reason), "{}", error.message);
        let details = error.details.unwrap();
        assert_eq!(details["machine"], "destination-machine");
        assert_eq!(details["session_id"], TARGET);
        assert_eq!(details["reason"], reason);
        if reason == "ownership-binding-stale" {
            for hint in [
                "owner-authorized",
                "import-owner --claim-session --expected-incarnation",
                "check first, then apply",
            ] {
                assert!(error.message.contains(hint), "{}", error.message);
            }
            assert_eq!(details["recovery"]["kind"], "owner-authorized");
        } else {
            assert!(details["recovery"].is_null());
            assert!(!error.message.contains("import-owner"));
        }
        assert!(
            lock_journal(&context)
                .unwrap()
                .registry
                .remote_outbox
                .is_empty()
        );
        assert!(relay.posted_envelopes().is_empty());
        relay.assert_no_protocol_error();
    }
}

#[test]
fn excluded_field_absent_keeps_existing_peers_and_not_discovered_behavior() {
    let (_temp, context, _capability, token) = sender_fixture();
    let expected = snapshot(None);
    let relay = ScriptedRelay::new(vec![expected.clone()], Vec::new());
    let value = peers(&context, &relay.config, SENDER, &token).unwrap();
    assert_eq!(value, expected);
    assert_eq!(
        super::super::super::render_value_text(&value),
        serde_json::to_string_pretty(&expected).unwrap() + "\n"
    );
    assert_eq!(
        submit_fixture(&context, &relay, &token)
            .unwrap_err()
            .into_inner()
            .code,
        "remote-recipient-not-discovered"
    );
    relay.assert_no_protocol_error();
}

#[test]
fn excluded_send_matches_both_address_fields_and_keeps_authorized_peer_priority() {
    for (machine, session, authorized) in [
        ("different-machine", TARGET, false),
        ("destination-machine", "different-session", false),
        ("destination-machine", TARGET, true),
    ] {
        let (_temp, context, _capability, token) = sender_fixture();
        let mut value = snapshot(Some("ownership-binding-stale"));
        value["excluded"][0]["machine"] = json!(machine);
        value["excluded"][0]["session_id"] = json!(session);
        if authorized {
            value["peers"] = peer_snapshot(true, None)["peers"].clone();
        }
        let relay = ScriptedRelay::new(vec![value], Vec::new());
        let result = submit_fixture(&context, &relay, &token);
        if authorized {
            assert_eq!(result.unwrap()["state"], "queued");
        } else {
            assert_eq!(
                result.unwrap_err().into_inner().code,
                "remote-recipient-not-discovered"
            );
        }
        relay.assert_no_protocol_error();
    }
}

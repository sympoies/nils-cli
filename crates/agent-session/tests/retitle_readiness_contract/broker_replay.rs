use pretty_assertions::assert_eq;
use serde_json::json;

use super::{Fixture, INCARNATION, SESSION_ID, ServeProcess, write_executable};

#[test]
fn failed_broker_requests_replay_the_same_recovery_details_after_restart() {
    for (name, account, fallback, expected_action, expected_class) in [
        (
            "automatic selection",
            json!({"account_selection": "default_with_capacity"}),
            None,
            "configure_account_broker",
            Some("account_broker_unavailable"),
        ),
        (
            "configured account",
            json!({"account": "acct1"}),
            None,
            "configure_account_broker",
            Some("account_broker_unavailable"),
        ),
        (
            "generic fallback failure after broker failure",
            json!({"account_selection": "default_with_capacity"}),
            Some(json!({"provider": "command", "argv": ["/bin/false"]})),
            "retry",
            None,
        ),
    ] {
        let fixture = Fixture::new();
        write_executable(
            &fixture.broker_bin,
            "#!/bin/sh\nprintf '%s\\n' 'not json'\n",
        );
        let mut config = account;
        config["provider"] = json!("codex_subscription");
        config["codex_bin"] = json!(fixture.provider_bin);
        config["model"] = json!("gpt-5.6-luna");
        config["timeout_ms"] = json!(10_000);
        if let Some(fallback) = fallback {
            config["fallback"] = fallback;
        }
        let config = config.to_string();
        let request = json!({
            "schema_version": "agent-session.session-retitle.request.v2",
            "trigger": "manual",
            "idempotency_key": "broker-failure-replay",
            "expected_session_incarnation": INCARNATION,
            "expected_title_revision": 0,
        });
        let path = format!("/sessions/{SESSION_ID}/retitle");
        let server = ServeProcess::spawn(&fixture, Some(&config), false, true);
        let first = server.post(&path, &request);
        assert_eq!(first.status, 503, "{name}: {}", first.body);
        assert_eq!(first.body["error"]["code"], "retitle-provider-unavailable");
        let details = &first.body["error"]["details"];
        assert_eq!(details["next_action"], expected_action, "{name}");
        assert_eq!(
            details["recovery"]["strategy"],
            if expected_class.is_some() {
                "configure_account_broker"
            } else {
                "retry_request"
            }
        );
        assert_eq!(details["failure_class"].as_str(), expected_class, "{name}");
        drop(server);

        // Replay must use the durable terminal attempt, even when the broker's
        // current result differs and the serving process has been replaced.
        write_executable(&fixture.broker_bin, "#!/bin/sh\nexit 3\n");
        let restarted = ServeProcess::spawn(&fixture, Some(&config), false, true);
        let replay = restarted.post(&path, &request);
        assert_eq!(replay.status, first.status, "{name}: {}", replay.body);
        assert_eq!(replay.body["error"]["code"], first.body["error"]["code"]);
        assert_eq!(replay.body["error"]["details"], *details, "{name}");
    }
}

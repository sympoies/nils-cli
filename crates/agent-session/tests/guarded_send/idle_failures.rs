//! Observation failures name a fixed reason without exposing provider values.
use super::{GUARDS, Server};
use crate::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

fn expected_hint(reason: &str) -> &'static str {
    match reason {
        "connect_failed" => "App-server not reachable: restart the session before sending again.",
        "initialize_failed" => {
            "App-server initialization failed: restart the session before sending again."
        }
        "observation_deadline_expired" => {
            "Native idle observation timed out: inspect app-server health; restart the session if the timeout persists."
        }
        "loaded_list_rpc_failed" => {
            "Loaded thread identities could not be read: inspect app-server health; restart the session if the failure persists."
        }
        "thread_read_rpc_failed" => {
            "Native thread state could not be read: inspect app-server health; restart the session if the failure persists."
        }
        "thread_not_loaded" => {
            "Thread not loaded: restart the session to reload its conversation before sending again."
        }
        "thread_status_missing" => {
            "Provider thread status is missing: check the configured Codex app-server version before restarting the session."
        }
        "thread_status_unknown" => {
            "Provider thread status is unrecognized: check the configured Codex app-server version before restarting the session."
        }
        _ => panic!("unexpected fixture reason"),
    }
}

fn refuses(mode: &'static str, reason: &str) {
    for notice in [false, true] {
        let fixture = Fixture::new("codex");
        let _lease = fixture.bind_codex();
        let server = Server::start(&fixture, mode);
        let recovery = fixture.arm_recovery();
        if mode == "connect-error" {
            fs::remove_file(fixture.root().join("cx.sock")).unwrap();
        }
        let output = if notice {
            fixture.notice(&[])
        } else {
            fixture.send(GUARDS)
        };
        let value = fixture.refused(&output, "send-idle-unobservable");
        assert_eq!(value["error"]["details"]["native_idle_reason"], reason);
        assert_eq!(value["error"]["hint"], expected_hint(reason));
        for private in [
            "private-observation-error",
            "private-observation-token",
            "futureStatus",
            "notLoaded",
        ] {
            assert!(!output.stdout_text().contains(private), "{value}");
        }
        fixture.unchanged_control_state(&recovery);
        fixture.no_delivery();
        assert!(server.mutations().is_empty());
        if mode.starts_with("unknown-retained") {
            // send_observing, guarded routing and native::deliver each do
            // two shared reads. Retained reads are then 7 and 10.
            let claimed = mode == "unknown-retained-claimed";
            assert_eq!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|r| r["method"] == "thread/read")
                    .count(),
                if claimed { 10 } else { 7 }
            );
            assert_eq!(
                value["error"]["details"]["stage"],
                if claimed { "claimed" } else { "admission" }
            );
            if claimed {
                let repeated = if notice {
                    fixture.notice(&[])
                } else {
                    fixture.send(GUARDS)
                };
                fixture.refused(&repeated, "send-idempotency-key-used");
                assert!(server.mutations().is_empty());
            }
        }
        if mode == "retained-connect-error" {
            let requests = server.requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r["method"] == "thread/loaded/list")
                    .count(),
                5
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r["method"] == "thread/read")
                    .count(),
                4
            );
        }
    }
}

#[test]
fn native_idle_reason_connect_failure() {
    refuses("connect-error", "connect_failed");
}

#[test]
fn native_idle_reason_websocket_handshake_failure() {
    refuses("handshake-error", "connect_failed");
}

#[test]
fn native_idle_reason_retained_connect_preserves_failure() {
    refuses("retained-connect-error", "loaded_list_rpc_failed");
}

#[test]
fn native_idle_reason_initialize_failure() {
    refuses("initialize-error", "initialize_failed");
}

#[test]
fn native_idle_reason_observation_deadline() {
    refuses("initialize-timeout", "observation_deadline_expired");
}

#[test]
fn native_idle_reason_loaded_list_failure() {
    refuses("loaded-list-error", "loaded_list_rpc_failed");
}

#[test]
fn native_idle_reason_thread_read_failure() {
    refuses("thread-read-error", "thread_read_rpc_failed");
}

#[test]
fn native_idle_reason_thread_not_loaded() {
    refuses("not-loaded", "thread_not_loaded");
}

#[test]
fn native_idle_reason_thread_status_missing() {
    refuses("missing-status", "thread_status_missing");
}

#[test]
fn native_idle_reason_thread_status_unknown() {
    refuses("unknown-status", "thread_status_unknown");
}

#[test]
fn native_idle_reason_retained_client_preserves_failure() {
    refuses("unknown-retained", "thread_status_unknown");
}

#[test]
fn native_idle_reason_claimed_retained_client_preserves_failure_and_key() {
    refuses("unknown-retained-claimed", "thread_status_unknown");
}

//! Observation failures name a fixed reason without exposing provider values.
use super::{GUARDS, Server};
use crate::Fixture;
use pretty_assertions::assert_eq;
use std::fs;

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
        assert!(
            value["error"]["hint"]
                .as_str()
                .is_some_and(|hint| !hint.is_empty()),
            "{value}"
        );
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
            // send_observing and native::deliver each do two shared reads;
            // the retained client then reads once, and again after claim.
            let claimed = mode == "unknown-retained-claimed";
            assert_eq!(
                server
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|r| r["method"] == "thread/read")
                    .count(),
                if claimed { 8 } else { 5 }
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
    }
}

#[test]
fn native_idle_reason_connect_failure() {
    refuses("connect-error", "connect_failed");
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

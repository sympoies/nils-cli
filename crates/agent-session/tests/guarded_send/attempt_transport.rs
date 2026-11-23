//! The CLI exchanges real WebSocket frames with a disposable provider fixture.
use super::{Fixture, native::Server};
use crate::attempt::{query, status};
use pretty_assertions::assert_eq;
use std::fs;

#[test]
fn attempt_delayed_provider_readback_is_correlated_and_read_only() {
    for (notice, key) in [(false, "synthetic-attempt"), (true, "notice-once")] {
        let fixture = Fixture::new("codex");
        let _capability = fixture.bind_codex();
        let server = Server::start(&fixture, "attempt-late-input");
        if notice {
            fixture.refused(&fixture.notice(&[]), "notice-native-unconfirmed");
        } else {
            fixture.refused(
                &fixture.send(&[
                    "--require-idle",
                    "--require-empty-composer",
                    "--idempotency-key",
                    key,
                ]),
                "send-native-unconfirmed",
            );
        }
        assert_eq!(status(&fixture, key)["outcome"], "unknown");
        assert_eq!(status(&fixture, "different-key")["outcome"], "unknown");
        fs::write(fixture.root().join("publish-attempt"), "").unwrap();
        assert_eq!(status(&fixture, key)["outcome"], "submitted");
        assert_ne!(query(&fixture, "recover", key).code, 0);
        assert_eq!(server.mutations().len(), 1);
        fixture.no_delivery();
    }
}

//! Process-local reuse of a successful actor verification.
//!
//! An entry maps a one-way digest of the credential and the verified identity
//! to the verified actor. The credential itself is never stored. Only a success
//! is cached; a refusal or mismatch leaves no entry. Entries expire after
//! [`TTL`] and are dropped when a managed call reports a rejected credential.
use super::{Authorization, probe};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::process::Output;
use std::sync::{LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const TTL: Duration = Duration::from_secs(10 * 60);

pub(super) type Key = [u8; 32];

/// How the actor of an authorization was verified, for the identity audit.
#[derive(Serialize)]
pub(super) struct ActorVerification {
    pub cached: bool,
    pub verified_at: String,
}

struct Entry {
    actor: String,
    verified_at: String,
    stored: Instant,
}

static CACHE: LazyLock<Mutex<BTreeMap<Key, Entry>>> = LazyLock::new(Default::default);
static VERIFYING: Mutex<()> = Mutex::new(());

/// Held while a missing verification is probed, so concurrent managed calls in one
/// process wait for it and reuse the result instead of each probing the actor.
pub(super) fn single_flight() -> MutexGuard<'static, ()> {
    VERIFYING.lock().unwrap_or_else(|e| e.into_inner())
}

/// Digest of everything the verification depends on. A human profile is bound to
/// its expected login; an App profile also to its id, slug and (when repo-scoped)
/// the installation repository it was checked against. The audit state root is
/// included so a cached-verification note only refers to a verification recorded
/// in the same audit log.
pub(super) fn key(auth: &Authorization) -> Key {
    let profile = &auth.selection.profile;
    let target = &auth.selection.target;
    let state = super::state_home().unwrap_or_default();
    let mut digest = Sha256::new();
    for part in [
        "forge-identity-actor-verification-v1",
        &auth.token,
        &state.to_string_lossy(),
        &target.host,
        profile.expected_login.as_deref().unwrap_or(""),
        profile.app_slug.as_deref().unwrap_or(""),
        &profile
            .expected_app_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        if profile.app_slug.is_some() {
            &target.repo
        } else {
            ""
        },
    ] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    digest.finalize().into()
}

fn lookup_at(key: &Key, now: Instant) -> Option<(String, String)> {
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, entry| now.saturating_duration_since(entry.stored) < TTL);
    cache
        .get(key)
        .map(|entry| (entry.actor.clone(), entry.verified_at.clone()))
}

fn store_at(key: Key, actor: &str, verified_at: &str, now: Instant) {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).insert(
        key,
        Entry {
            actor: actor.to_owned(),
            verified_at: verified_at.to_owned(),
            stored: now,
        },
    );
}

/// The verified actor and its verification time while the entry is fresh.
pub(super) fn lookup(auth: &Authorization) -> Option<(String, ActorVerification)> {
    lookup_at(&key(auth), probe::now()).map(|(actor, verified_at)| {
        (
            actor,
            ActorVerification {
                cached: true,
                verified_at,
            },
        )
    })
}

/// Remember a successful verification and report it as a fresh probe.
pub(super) fn store(auth: &Authorization) -> ActorVerification {
    let verified_at = jiff::Timestamp::now().to_string();
    store_at(key(auth), &auth.actor, &verified_at, probe::now());
    ActorVerification {
        cached: false,
        verified_at,
    }
}

impl Authorization {
    /// Drop the cached verification when a managed call shows the provider rejected
    /// this credential (HTTP 401/403), so the next call verifies the actor again.
    pub fn observe_output(&self, output: &Output) {
        if output.status.success() || !(rejected(&output.stderr) || rejected(&output.stdout)) {
            return;
        }
        CACHE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key(self));
    }
}

/// `gh` reports a rejection as `(HTTP 401)`, `HTTP 403: ...` or an `HTTP/2 403` status line.
fn rejected(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes);
    ["401", "403"].iter().any(|code| {
        text.contains(&format!("HTTP {code}"))
            || text
                .lines()
                .any(|line| line.starts_with("HTTP/") && line.split(' ').nth(1) == Some(code))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn an_entry_expires_after_the_ttl() {
        let key = [7; 32];
        let start = probe::now();
        store_at(key, "account-a", "2026-01-01T00:00:00Z", start);
        assert_eq!(
            lookup_at(&key, start + TTL - Duration::from_secs(1)),
            Some(("account-a".to_owned(), "2026-01-01T00:00:00Z".to_owned()))
        );
        assert_eq!(lookup_at(&key, start + TTL), None);
    }

    fn auth(
        token: &str,
        host: &str,
        repo: &str,
        login: Option<&str>,
        app: Option<(&str, u64)>,
    ) -> Authorization {
        use super::super::{Operation, Profile, Selection, Target};
        Authorization {
            selection: Selection {
                principal: "fixture".to_owned(),
                target: Target::new(host, repo).unwrap(),
                operation: Operation::ApiRead,
                profile_id: "fixture".to_owned(),
                matched_rule: "fixture".to_owned(),
                profile: Profile {
                    expected_login: login.map(str::to_owned),
                    expected_app_id: app.map(|(_, id)| id),
                    app_slug: app.map(|(slug, _)| slug.to_owned()),
                    credential: "fixture".to_owned(),
                    commit_name: "Fixture".to_owned(),
                    commit_email: "fixture@example.invalid".to_owned(),
                    signing_fingerprint: "fixture".to_owned(),
                    operations: vec![Operation::ApiRead],
                },
                session_binding: None,
            },
            actor: login.unwrap_or("fixture-app[bot]").to_owned(),
            token: token.to_owned(),
            policy_digest: "fixture".to_owned(),
        }
    }

    fn reused(stored: &Authorization, other: &Authorization) -> bool {
        store(stored);
        lookup(other).is_some()
    }

    #[test]
    fn a_verification_is_reused_only_for_the_same_credential_host_and_login() {
        let _lock = nils_test_support::GlobalStateLock::new();
        let base = auth("unit-human-token", "github.com", "o/r", Some("alice"), None);
        assert!(reused(&base, &base));
        for different in [
            auth(
                "unit-human-token-2",
                "github.com",
                "o/r",
                Some("alice"),
                None,
            ),
            auth(
                "unit-human-token",
                "ghe.example.com",
                "o/r",
                Some("alice"),
                None,
            ),
            auth("unit-human-token", "github.com", "o/r", Some("bob"), None),
        ] {
            assert!(!reused(&base, &different));
        }
        // A human credential is not repository scoped.
        assert!(reused(
            &base,
            &auth(
                "unit-human-token",
                "github.com",
                "o/other",
                Some("alice"),
                None
            )
        ));
    }

    #[test]
    fn an_app_verification_is_bound_to_its_slug_id_and_checked_repository() {
        let _lock = nils_test_support::GlobalStateLock::new();
        let base = auth(
            "unit-app-token",
            "github.com",
            "o/r",
            None,
            Some(("app-a", 1)),
        );
        assert!(reused(&base, &base));
        for different in [
            auth(
                "unit-app-token",
                "github.com",
                "o/r",
                None,
                Some(("app-b", 1)),
            ),
            auth(
                "unit-app-token",
                "github.com",
                "o/r",
                None,
                Some(("app-a", 2)),
            ),
            auth(
                "unit-app-token",
                "github.com",
                "o/other",
                None,
                Some(("app-a", 1)),
            ),
        ] {
            assert!(!reused(&base, &different));
        }
    }

    #[test]
    fn only_a_provider_rejection_counts_as_rejected() {
        assert!(rejected(b"gh: Bad credentials (HTTP 401)"));
        assert!(rejected(
            b"HTTP 403: Resource not accessible (https://api.github.com/graphql)"
        ));
        assert!(rejected(b"HTTP/2.0 403 Forbidden\r\n\r\n"));
        assert!(!rejected(b"gh: Not Found (HTTP 404)"));
        assert!(!rejected(b"build 4013 finished"));
    }
}

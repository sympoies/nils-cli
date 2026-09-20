//! Tests whose subject is the shared fixture code in `support.rs` rather than
//! a `claude-cli` command.

use crate::support::*;
use pretty_assertions::assert_eq;
use std::fs::OpenOptions;
use std::os::fd::AsRawFd;
use std::thread;
use std::time::{Duration, Instant};

/// Pins the containment defaults, asserting the **effective** value rather than
/// mere presence.
///
/// `run_impl_os` replays `envs` in order through `Command::env`, whose map is
/// last-write-wins per key, and applies `envs_os` after `envs`. So a later entry
/// for the same key decides what the child sees. An `any()` assertion would stay
/// green if a future edit appended a live endpoint after the pin — exactly the
/// regression this guards. `with_path_prepend` in `nils-test-support` reads the
/// effective value with `rev().find(..)` for the same reason.
#[test]
fn base_options_pins_containment_defaults_as_the_effective_values() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let options = base_options(tmp.path());
    let missing_claude = path_str(&tmp.path().join("missing-claude"));

    let effective = |key: &str| {
        assert!(
            !options.envs_os.iter().any(|(name, _)| name == key),
            "{key} must not also be set through envs_os, which is applied last"
        );
        options
            .envs
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    };

    assert_eq!(
        effective("CLAUDE_PROMPT_SEGMENT_ENDPOINT"),
        Some(UNROUTABLE_ENDPOINT),
        "base_options must pin the endpoint: it clears the CLAUDE_PROMPT prefix, and the \
         production default is a real host, so omission means reaching it; envs={:?}",
        options.envs
    );
    assert_eq!(
        effective("CLAUDE_PROMPT_SEGMENT_MAX_TIME_SECONDS"),
        Some(FAST_FAIL_MAX_TIME_SECONDS),
        "the fast-fail must be bounded so the pinned endpoint fails fast on a host that \
         drops rather than refuses loopback traffic; envs={:?}",
        options.envs
    );
    assert_eq!(effective("NO_PROXY"), Some("127.0.0.1,localhost"));
    assert_eq!(effective("no_proxy"), Some("127.0.0.1,localhost"));
    assert_eq!(effective("CLAUDE_CLI_BIN"), Some(missing_claude.as_str()));
    assert_eq!(
        effective("CLAUDE_PROMPT_SEGMENT_CLAUDE_BIN"),
        Some(missing_claude.as_str())
    );
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        assert!(
            options.env_remove.iter().any(|removed| removed == key),
            "base_options must remove ambient {key}; removed={:?}",
            options.env_remove
        );
    }
}

/// Pins the predicate that keeps the background-refresh fixtures leak-free.
///
/// Asserted against a synthetic lock rather than a real refresh: the real race
/// is won by the child on an idle host, so a test that waits for an actual
/// detached refresh passes with or without the fix and pins nothing.
#[test]
fn settled_wait_blocks_until_the_refresh_lock_is_released() {
    const HOLD: Duration = Duration::from_millis(300);

    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), "settled-body");
    let held = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(refresh_lock_path(&cache_file))
        .expect("hold refresh lock");
    // SAFETY: `flock` observes the valid descriptor owned by `held`.
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "test could not take the refresh lock"
    );

    // Measured *inside* the scope: `thread::scope` joins the releasing thread on
    // the way out, so an elapsed time taken after the scope would satisfy this
    // assertion even if the wait returned immediately.
    let started = Instant::now();
    let waited = thread::scope(|scope| {
        let held = &held;
        scope.spawn(move || {
            thread::sleep(HOLD);
            // SAFETY: `flock` observes the descriptor still owned by `held`.
            unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_UN) };
        });
        assert!(
            wait_for_background_refresh_settled(
                &cache_file,
                "settled-body",
                Duration::from_secs(5)
            ),
            "settled wait gave up while the lock was still held"
        );
        started.elapsed()
    });

    assert!(
        waited >= HOLD,
        "settled wait returned after {waited:?}, before the lock was released"
    );
}

/// A child that never releases must surface as a failure, not a silent pass.
#[test]
fn settled_wait_times_out_when_the_refresh_lock_is_never_released() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let cache_file = write_cache(tmp.path(), "stuck-body");
    let held = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(refresh_lock_path(&cache_file))
        .expect("hold refresh lock");
    // SAFETY: `flock` observes the valid descriptor owned by `held`.
    assert_eq!(
        unsafe { libc::flock(held.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0,
        "test could not take the refresh lock"
    );

    assert!(
        !wait_for_background_refresh_settled(&cache_file, "stuck-body", Duration::from_millis(200)),
        "settled wait reported success while the lock was still held"
    );
}

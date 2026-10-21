//! Conditions a serve test waits on instead of sleeping.
use super::super::ServeState;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::{Duration, Instant};

/// Waits until the provider-history scan started by session `id`'s attach
/// subscription ran to completion, before the test writes the transcript or
/// identity it expects to recover. An earlier write lets the scan find the file
/// and baseline at its end, so the first prompt would be missed.
pub(super) async fn wait_for_pending_scan(state: &ServeState, id: &str) {
    let discovery = &state.provider_prompt_discovery;
    discovery
        .wait_for_finished_scan_of(&state.context, id)
        .await;
}

/// Waits until a request is blocked acquiring the record lock of `id`.
///
/// Linux lists every parked `flock` waiter in `/proc/locks`, so the wait ends on
/// the condition itself. Other hosts expose no waiter list; the wait then
/// settles briefly instead, which can only make the caller's assertions weaker,
/// never flaky.
pub(super) async fn wait_for_record_lock_waiter(state_dir: &Path, id: &str) {
    let lock_path = state_dir
        .join(crate::SESSION_LOCKS_DIR)
        .join(format!("{id}.lock"));
    let inode = std::fs::metadata(&lock_path)
        .expect("the test holds the record lock file")
        .ino();
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match std::fs::read_to_string("/proc/locks") {
            Ok(locks) if has_waiter(&locks, inode) => return,
            Ok(_) => {}
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(250)).await;
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "no request parked on the record lock of {id}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Whether `/proc/locks` lists a blocked waiter (`->`) on `inode`. The device
/// field is `major:minor:inode`, with the inode in decimal.
fn has_waiter(locks: &str, inode: u64) -> bool {
    locks.lines().any(|line| {
        let mut fields = line.split_whitespace();
        fields.clone().any(|field| field == "->") && fields.any(|field| {
            let mut parts = field.split(':');
            matches!(
                (parts.next(), parts.next(), parts.next(), parts.next()),
                (Some(_), Some(_), Some(found), None) if found.parse::<u64>().ok() == Some(inode)
            )
        })
    })
}

#[test]
fn only_a_blocked_waiter_on_the_inode_counts() {
    let locks = "\
1: FLOCK  ADVISORY  WRITE 4242 08:01:5512 0 EOF
1: -> FLOCK  ADVISORY  WRITE 4243 08:01:5512 0 EOF
2: -> FLOCK  ADVISORY  WRITE 4244 08:01:777 0 EOF
3: POSIX  ADVISORY  WRITE 4245 08:01:888 0 EOF
";
    assert!(has_waiter(locks, 5512));
    assert!(has_waiter(locks, 777));
    assert!(!has_waiter(locks, 888), "a holder is not a waiter");
    assert!(!has_waiter(locks, 551), "inode must match exactly");
    assert!(!has_waiter("", 5512));
}

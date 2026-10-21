mod adoption_deadline;
mod adoption_recovery;
mod adoption_transaction;
mod challenge_descriptor;
mod checkout_recovery;
mod error_contract;
mod git_output_budget;
mod owned_cleanup;
mod pending_recovery;
mod proc_entry;
mod process_cleanup;
mod process_scan;
mod process_supervisor;
mod process_tree_tracking;
mod revocation;
mod snapshot_worker;
mod supervisor_lifecycle;
mod supervisor_status;
mod user_namespace;
mod wire_contract;

use super::*;

use std::os::fd::{IntoRawFd, OwnedFd};

#[cfg(target_os = "macos")]
use std::os::unix::net::UnixListener;

use std::os::unix::net::UnixStream;

use std::os::unix::process::ExitStatusExt;

/// Bound for waits and budgets that end as soon as their condition holds, so a
/// loaded host cannot turn a slow child start into a failure. Deadlines whose
/// expiry is the behavior under test do not use it.
const TEST_WAIT: Duration = Duration::from_secs(20);

fn successful_worker_output(snapshot: serde_json::Value) -> std::process::Output {
    std::process::Output {
        status: std::process::ExitStatus::from_raw(0),
        stdout: serde_json::to_vec(&serde_json::json!({
            "schema": SNAPSHOT_WORKER_SCHEMA,
            "snapshot": snapshot,
            "error": null,
        }))
        .expect("serialize worker response"),
        stderr: Vec::new(),
    }
}

fn true_executable() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "/usr/bin/true"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "/bin/true"
    }
}

fn test_git(checkout: &Path, args: &[&str]) {
    let status = Command::new("/usr/bin/git")
        .args(args)
        .current_dir(checkout)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .expect("run test Git");
    assert!(status.success(), "test Git failed: {args:?}");
}

fn adoption_transaction_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    PathBuf,
    DirtySnapshot,
    PathBuf,
) {
    let checkout = tempfile::TempDir::new().expect("adoption checkout");
    test_git(checkout.path(), &["init", "-q"]);
    test_git(checkout.path(), &["config", "user.name", "Test User"]);
    test_git(
        checkout.path(),
        &["config", "user.email", "test@example.com"],
    );
    fs::write(checkout.path().join("tracked.txt"), b"base\n").expect("write tracked file");
    test_git(checkout.path(), &["add", "tracked.txt"]);
    test_git(checkout.path(), &["commit", "-qm", "base"]);
    fs::write(checkout.path().join("dirty.txt"), b"initial dirty state\n")
        .expect("write dirty file");
    let snapshot = dirty_snapshot(checkout.path()).expect("snapshot adoption fixture");

    let state_root = tempfile::TempDir::new().expect("adoption state root");
    fs::set_permissions(state_root.path(), fs::Permissions::from_mode(0o700))
        .expect("make state root private");
    let checkout_dir = state_root
        .path()
        .join(&snapshot.repository_key)
        .join(&snapshot.checkout_key);
    private_directory(checkout_dir.parent().expect("repository state directory"))
        .expect("create repository state directory");
    private_directory(&checkout_dir).expect("create checkout state directory");
    let challenge_dir = checkout_dir.join("challenges");
    private_directory(&challenge_dir).expect("create challenge directory");

    let token = "a".repeat(64);
    let challenge_digest = sha256_hex(token.as_bytes());
    let now = unix_time().expect("challenge time");
    let challenge = ChallengeRecord {
        schema: CHALLENGE_SCHEMA.to_string(),
        token_digest: challenge_digest.clone(),
        session_key: "b".repeat(64),
        repository_key: snapshot.repository_key.clone(),
        checkout_key: snapshot.checkout_key.clone(),
        checkout_instance: snapshot.checkout_instance.clone(),
        snapshot_id: snapshot.snapshot_id.clone(),
        head_oid: snapshot.head_oid.clone(),
        branch_ref_digest: snapshot.branch_ref_digest.clone(),
        authorization_turn_digest: "c".repeat(64),
        issued_at: now,
        expires_at: now + 300,
    };
    let challenge_path = challenge_dir.join(format!("{challenge_digest}.json"));
    write_json_atomic(&challenge_path, &challenge, false).expect("write challenge");
    let reason_path = state_root.path().join("reason.txt");
    fs::write(&reason_path, b"Preserve user-owned dirty state.\n").expect("write reason");
    fs::set_permissions(&reason_path, fs::Permissions::from_mode(0o600))
        .expect("make reason private");

    (checkout, state_root, reason_path, snapshot, challenge_path)
}

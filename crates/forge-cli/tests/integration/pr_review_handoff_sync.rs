//! A designated PASS carried across a base-sync merge, proven against real git
//! fixtures. The provider is stubbed; every commit, tree and ancestry fact is a
//! real object in a throwaway repository.
use std::fs;
use std::path::Path;
use std::process::Command;

use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::{self, ReviewStatePayload, ReviewStateRecord};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use tempfile::TempDir;

use super::pr_review_handoff::{assert_refusal, fixture, review, writable_ledger};
use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli_in};

const REVIEW_URL: &str = "https://github.com/acme/widgets/pull/7#pullrequestreview-1";

struct Repo {
    dir: TempDir,
}

impl Repo {
    fn path(&self) -> &Path {
        self.dir.path()
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }

    fn commit(&self, file: &str, content: &str) -> String {
        fs::write(self.path().join(file), content).unwrap();
        self.git(&["add", file]);
        self.git(&["commit", "-q", "-m", file]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn tree(&self, commit: &str) -> String {
        self.git(&["rev-parse", &format!("{commit}^{{tree}}")])
    }
}

/// `base` is the assigned base, `reviewed` the designated PASS head, `main`
/// the advanced target branch tip, and `synced` a clean `git merge main`.
struct Sync {
    repo: Repo,
    base: String,
    reviewed: String,
    main: String,
    synced: String,
}

fn sync(conflicting: bool) -> Sync {
    let repo = Repo {
        dir: TempDir::new().unwrap(),
    };
    repo.git(&["init", "-q", "-b", "main"]);
    let base = repo.commit("shared.txt", "one\ntwo\nthree\n");
    repo.git(&["checkout", "-q", "-b", "feat"]);
    let reviewed = if conflicting {
        repo.commit("shared.txt", "one\nfeature\nthree\n")
    } else {
        repo.commit("feature.txt", "feature\n")
    };
    repo.git(&["checkout", "-q", "main"]);
    let main = repo.commit("shared.txt", "one\nmain\nthree\n");
    repo.git(&["checkout", "-q", "feat"]);
    let synced = if conflicting {
        let output = Command::new("git")
            .args(["merge", "-q", "--no-edit", "main"])
            .current_dir(repo.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(!output.status.success(), "fixture merge must conflict");
        // A hand resolution: the reviewer never saw this content.
        repo.commit("shared.txt", "one\nfeature and main\nthree\n")
    } else {
        repo.git(&["merge", "-q", "--no-edit", "main"]);
        repo.git(&["rev-parse", "HEAD"])
    };
    Sync {
        repo,
        base,
        reviewed,
        main,
        synced,
    }
}

fn assigned(base: &str, reviewed: &str) -> Vec<ReviewStateRecord> {
    let handoff = ReviewHandoff {
        coordinator_digest: review_state::sha256_digest(b"worker-session"),
        reviewer_digest: review_state::sha256_digest(b"reviewer-session"),
        review_author: "review-app[bot]".into(),
        base_sha: base.into(),
        assigned_head: reviewed.into(),
        returned_reason: None,
        assignment_generation: 1,
        surrendered: false,
        coordinator_transfer: None,
    };
    let first = ReviewStateRecord::new(
        "acme/widgets",
        7,
        reviewed,
        0,
        None,
        ReviewStatePayload::ReviewHandoff { handoff },
    )
    .unwrap();
    let state = review_state::observe_review_loop(None, reviewed, &[])
        .unwrap()
        .state;
    let second = ReviewStateRecord::new(
        "acme/widgets",
        7,
        reviewed,
        1,
        Some(first.record_digest.clone()),
        ReviewStatePayload::ReviewLoop { state },
    )
    .unwrap()
    .with_assignment_generation(Some(1))
    .unwrap();
    vec![first, second]
}

/// The provider reports `head` with the assigned PASS published at `reviewed`.
fn provider(s: &Sync, head: &str, reviews: Vec<Value>) -> StubEnv {
    let seeded = assigned(&s.base, &s.reviewed);
    writable_ledger(fixture(head, &seeded, reviews), &seeded).env("PROVIDER_BASE", &s.main)
}

fn handoff_cmd(stub: &StubEnv, dir: &Path, dry_run: bool, op: &str, head: &str) -> CmdOutput {
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    if dry_run {
        args.push("--dry-run");
    }
    args.extend(["pr", "review-handoff", op, "7", "--expected-head", head]);
    run_forge_cli_in(stub, &args, Some(dir))
}

fn merge_preview(stub: &StubEnv, dir: &Path) -> CmdOutput {
    run_forge_cli_in(
        stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "--dry-run",
            "pr",
            "merge",
            "7",
        ],
        Some(dir),
    )
}

fn ledger_bodies(stub: &StubEnv) -> Vec<String> {
    serde_json::from_str(&fs::read_to_string(stub.tempdir.path().join("ledger.json")).unwrap())
        .unwrap()
}

fn assert_not_carried(output: &CmdOutput, stub: &StubEnv, reason: &str) {
    assert_refusal(output, 65, "awaiting_designated_review");
    assert!(
        output.stdout.contains(&format!("sync_carry_over={reason}")),
        "{}",
        output.stdout
    );
    assert_eq!(
        ledger_bodies(stub).len(),
        2,
        "no carry-over may be recorded"
    );
}

#[test]
fn clean_base_sync_carries_the_pass_and_records_it_once() {
    let s = sync(false);
    let dir = s.repo.path();
    let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass")]);
    let evidence = json!({
        "review_url": REVIEW_URL,
        "reviewed_head": s.reviewed,
        "head": s.synced,
        "base_commit": s.main,
        "merge_tree": s.repo.tree(&s.synced),
    });

    // The merge gate admits a carried PASS only once it is in the ledger.
    let gate = merge_preview(&stub, dir);
    assert_refusal(&gate, 65, "awaiting_designated_review");
    assert!(
        gate.stdout.contains("review-handoff check"),
        "{}",
        gate.stdout
    );

    let preview = handoff_cmd(&stub, dir, true, "check", &s.synced);
    assert_eq!(preview.code, 0, "{} {}", preview.stdout, preview.stderr);
    assert_eq!(
        parse_envelope(&preview.stdout)["data"]["sync_carry_over"],
        evidence
    );
    assert_eq!(ledger_bodies(&stub).len(), 2, "dry-run never records");

    let checked = handoff_cmd(&stub, dir, false, "check", &s.synced);
    assert_eq!(checked.code, 0, "{} {}", checked.stdout, checked.stderr);
    let data = &parse_envelope(&checked.stdout)["data"];
    assert_eq!(data["status"], "reviewed");
    assert_eq!(data["sync_carry_over"], evidence);
    let bodies = ledger_bodies(&stub);
    assert_eq!(bodies.len(), 3);
    assert!(
        bodies[2].contains("review-sync-carry-over"),
        "{}",
        bodies[2]
    );
    assert!(bodies[2].contains(&s.repo.tree(&s.synced)), "{}", bodies[2]);

    let again = handoff_cmd(&stub, dir, false, "check", &s.synced);
    assert_eq!(again.code, 0, "{} {}", again.stdout, again.stderr);
    assert_eq!(
        ledger_bodies(&stub).len(),
        3,
        "an identical carry-over is recorded once"
    );

    let gate = merge_preview(&stub, dir);
    assert_eq!(gate.code, 0, "{} {}", gate.stdout, gate.stderr);

    let inspected = handoff_cmd(&stub, dir, false, "inspect", &s.synced);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    assert_eq!(
        parse_envelope(&inspected.stdout)["data"]["sync_carry_over"],
        evidence
    );
}

#[test]
fn conflicted_sync_requires_re_review() {
    let s = sync(true);
    let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass")]);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_not_carried(&output, &stub, "merge-conflicted");
}

#[test]
fn extra_commit_after_the_sync_requires_re_review() {
    let s = sync(false);
    let extra = s.repo.commit("feature.txt", "feature and more\n");
    let stub = provider(&s, &extra, vec![review(&s.reviewed, "pass")]);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &extra);
    assert_not_carried(&output, &stub, "not-a-sync-merge");
}

#[test]
fn swapped_or_extra_parents_are_not_a_sync_merge() {
    let s = sync(false);
    let tree = s.repo.tree(&s.synced);
    let swapped = s.repo.git(&[
        "commit-tree",
        &tree,
        "-p",
        &s.main,
        "-p",
        &s.reviewed,
        "-m",
        "swapped",
    ]);
    let octopus = s.repo.git(&[
        "commit-tree",
        &tree,
        "-p",
        &s.reviewed,
        "-p",
        &s.main,
        "-p",
        &s.base,
        "-m",
        "octopus",
    ]);
    for head in [swapped, octopus] {
        let stub = provider(&s, &head, vec![review(&s.reviewed, "pass")]);
        let output = handoff_cmd(&stub, s.repo.path(), false, "check", &head);
        assert_not_carried(&output, &stub, "not-a-sync-merge");
    }
}

#[test]
fn forged_merge_tree_requires_re_review() {
    let s = sync(false);
    fs::write(s.repo.path().join("smuggled.txt"), "unreviewed\n").unwrap();
    s.repo.git(&["add", "smuggled.txt"]);
    let forged_tree = s.repo.git(&["write-tree"]);
    let forged = s.repo.git(&[
        "commit-tree",
        &forged_tree,
        "-p",
        &s.reviewed,
        "-p",
        &s.main,
        "-m",
        "Merge branch 'main' into feat",
    ]);
    let stub = provider(&s, &forged, vec![review(&s.reviewed, "pass")]);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &forged);
    assert_not_carried(&output, &stub, "tree-mismatch");
}

#[test]
fn base_parent_off_the_target_branch_requires_re_review() {
    let s = sync(false);
    s.repo.git(&["checkout", "-q", "-b", "side", &s.base]);
    s.repo.commit("side.txt", "side\n");
    s.repo.git(&["checkout", "-q", "--detach", &s.reviewed]);
    s.repo.git(&["merge", "-q", "--no-edit", "side"]);
    let head = s.repo.git(&["rev-parse", "HEAD"]);
    let stub = provider(&s, &head, vec![review(&s.reviewed, "pass")]);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &head);
    assert_not_carried(&output, &stub, "base-not-on-target");
}

#[test]
fn retargeted_pull_request_cannot_carry_the_assigned_scope() {
    // Assigned against a release branch, then retargeted to main and synced:
    // the clean merge would bring main's unreviewed history into scope.
    let s = sync(false);
    s.repo.git(&["checkout", "-q", "-b", "release", &s.base]);
    let release = s.repo.commit("release.txt", "release\n");
    let seeded = assigned(&release, &s.reviewed);
    let stub = writable_ledger(
        fixture(&s.synced, &seeded, vec![review(&s.reviewed, "pass")]),
        &seeded,
    )
    .env("PROVIDER_BASE", &s.main);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_not_carried(&output, &stub, "assigned-base-not-on-target");
}

#[test]
fn unavailable_commit_objects_fail_closed() {
    let s = sync(false);
    let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass")]);
    let elsewhere = TempDir::new().unwrap();
    let output = handoff_cmd(&stub, elsewhere.path(), false, "check", &s.synced);
    assert_not_carried(&output, &stub, "head-unavailable");
}

#[test]
fn newer_blocked_report_on_the_synced_head_supersedes_the_carried_pass() {
    let s = sync(false);
    let mut blocked = review(&s.synced, "blocked");
    blocked["databaseId"] = json!(2);
    blocked["url"] = json!("https://github.com/acme/widgets/pull/7#pullrequestreview-2");
    blocked["submittedAt"] = json!("2026-07-20T12:00:05Z");
    let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass"), blocked]);
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_refusal(&output, 65, "awaiting_designated_review");
    assert_eq!(ledger_bodies(&stub).len(), 2);
}

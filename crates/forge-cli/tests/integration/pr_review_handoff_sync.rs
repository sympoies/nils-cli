//! A designated PASS carried across a base-sync merge, proven against real git
//! fixtures. The provider is stubbed; every commit, tree and ancestry fact is a
//! real object in a throwaway repository.
use std::fs;
use std::path::Path;
use std::process::Command;

use forge_cli::ops::pr_review_handoff::ReviewHandoff;
use forge_cli::ops::review_state::{self, ReviewStatePayload, ReviewStateRecord};
use forge_cli::ops::review_sync_carry_over::SyncCarryOver;
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

    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new("git")
            .args(args)
            .current_dir(self.path())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Fixture")
            .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
            .env("GIT_COMMITTER_NAME", "Fixture")
            .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
            .output()
            .expect("spawn git")
    }

    fn git(&self, args: &[&str]) -> String {
        let output = self.run(args);
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
        let output = repo.run(&["merge", "-q", "--no-edit", "main"]);
        assert!(!output.status.success(), "fixture merge must conflict");
        // The resolution must conclude this merge, so it has both parents.
        repo.git(&["rev-parse", "--verify", "MERGE_HEAD"]);
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
    scoped(base, reviewed, Some("main"))
}

/// An assignment, optionally bound to the provider base branch name.
fn scoped(base: &str, reviewed: &str, base_ref: Option<&str>) -> Vec<ReviewStateRecord> {
    let mut handoff = json!({
        "coordinator_digest": review_state::sha256_digest(b"worker-session"),
        "reviewer_digest": review_state::sha256_digest(b"reviewer-session"),
        "review_author": "review-app[bot]",
        "base_sha": base,
        "assigned_head": reviewed,
        "returned_reason": null,
        "assignment_generation": 1,
        "surrendered": false,
    });
    if let Some(base_ref) = base_ref {
        handoff["base_ref"] = json!(base_ref);
    }
    let handoff: ReviewHandoff = serde_json::from_value(handoff).unwrap();
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
fn coordinator_with_active_assignment_records_clean_base_sync_once() {
    let s = sync(false);
    let dir = s.repo.path();
    let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass")])
        .env("AGENT_SESSION_ID", "worker-session");
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
    let record = review_state::parse_state_marker(&bodies[2])
        .unwrap()
        .expect("carry-over record");
    assert_eq!(record.expected_head, s.synced);
    let ReviewStatePayload::ReviewSyncCarryOver { carry_over } = record.payload else {
        panic!("unexpected payload: {}", bodies[2]);
    };
    assert_eq!(serde_json::to_value(carry_over).unwrap(), evidence);

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
fn carried_native_approval_is_source_pass_until_current_head_is_approved() {
    let s = sync(false);
    let mut report = review(&s.reviewed, "pass");
    report["state"] = json!("APPROVED");
    let stub = provider(&s, &s.synced, vec![report]);
    let preview = handoff_cmd(&stub, s.repo.path(), true, "check", &s.synced);
    assert_eq!(preview.code, 0, "{} {}", preview.stdout, preview.stderr);
    let data = &parse_envelope(&preview.stdout)["data"];
    assert_eq!(data["approval"]["state"], "source-passed");
    assert_eq!(data["approval"]["native_state"], "APPROVED");
    assert_eq!(data["approval"]["reviewed_head"], s.reviewed);
    let strict = run_forge_cli_in(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "check",
            "7",
            "--expected-head",
            &s.synced,
            "--approval-policy",
            "final-approval",
        ],
        Some(s.repo.path()),
    );
    assert_refusal(&strict, 65, "designated_final_approval_required");
    assert_eq!(
        ledger_bodies(&stub).len(),
        2,
        "a refused check must not append carry-over"
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

#[test]
fn checkout_merge_attributes_cannot_hide_a_sync_conflict() {
    // A union merge driver from the reviewed tree or the caller's
    // info/attributes turns a real content conflict into a "clean" sync.
    for committed in [true, false] {
        let repo = Repo {
            dir: TempDir::new().unwrap(),
        };
        repo.git(&["init", "-q", "-b", "main"]);
        let base = repo.commit("shared.txt", "one\ntwo\nthree\n");
        repo.git(&["checkout", "-q", "-b", "feat"]);
        let attributes = if committed {
            repo.path().join(".gitattributes")
        } else {
            repo.path().join(".git/info/attributes")
        };
        fs::create_dir_all(attributes.parent().unwrap()).unwrap();
        fs::write(&attributes, "shared.txt merge=union\n").unwrap();
        if committed {
            repo.git(&["add", ".gitattributes"]);
        }
        let reviewed = repo.commit("shared.txt", "one\nfeature\nthree\n");
        repo.git(&["checkout", "-q", "main"]);
        let main = repo.commit("shared.txt", "one\nmain\nthree\n");
        repo.git(&["checkout", "-q", "feat"]);
        repo.git(&["merge", "-q", "--no-edit", "main"]);
        let synced = repo.git(&["rev-parse", "HEAD"]);
        let s = Sync {
            repo,
            base,
            reviewed,
            main,
            synced,
        };
        let stub = provider(&s, &s.synced, vec![review(&s.reviewed, "pass")]);
        let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
        assert_not_carried(&output, &stub, "merge-conflicted");
    }
}

#[test]
fn retarget_to_a_descendant_base_requires_the_recorded_base_ref() {
    // A retarget to a branch descending from the assigned base keeps every
    // SHA ancestry fact; only the recorded base branch name exposes it.
    let s = sync(false);
    let seeded = scoped(&s.base, &s.reviewed, None);
    let stub = writable_ledger(
        fixture(&s.synced, &seeded, vec![review(&s.reviewed, "pass")]),
        &seeded,
    )
    .env("PROVIDER_BASE", &s.main)
    .env("PROVIDER_BASE_REF", "release");
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_not_carried(&output, &stub, "base-ref-unrecorded");

    let seeded = scoped(&s.base, &s.reviewed, Some("main"));
    let stub = writable_ledger(
        fixture(&s.synced, &seeded, vec![review(&s.reviewed, "pass")]),
        &seeded,
    )
    .env("PROVIDER_BASE", &s.main)
    .env("PROVIDER_BASE_REF", "release");
    let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_not_carried(&output, &stub, "base-ref-changed");
}

#[test]
fn a_recorded_carry_over_admits_only_when_it_matches_the_recomputation() {
    let s = sync(false);
    let mut seeded = assigned(&s.base, &s.reviewed);
    let forged = SyncCarryOver {
        review_url: REVIEW_URL.into(),
        reviewed_head: s.reviewed.clone(),
        head: s.synced.clone(),
        base_commit: s.main.clone(),
        merge_tree: s.repo.tree(&s.reviewed),
    };
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            &s.synced,
            2,
            Some(seeded[1].record_digest.clone()),
            ReviewStatePayload::ReviewSyncCarryOver { carry_over: forged },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let stub = writable_ledger(
        fixture(&s.synced, &seeded, vec![review(&s.reviewed, "pass")]),
        &seeded,
    )
    .env("PROVIDER_BASE", &s.main);
    let gate = merge_preview(&stub, s.repo.path());
    assert_refusal(&gate, 65, "awaiting_designated_review");
    assert!(
        gate.stdout.contains("review-handoff check"),
        "{}",
        gate.stdout
    );
}

#[test]
fn ledger_rejects_a_carry_over_for_another_head_or_without_an_owner() {
    let s = sync(false);
    let seeded = assigned(&s.base, &s.reviewed);
    let carry_over = SyncCarryOver {
        review_url: REVIEW_URL.into(),
        reviewed_head: s.reviewed.clone(),
        head: s.synced.clone(),
        base_commit: s.main.clone(),
        merge_tree: s.repo.tree(&s.synced),
    };
    let record = |head: &str, previous: Option<&ReviewStateRecord>, generation| {
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            head,
            previous.map_or(0, |_| 2),
            previous.map(|r| r.record_digest.clone()),
            ReviewStatePayload::ReviewSyncCarryOver {
                carry_over: carry_over.clone(),
            },
        )
        .unwrap()
        .with_assignment_generation(generation)
        .unwrap()
    };
    let wrong_head = [
        seeded.clone(),
        vec![record(&s.reviewed, seeded.last(), Some(1))],
    ]
    .concat();
    let ownerless = vec![record(&s.synced, None, None)];
    for chain in [wrong_head, ownerless] {
        let bodies: Vec<String> = chain
            .iter()
            .map(|r| review_state::render_state_comment_body(r, None).unwrap())
            .collect();
        assert!(
            review_state::parse_chain(bodies.iter().map(String::as_str), "acme/widgets", 7)
                .is_err()
        );
    }
}

#[test]
fn assignment_binds_the_base_ref_and_sha_from_the_same_provider_response() {
    let s = sync(false);
    // The initial PR view reports main; the later REST snapshot reports a
    // retarget to release. Persist only the matching pair from that snapshot.
    let stub = writable_ledger(fixture(&s.reviewed, &[], vec![]), &[])
        .env("PROVIDER_BASE", &s.main)
        .env("PROVIDER_BASE_REF", "release")
        .env("AGENT_REVIEWER_SESSION", "");
    let output = run_forge_cli_in(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-handoff",
            "assign",
            "7",
            "--expected-head",
            &s.reviewed,
            "--reviewer-session",
            "6ba7b810-9dad-41d1-80b4-00c04fd430c8",
            "--review-author",
            "review-app[bot]",
            "--base-sha",
            &s.main,
            "--expected-state",
            "none",
        ],
        Some(s.repo.path()),
    );
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let bodies = ledger_bodies(&stub);
    let record = review_state::parse_state_marker(&bodies[0])
        .unwrap()
        .unwrap();
    let ReviewStatePayload::ReviewHandoff { handoff } = record.payload else {
        panic!("expected assignment");
    };
    assert_eq!(handoff.base_sha, s.main);
    assert_eq!(handoff.base_ref.as_deref(), Some("release"));
}

#[test]
fn carry_over_without_an_active_assignment_is_refused() {
    let s = sync(false);
    let mut surrendered = assigned(&s.base, &s.reviewed);
    let ReviewStatePayload::ReviewHandoff { mut handoff } = surrendered[0].payload.clone() else {
        panic!("expected assignment");
    };
    handoff.surrendered = true;
    surrendered.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            &s.synced,
            2,
            Some(surrendered.last().unwrap().record_digest.clone()),
            ReviewStatePayload::ReviewHandoff { handoff },
        )
        .unwrap(),
    );
    for (seeded, code, error) in [
        (vec![], 65, "review_assignment_missing"),
        (surrendered, 69, "designated_reviewer_unavailable"),
    ] {
        let stub = writable_ledger(
            fixture(&s.synced, &seeded, vec![review(&s.reviewed, "pass")]),
            &seeded,
        )
        .env("PROVIDER_BASE", &s.main)
        .env("AGENT_REVIEWER_SESSION", "");
        let output = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
        assert_refusal(&output, code, error);
        assert_eq!(
            ledger_bodies(&stub).len(),
            seeded.len(),
            "no carry-over may be appended"
        );
    }
}

#[test]
fn malformed_second_parent_is_refused_before_merge_evaluation() {
    let s = sync(false);
    for parent in [
        format!("{}^{{commit}}", s.main),
        s.main[..39].to_string(),
        "g".repeat(40),
        "--write-tree".to_string(),
    ] {
        let raw = format!(
            "tree {}\nparent {}\nparent {parent}\n\nmalformed parent\n",
            s.repo.tree(&s.synced),
            s.reviewed,
        );
        let file = s.repo.path().join("malformed-commit");
        fs::write(&file, raw).unwrap();
        // --literally lets the fixture retain an invalid raw commit header.
        let head = s.repo.git(&[
            "hash-object",
            "--literally",
            "-t",
            "commit",
            "-w",
            file.to_str().unwrap(),
        ]);
        let stub = provider(&s, &head, vec![review(&s.reviewed, "pass")]);
        let output = handoff_cmd(&stub, s.repo.path(), false, "check", &head);
        assert_not_carried(&output, &stub, "not-a-sync-merge");
    }
}

#[test]
fn carry_over_from_an_earlier_interval_is_ignored() {
    let s = sync(false);
    let mut seeded = assigned(&s.base, &s.reviewed);
    let carry = SyncCarryOver {
        review_url: REVIEW_URL.into(),
        reviewed_head: s.reviewed.clone(),
        head: s.synced.clone(),
        base_commit: s.main.clone(),
        merge_tree: s.repo.tree(&s.synced),
    };
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            &s.synced,
            2,
            Some(seeded.last().unwrap().record_digest.clone()),
            ReviewStatePayload::ReviewSyncCarryOver { carry_over: carry },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let ReviewStatePayload::ReviewHandoff { mut handoff } = seeded[0].payload.clone() else {
        panic!("expected assignment");
    };
    handoff.assignment_generation = 2;
    handoff.base_sha = s.main.clone();
    let refresh = ReviewStateRecord::new(
        "acme/widgets",
        7,
        &s.reviewed,
        3,
        Some(seeded.last().unwrap().record_digest.clone()),
        ReviewStatePayload::ReviewHandoff { handoff },
    )
    .unwrap();
    let binding = refresh.record_digest.clone();
    seeded.push(refresh);
    let state = review_state::observe_review_loop(None, &s.reviewed, &[])
        .unwrap()
        .state;
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            &s.reviewed,
            4,
            Some(binding.clone()),
            ReviewStatePayload::ReviewLoop { state },
        )
        .unwrap()
        .with_assignment_generation(Some(2))
        .unwrap(),
    );
    let mut current = review(&s.reviewed, "pass");
    current["databaseId"] = json!(2);
    current["id"] = json!("REVIEW_2");
    current["url"] = json!("https://github.com/acme/widgets/pull/7#pullrequestreview-2");
    current["submittedAt"] = json!("2026-07-20T12:00:04Z");
    current["body"] = json!(format!(
        "{}\n<!-- forge-cli:review-handoff:v1 {binding} -->",
        current["body"].as_str().unwrap(),
    ));
    let stub = writable_ledger(fixture(&s.synced, &seeded, vec![current]), &seeded)
        .env("PROVIDER_BASE", &s.main)
        .env("AGENT_REVIEW_ASSIGNMENT_GENERATION", "2");
    let inspected = handoff_cmd(&stub, s.repo.path(), false, "inspect", &s.synced);
    assert_eq!(
        inspected.code, 0,
        "{} {}",
        inspected.stdout, inspected.stderr
    );
    assert!(parse_envelope(&inspected.stdout)["data"]["sync_carry_over"].is_null());
    let gate = merge_preview(&stub, s.repo.path());
    assert_refusal(&gate, 65, "awaiting_designated_review");
    let checked = handoff_cmd(&stub, s.repo.path(), false, "check", &s.synced);
    assert_eq!(checked.code, 0, "{} {}", checked.stdout, checked.stderr);
    assert_eq!(ledger_bodies(&stub).len(), 6);
    assert_eq!(
        parse_envelope(&checked.stdout)["data"]["sync_carry_over"]["review_url"],
        "https://github.com/acme/widgets/pull/7#pullrequestreview-2"
    );
    let gate = merge_preview(&stub, s.repo.path());
    assert_eq!(gate.code, 0, "{} {}", gate.stdout, gate.stderr);
}

//! Governed dismissal of automated CHANGES_REQUESTED reviews. Fixtures never
//! contact a live provider; every refusal must leave the review untouched.
use std::fs;

use pretty_assertions::assert_eq;
use serde_json::{Value, json};

use forge_cli::ops::review_state::{
    self, ReviewFindingObservation, ReviewFindingStatus, ReviewStatePayload, ReviewStateRecord,
};

use super::pr_review_handoff::{assert_refusal, fixture, handoff, records, review};
use super::support::{CmdOutput, StubEnv, parse_envelope, run_forge_cli};

const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OLD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const BOT_REVIEW_ID: u64 = 5;

/// An automated review submitted before the designated PASS (12:00:02).
fn automated_review(head: &str, state: &str, login: &str, typename: &str) -> Value {
    automated_review_at(head, state, login, typename, "2026-07-20T12:00:01Z")
}

fn automated_review_at(
    head: &str,
    state: &str,
    login: &str,
    typename: &str,
    submitted_at: &str,
) -> Value {
    json!({"id":"REVIEW_5", "databaseId":BOT_REVIEW_ID,
        "url":"https://github.com/acme/widgets/pull/7#pullrequestreview-5",
        "author":{"login":login, "__typename":typename, "id":"NODE_AUTOMATED"},
        "state":state, "commit":{"oid":head}, "submittedAt":submitted_at,
        "body":"Automated finding.", "viewerDidAuthor":false})
}

/// A designated PASS at HEAD plus the given automated review, with `lint-app`
/// configured as the only automated-review identity.
fn dismissal_fixture(target: Value, with_pass: bool) -> StubEnv {
    let mut reviews = vec![target];
    if with_pass {
        reviews.push(review(HEAD, "pass"));
    }
    with_wrapper(fixture(HEAD, &records(handoff(None), Some(HEAD)), reviews))
}

fn dismissal_fixture_with(target: Value, with_pass: bool, seeded: &[ReviewStateRecord]) -> StubEnv {
    let mut reviews = vec![target];
    if with_pass {
        reviews.push(review(HEAD, "pass"));
    }
    with_wrapper(fixture(HEAD, seeded, reviews))
}

fn configured_target() -> Value {
    automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot")
}

/// Answer the actor and dismissal calls, then delegate every read to the
/// designated-review fixture stub, which rejects any other mutation.
fn with_wrapper(stub: StubEnv) -> StubEnv {
    let dir = stub.tempdir.path().to_path_buf();
    let config = dir.join("xdg-config").join("forge-cli");
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("config.toml"),
        "[[review_convergence.bots]]\nlogin = \"lint-app\"\nmode = \"observed\"\n",
    )
    .unwrap();
    let script = format!(
        r#"#!/bin/sh
case "$*" in
  "api user --jq .login") echo "$*" >> "{log}"; echo "${{ACTOR_LOGIN-delivery-actor}}"; exit 0 ;;
  "api --method PUT repos/acme/widgets/pulls/7/reviews/{BOT_REVIEW_ID}/dismissals "*)
    echo "$*" >> "{log}"
    echo "{{\"id\":{BOT_REVIEW_ID},\"state\":\"${{DISMISS_STATE:-DISMISSED}}\",\"commit_id\":\"{HEAD}\"}}"
    exit 0 ;;
esac
exec "{base}" "$@"
"#,
        log = dir.join("calls.log").display(),
        base = dir.join("gh").display(),
    );
    let wrapper = stub.write_stub("gh-dismiss", &script);
    stub.env("FORGE_CLI_GH_BIN", wrapper.to_string_lossy())
}

fn dismiss_with(stub: &StubEnv, head: &str, reason: &str, extra: &[&str]) -> CmdOutput {
    let reason_path = stub.tempdir.path().join("reason.md");
    fs::write(&reason_path, reason).unwrap();
    let id = BOT_REVIEW_ID.to_string();
    let mut args = vec![
        "--provider",
        "github",
        "--repo",
        "acme/widgets",
        "--format",
        "json",
    ];
    args.extend_from_slice(extra);
    args.extend([
        "pr",
        "review-dismiss",
        "7",
        "--review-id",
        id.as_str(),
        "--expected-head",
        head,
        "--reason-file",
        reason_path.to_str().unwrap(),
    ]);
    run_forge_cli(stub, &args)
}

fn dismiss(stub: &StubEnv, head: &str, reason: &str) -> CmdOutput {
    dismiss_with(stub, head, reason, &[])
}

fn calls(stub: &StubEnv) -> String {
    fs::read_to_string(stub.tempdir.path().join("calls.log")).unwrap_or_default()
}

fn assert_untouched(stub: &StubEnv) {
    assert!(!calls(stub).contains("--method PUT"), "{}", calls(stub));
}

const REASON: &str = "Fixed in a follow-up commit; residual risk tracked in a follow-up issue.\n";

#[test]
fn configured_automated_review_is_dismissed_with_recorded_reason_actor_and_head() {
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        true,
    );
    let output = dismiss(&stub, HEAD, REASON);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let data = &parse_envelope(&output.stdout)["data"];
    assert_eq!(data["review_id"], BOT_REVIEW_ID);
    assert_eq!(data["author"], "lint-app");
    assert_eq!(data["head_sha"], HEAD);
    assert_eq!(data["actor"], "delivery-actor");
    assert_eq!(data["state"], "DISMISSED");
    assert_eq!(data["reason"], REASON.trim());
    assert!(data["dismissed_at"].as_str().is_some_and(|t| !t.is_empty()));
    let log = calls(&stub);
    // The logged argv spans lines because the message does; read from the call on.
    let put = &log[log.find("--method PUT").expect("dismissal call")..];
    let message = format!(
        "message={}\n\nDismissed with forge-cli pr review-dismiss at head {HEAD} by delivery-actor.",
        REASON.trim()
    );
    assert!(put.contains(&message), "{put}");
    assert!(put.contains("event=DISMISS"), "{put}");
    assert!(!output.stdout.contains("private-machine-canary"));
}

#[test]
fn dry_run_runs_every_guard_without_dismissing() {
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        true,
    );
    let output = dismiss_with(&stub, HEAD, REASON, &["--dry-run"]);
    assert_eq!(output.code, 0, "{} {}", output.stdout, output.stderr);
    let data = &parse_envelope(&output.stdout)["data"];
    assert_eq!(data["review_id"], BOT_REVIEW_ID);
    let plan = data["plan"].as_array().expect("dry-run plan").clone();
    assert!(
        plan.iter()
            .any(|a| a.as_str().is_some_and(|a| a.ends_with("/dismissals")))
    );
    assert_untouched(&stub);

    let human = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "alice", "User"),
        true,
    );
    let output = dismiss_with(&human, HEAD, REASON, &["--dry-run"]);
    assert_refusal(&output, 65, "review_dismiss_author_not_automated");
}

#[test]
fn human_or_unconfigured_authors_are_refused_and_names_never_imply_automation() {
    for (login, typename) in [
        // A human reviewer.
        ("alice", "User"),
        // A configured login that the provider reports as a user account.
        ("lint-app", "User"),
        // A provider App account that is not configured.
        ("other-app", "Bot"),
        // A bot-looking name that is neither configured nor an App account.
        ("lint-app-bot", "User"),
    ] {
        let stub = dismissal_fixture(
            automated_review(HEAD, "CHANGES_REQUESTED", login, typename),
            true,
        );
        assert_refusal(
            &dismiss(&stub, HEAD, REASON),
            65,
            "review_dismiss_author_not_automated",
        );
        assert_untouched(&stub);
    }
}

#[test]
fn wrong_pr_head_or_review_head_is_refused() {
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        true,
    );
    assert_refusal(
        &dismiss(&stub, OLD, REASON),
        65,
        "review_dismiss_head_mismatch",
    );
    assert_untouched(&stub);

    let stale = dismissal_fixture(
        automated_review(OLD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        true,
    );
    assert_refusal(
        &dismiss(&stale, HEAD, REASON),
        65,
        "review_dismiss_head_mismatch",
    );
    assert_untouched(&stale);
}

#[test]
fn only_changes_requested_reviews_are_dismissable() {
    for state in ["COMMENTED", "APPROVED", "DISMISSED"] {
        let stub = dismissal_fixture(automated_review(HEAD, state, "lint-app", "Bot"), true);
        assert_refusal(
            &dismiss(&stub, HEAD, REASON),
            65,
            "review_dismiss_state_invalid",
        );
        assert_untouched(&stub);
    }
}

#[test]
fn empty_reason_is_refused_before_any_provider_call() {
    for reason in ["", "  \n\t\n"] {
        let stub = dismissal_fixture(
            automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
            true,
        );
        assert_refusal(
            &dismiss(&stub, HEAD, reason),
            65,
            "review_dismiss_reason_missing",
        );
        assert_eq!(calls(&stub), "");
    }
}

#[test]
fn missing_designated_pass_is_refused() {
    // Handoff exists, but no designated PASS is published at the head.
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        false,
    );
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "awaiting_designated_review",
    );
    assert_untouched(&stub);

    // No designated review at all: dismissal never replaces designated review.
    let mut unassigned = with_wrapper(fixture(
        HEAD,
        &[],
        vec![automated_review(
            HEAD,
            "CHANGES_REQUESTED",
            "lint-app",
            "Bot",
        )],
    ));
    unassigned
        .envs
        .retain(|(k, _)| k != "AGENT_REVIEWER_SESSION");
    assert_refusal(
        &dismiss(&unassigned, HEAD, REASON),
        65,
        "awaiting_designated_review",
    );
    assert_untouched(&unassigned);
}

#[test]
fn designated_reviewer_cannot_dismiss_or_be_dismissed() {
    // The designated reviewer session may not clear blockers on a head it passed.
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "lint-app", "Bot"),
        true,
    )
    .env("AGENT_SESSION_ID", "reviewer-session");
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "review_dismiss_self_review",
    );
    assert_untouched(&stub);

    // The designated reviewer's own blocking review is never an automated blocker,
    // even when its App login is also configured as automated.
    let stub = dismissal_fixture(
        automated_review(HEAD, "CHANGES_REQUESTED", "review-app", "Bot"),
        true,
    );
    let config = stub.tempdir.path().join("xdg-config/forge-cli/config.toml");
    fs::write(
        &config,
        "[[review_convergence.bots]]\nlogin = \"review-app[bot]\"\nmode = \"observed\"\n",
    )
    .unwrap();
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "review_dismiss_designated_reviewer",
    );
    assert_untouched(&stub);
}

#[test]
fn gitlab_refuses_as_unsupported_before_any_call() {
    let stub = StubEnv::new();
    let reason = stub.tempdir.path().join("reason.md");
    fs::write(&reason, REASON).unwrap();
    let output = run_forge_cli(
        &stub,
        &[
            "--provider",
            "gitlab",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "pr",
            "review-dismiss",
            "7",
            "--review-id",
            "5",
            "--expected-head",
            HEAD,
            "--reason-file",
            reason.to_str().unwrap(),
        ],
    );
    assert_eq!(output.code, 64, "{} {}", output.stdout, output.stderr);
    assert!(
        output.stdout.contains("provider_unsupported"),
        "{}",
        output.stdout
    );
    assert!(output.stdout.contains("GitLab"), "{}", output.stdout);
}

#[test]
fn repository_config_never_grants_dismissal_authority() {
    // A PR branch can edit `.forge-cli.toml`, so only the user-global layer counts.
    let stub = dismissal_fixture(configured_target(), true);
    let dir = stub.tempdir.path();
    fs::remove_file(dir.join("xdg-config/forge-cli/config.toml")).unwrap();
    fs::write(
        dir.join(".forge-cli.toml"),
        "[[review_convergence.bots]]\nlogin = \"lint-app\"\nmode = \"observed\"\n",
    )
    .unwrap();
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "review_dismiss_author_not_automated",
    );
    assert_untouched(&stub);
}

#[test]
fn review_submitted_after_the_designated_pass_is_refused() {
    let stub = dismissal_fixture(
        automated_review_at(
            HEAD,
            "CHANGES_REQUESTED",
            "lint-app",
            "Bot",
            "2026-07-20T12:00:04Z",
        ),
        true,
    );
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "review_dismiss_after_designated_pass",
    );
    assert_untouched(&stub);
}

#[test]
fn actor_must_prove_it_is_neither_the_designated_session_nor_account() {
    // The harness inherits the caller's session id, so blank it explicitly.
    let anonymous = dismissal_fixture(configured_target(), true).env("AGENT_SESSION_ID", "");
    assert_refusal(
        &dismiss(&anonymous, HEAD, REASON),
        65,
        "review_dismiss_self_review",
    );
    assert_eq!(calls(&anonymous), "");

    let same_account =
        dismissal_fixture(configured_target(), true).env("ACTOR_LOGIN", "review-app[bot]");
    assert_refusal(
        &dismiss(&same_account, HEAD, REASON),
        65,
        "review_dismiss_self_review",
    );
    assert_untouched(&same_account);
}

#[test]
fn open_blocking_ledger_finding_blocks_dismissal_despite_a_pass() {
    let mut seeded = records(handoff(None), None);
    let open = ReviewFindingObservation {
        fingerprint: "correctness:fixture:open-finding".into(),
        root_cause_fingerprint: None,
        blocking: true,
        status: ReviewFindingStatus::Open,
        threads: vec![],
    };
    let state = review_state::observe_review_loop(None, HEAD, &[open])
        .unwrap()
        .state;
    let previous = seeded.last().unwrap().record_digest.clone();
    seeded.push(
        ReviewStateRecord::new(
            "acme/widgets",
            7,
            HEAD,
            1,
            Some(previous),
            ReviewStatePayload::ReviewLoop { state },
        )
        .unwrap()
        .with_assignment_generation(Some(1))
        .unwrap(),
    );
    let stub = dismissal_fixture_with(configured_target(), true, &seeded);
    assert_refusal(
        &dismiss(&stub, HEAD, REASON),
        65,
        "awaiting_designated_review",
    );
    assert_untouched(&stub);
}

#[test]
fn unsafe_reason_text_is_refused_before_any_provider_call() {
    for (reason, kind) in [
        (
            "Logs are under /Users/dev/Project/secret/out.md\n",
            "local_path_present",
        ),
        (
            "Dispositioned.\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>\n",
            "agent_attribution_present",
        ),
    ] {
        let stub = dismissal_fixture(configured_target(), true);
        assert_refusal(&dismiss(&stub, HEAD, reason), 65, kind);
        assert_eq!(calls(&stub), "");
    }
}

#[test]
fn unknown_actor_or_unconfirmed_dismissal_fails_closed() {
    let unknown = dismissal_fixture(configured_target(), true).env("ACTOR_LOGIN", "");
    assert_refusal(
        &dismiss(&unknown, HEAD, REASON),
        69,
        "review_dismiss_actor_unknown",
    );
    assert_untouched(&unknown);

    let unconfirmed =
        dismissal_fixture(configured_target(), true).env("DISMISS_STATE", "CHANGES_REQUESTED");
    assert_refusal(
        &dismiss(&unconfirmed, HEAD, REASON),
        69,
        "review_dismiss_unconfirmed",
    );
}

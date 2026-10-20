//! Complete native-review reads use the normal, fixed provider transport.
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

const HEAD: &str = "1111111111111111111111111111111111111111";
const REVIEW_URL: &str = "https://github.com/acme/widgets/pull/42#pullrequestreview-102";

struct Fixture {
    temp: tempfile::TempDir,
    view: Value,
    snapshot: Value,
    review: Value,
}

impl Fixture {
    fn new(body: &str) -> Self {
        Self {
            temp: tempfile::tempdir().unwrap(),
            view: json!({"number":42,"url":"https://github.com/acme/widgets/pull/42",
                "state":"OPEN","isDraft":false,"baseRefName":"main",
                "headRefName":"feat/example","headRefOid":HEAD,"title":"Example","body":""}),
            snapshot: json!({"data":{"viewer":{"login":"reader"},"repository":{"pullRequest":{
                "headRefOid":HEAD,"reviews":{"nodes":[{
                    "id":"PRR_fixture","databaseId":102,"url":REVIEW_URL,
                    "author":{"login":"reviewer","__typename":"User","id":"U_reviewer"},
                    "state":"APPROVED","commit":{"oid":HEAD},
                    "submittedAt":"2026-01-01T00:00:00Z","body":body
                }],"pageInfo":{"hasNextPage":false,"endCursor":null}}}}}}),
            review: json!({"id":102,"node_id":"PRR_fixture","html_url":REVIEW_URL,
                "pull_request_url":"https://api.github.com/repos/acme/widgets/pulls/42",
                "commit_id":HEAD,"user":{"login":"reviewer","node_id":"U_reviewer","type":"User"},
                "state":"APPROVED","submitted_at":"2026-01-01T00:00:00Z","body":body}),
        }
    }

    fn run(&self, full: bool, dry_run: bool) -> (i32, Value, String) {
        let root = self.temp.path();
        for (name, value) in [
            ("view", &self.view),
            ("snapshot", &self.snapshot),
            ("review", &self.review),
        ] {
            fs::write(root.join(format!("{name}.json")), value.to_string()).unwrap();
        }
        let backend = root.join("gh");
        fs::write(
            &backend,
            r#"#!/bin/sh
set -eu
printf '%s\n' "$*" >> "$FIXTURE_ROOT/calls"
case "$1 $2" in
  'pr view') cat "$FIXTURE_ROOT/view.json" ;;
  'api graphql') cat "$FIXTURE_ROOT/snapshot.json" ;;
  'api repos/acme/widgets/pulls/42/reviews/102') cat "$FIXTURE_ROOT/review.json" ;;
  *) echo unexpected-provider-call >&2; exit 99 ;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&backend, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(root.join("calls"), "").unwrap();
        let mut command = Command::new(nils_test_support::bin::resolve("forge-cli"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", root)
            .env("XDG_CONFIG_HOME", root.join("config"))
            .env("XDG_STATE_HOME", root.join("state"))
            .env("FORGE_CLI_GH_BIN", backend)
            .env("FORGE_CLI_RATE_LIMIT_GATE", "off")
            .env("FIXTURE_ROOT", root)
            .current_dir(root)
            .args([
                "--provider",
                "github",
                "--repo",
                "acme/widgets",
                "--format",
                "json",
            ]);
        if dry_run {
            command.arg("--dry-run");
        }
        command.args(["pr", "reviews", "42"]);
        if full {
            command.args(["--review-id", "102"]);
        }
        let output = command.output().unwrap();
        let envelope = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
            json!({"stdout":String::from_utf8_lossy(&output.stdout),
                "stderr":String::from_utf8_lossy(&output.stderr)})
        });
        (
            output.status.code().unwrap_or(-1),
            envelope,
            fs::read_to_string(root.join("calls")).unwrap(),
        )
    }
}

#[test]
fn full_review_returns_the_unicode_suffix_beyond_summary_limit() {
    let body = format!(
        "{}\nFinal acceptance qualifier.",
        "Review 確認. ".repeat(600)
    );
    assert!(body.len() > 4096);
    let fixture = Fixture::new(&body);
    let (code, envelope, calls) = fixture.run(true, false);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["schema_version"], "cli.forge-cli.pr.review-get.v1");
    let data = &envelope["data"];
    assert_eq!(data["body"], body);
    assert_eq!(data["database_id"], 102);
    assert_eq!(data["author"], "reviewer");
    assert_eq!(data["commit_sha"], HEAD);
    assert_eq!(data["state"], "APPROVED");
    assert_eq!(data["url"], REVIEW_URL);
    assert_eq!(data["body_bytes"], body.len());
    assert_eq!(data["body_truncated"], false);
    assert_eq!(data["complete"], true);
    assert!(data["observed_at"].as_str().is_some_and(|s| !s.is_empty()));
    assert_eq!(
        calls
            .matches("repos/acme/widgets/pulls/42/reviews/102")
            .count(),
        1
    );
}

#[test]
fn full_review_preserves_ordinary_and_empty_native_bodies() {
    for body in ["Ordinary report.", ""] {
        let (code, envelope, _) = Fixture::new(body).run(true, false);
        assert_eq!(code, 0, "{envelope}");
        assert_eq!(envelope["data"]["body"], body);
        assert_eq!(envelope["data"]["complete"], true);
    }
}

#[test]
fn default_review_summary_keeps_its_existing_bound_and_schema() {
    let body = "Review 確認. ".repeat(600);
    let (code, envelope, calls) = Fixture::new(&body).run(false, false);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["schema_version"], "cli.forge-cli.pr.reviews.v1");
    let review = &envelope["data"]["current_head_reviews"][0];
    assert_eq!(review["summary_truncated"], true);
    assert!(review["summary"].as_str().unwrap().len() <= 4096);
    assert!(!calls.contains("/reviews/102"));
}

#[test]
fn full_review_refuses_missing_or_changed_native_review_binding() {
    let mut missing = Fixture::new("Report.");
    missing.snapshot["data"]["repository"]["pullRequest"]["reviews"]["nodes"] = json!([]);
    let (code, envelope, calls) = missing.run(true, false);
    assert_eq!(code, 65, "{envelope}");
    assert_eq!(envelope["error"]["code"], "review_not_found");
    assert!(!calls.contains("/reviews/102"));

    for (pointer, replacement) in [
        ("/id", json!(999)),
        (
            "/html_url",
            json!("https://github.com/acme/widgets/pull/99#pullrequestreview-102"),
        ),
        (
            "/commit_id",
            json!("2222222222222222222222222222222222222222"),
        ),
        ("/user/login", json!("other-reviewer")),
        ("/user/node_id", json!("U_other")),
        ("/state", json!("CHANGES_REQUESTED")),
    ] {
        let mut fixture = Fixture::new("Report.");
        *fixture.review.pointer_mut(pointer).unwrap() = replacement;
        let (code, envelope, _) = fixture.run(true, false);
        assert_eq!(code, 65, "{pointer}: {envelope}");
        assert_eq!(envelope["error"]["code"], "review_snapshot_incomplete");
    }
}

#[test]
fn full_review_refuses_a_body_beyond_the_existing_complete_report_bound() {
    let (code, envelope, _) = Fixture::new(&"x".repeat(64 * 1024)).run(true, false);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["body_bytes"], 64 * 1024);
    let (code, envelope, _) = Fixture::new(&"x".repeat(64 * 1024 + 1)).run(true, false);
    assert_eq!(code, 65, "{envelope}");
    assert_eq!(envelope["error"]["code"], "review_snapshot_incomplete");
}

#[test]
fn full_review_keeps_stale_and_pending_native_metadata() {
    let mut stale = Fixture::new("Earlier report.");
    let old = "2222222222222222222222222222222222222222";
    stale.snapshot["data"]["repository"]["pullRequest"]["reviews"]["nodes"][0]["commit"]["oid"] =
        json!(old);
    stale.review["commit_id"] = json!(old);
    let (code, envelope, _) = stale.run(true, false);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["head_sha"], HEAD);
    assert_eq!(envelope["data"]["commit_sha"], old);

    let mut pending = Fixture::new("Draft report.");
    let node = &mut pending.snapshot["data"]["repository"]["pullRequest"]["reviews"]["nodes"][0];
    node["state"] = json!("PENDING");
    node["commit"] = Value::Null;
    node["submittedAt"] = Value::Null;
    pending.review["state"] = json!("PENDING");
    pending.review["commit_id"] = Value::Null;
    let (code, envelope, _) = pending.run(true, false);
    assert_eq!(code, 0, "{envelope}");
    assert_eq!(envelope["data"]["state"], "PENDING");
    assert!(envelope["data"]["commit_sha"].is_null());
    assert!(envelope["data"]["submitted_at"].is_null());
}

#[test]
fn full_review_never_follows_a_changed_repository_or_pr_binding() {
    for url in [
        "https://github.com/other/widgets/pull/42",
        "https://github.com/acme/widgets/pull/99",
        "https://example.invalid/acme/widgets/pull/42",
    ] {
        let mut fixture = Fixture::new("Report.");
        fixture.view["url"] = json!(url);
        let (code, envelope, calls) = fixture.run(true, false);
        assert_eq!(code, 65, "{envelope}");
        assert_eq!(envelope["error"]["code"], "review_snapshot_incomplete");
        assert!(!calls.contains("api "));
    }
}

#[test]
fn full_review_has_a_network_read_only_effect_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    let output = Command::new(nils_test_support::bin::resolve("forge-cli"))
        .env_clear()
        .current_dir(temp.path())
        .args([
            "operation-effect",
            "--format",
            "json",
            "--",
            "pr",
            "reviews",
            "42",
            "--review-id",
            "102",
            "--repo",
            "acme/widgets",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["data"]["operation"], "pr.review-get");
    assert_eq!(envelope["data"]["effect"], "read_only");
    assert_eq!(envelope["data"]["provider_effect"], "network_read");
}

#[test]
fn full_review_dry_run_names_the_fixed_get_without_provider_execution() {
    let (code, envelope, calls) = Fixture::new("Report.").run(true, true);
    assert_eq!(code, 0, "{envelope}");
    assert!(calls.is_empty());
    assert!(
        envelope["data"]["plan"]
            .to_string()
            .contains("repos/acme/widgets/pulls/42/reviews/102")
    );
    assert!(envelope["data"].get("body").is_none());
}

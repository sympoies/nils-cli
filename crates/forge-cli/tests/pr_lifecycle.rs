//! Provider fixtures for additive PR lifecycle metadata.
#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use support::{StubEnv, parse_envelope, run_forge_cli};

const VIEW: &str = include_str!("fixtures/github/pr_lifecycle/view.json");
const QUEUED: &str = include_str!("fixtures/github/pr_lifecycle/queued.json");
const NEVER: &str = include_str!("fixtures/github/pr_lifecycle/never_promoted.json");
const CHECKS: &str = include_str!("fixtures/github/pr_lifecycle/checks.json");

fn script(view: &str, lifecycle: &str) -> String {
    format!(
        r#"#!/bin/sh
case "$1 $2" in
  "pr view") cat <<'VIEW'
{view}
VIEW
;;
  "api graphql")
    case "$*" in *'READY_FOR_REVIEW_EVENT'*'last:1'*) ;; *) exit 91;; esac
    cat <<'LIFECYCLE'
{lifecycle}
LIFECYCLE
;;
  *) case "$*" in *'commits/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/check-runs?per_page=100'*'--paginate'*'--slurp'*) cat <<'CHECKS'
{CHECKS}
CHECKS
;; *) echo unexpected-call >&2; exit 92;; esac;;
esac
"#
    )
}

fn read(script: &str) -> Value {
    let stub = StubEnv::new().gh_stub(script);
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "view",
            "example-branch",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    parse_envelope(&out.stdout)["data"].clone()
}

fn null_stages(data: &Value) {
    for key in ["ready_for_review_at", "merge_queue", "merge_group"] {
        assert_eq!(
            data.get(key),
            Some(&Value::Null),
            "{key} must be explicitly null"
        );
    }
}

#[test]
fn lifecycle_queued_provider_timestamps_and_group_checks() {
    let data = read(&script(VIEW, QUEUED));
    assert_eq!(data["created_at"], "2026-01-01T01:00:00Z");
    assert_eq!(data["ready_for_review_at"], "2026-01-02T02:00:00Z");
    assert_eq!(
        data["merge_queue"],
        json!({"queued":true,"position":2,"state":"AWAITING_CHECKS","enqueued_at":"2026-01-03T03:00:00Z","head_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"})
    );
    assert_eq!(
        data["merge_group"]["head_sha"],
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(data["merge_group"]["started_at"], "2026-01-03T03:01:00Z");
    assert_eq!(data["merge_group"]["completed_at"], Value::Null);
    assert_eq!(data["merge_group"]["checks"].as_array().unwrap().len(), 2);
    assert_eq!(
        data["merge_group"]["checks"][0]["completed_at"],
        "2026-01-03T03:04:00Z"
    );
    assert_eq!(
        data["merge_group"]["checks"][1]["completed_at"],
        Value::Null
    );
    assert_eq!(data["title"], "Example change");
}

#[test]
fn lifecycle_never_promoted_and_draft_stages_are_null() {
    for draft in [false, true] {
        let mut view: Value = serde_json::from_str(VIEW).unwrap();
        view["isDraft"] = json!(draft);
        let data = read(&script(&view.to_string(), NEVER));
        assert_eq!(data["created_at"], "2026-01-01T01:00:00Z");
        null_stages(&data);
    }
}

#[test]
fn lifecycle_absent_created_timestamp_is_null() {
    let mut view: Value = serde_json::from_str(VIEW).unwrap();
    view.as_object_mut().unwrap().remove("createdAt");
    let data = read(&script(&view.to_string(), NEVER));
    assert_eq!(data.get("created_at"), Some(&Value::Null));
    null_stages(&data);
}

#[test]
fn lifecycle_unsupported_queue_schema_retains_ready_timestamp() {
    let unsupported = include_str!("fixtures/github/pr_lifecycle/unsupported_queue.json");
    let mut fallback: Value = serde_json::from_str(QUEUED).unwrap();
    fallback["data"]["repository"]["pullRequest"]
        .as_object_mut()
        .unwrap()
        .remove("mergeQueueEntry");
    let body = format!(
        r#"#!/bin/sh
case "$1 $2" in
"pr view") cat <<'VIEW'
{VIEW}
VIEW
;;
"api graphql") case "$*" in *mergeQueueEntry*) cat <<'ERROR' >&2
{unsupported}
ERROR
exit 1;; *) cat <<'FALLBACK'
{fallback}
FALLBACK
;; esac;;
*) exit 93;; esac
"#
    );
    let data = read(&body);
    assert_eq!(data["ready_for_review_at"], "2026-01-02T02:00:00Z");
    assert_eq!(data.get("merge_queue"), Some(&Value::Null));
    assert_eq!(data.get("merge_group"), Some(&Value::Null));
}

#[test]
fn lifecycle_gitlab_has_created_timestamp_and_null_unsupported_stages() {
    let view = include_str!("fixtures/gitlab/pr_lifecycle/view.json");
    let stub = StubEnv::new().glab_stub(&format!("#!/bin/sh\ncat <<'VIEW'\n{view}\nVIEW\n"));
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "view",
            "7",
            "--provider",
            "gitlab",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    let data = &parse_envelope(&out.stdout)["data"];
    assert_eq!(data["created_at"], "2026-01-01T01:00:00Z");
    null_stages(data);
}

#[test]
fn lifecycle_local_provider_does_not_invent_timestamps() {
    use forge_cli::{
        backend::BackendSuccess,
        ops::pr_view::parse_view_output,
        provider::{DetectionSource, Provider, ProviderContext},
    };
    let context = ProviderContext {
        provider: Provider::Local,
        host: "local".into(),
        source: DetectionSource::Flag,
        repo: None,
    };
    let output = BackendSuccess {
        stdout: VIEW.replace("2026-01-01T01:00:00Z", ""),
        stderr: String::new(),
    };
    let data = serde_json::to_value(parse_view_output(&context, &output).unwrap()).unwrap();
    assert_eq!(data.get("created_at"), Some(&Value::Null));
    null_stages(&data);
}

#[test]
fn lifecycle_completed_group_reports_last_completion() {
    let checks = CHECKS.replace("in_progress", "completed").replace(
        "\"completed_at\": null",
        "\"completed_at\": \"2026-01-03T03:05:00Z\"",
    );
    let data = read(&script(VIEW, QUEUED).replace(CHECKS, &checks));
    assert_eq!(data["merge_group"]["completed_at"], "2026-01-03T03:05:00Z");
}

#[test]
fn lifecycle_queue_without_group_commit_has_null_group() {
    let mut response: Value = serde_json::from_str(QUEUED).unwrap();
    response["data"]["repository"]["pullRequest"]["mergeQueueEntry"]["headCommit"] = Value::Null;
    let data = read(&script(VIEW, &response.to_string()));
    assert_eq!(data["merge_queue"]["queued"], true);
    assert_eq!(data.get("merge_group"), Some(&Value::Null));
}

#[test]
fn lifecycle_never_relabels_pr_head_checks_as_merge_group() {
    let data = read(&script(
        VIEW,
        &QUEUED.replace(&"b".repeat(40), &"a".repeat(40)),
    ));
    assert_eq!(data["merge_queue"]["queued"], true);
    assert_eq!(data.get("merge_group"), Some(&Value::Null));
}

#[test]
fn lifecycle_queue_unknown_values_stay_null() {
    let mut response: Value = serde_json::from_str(QUEUED).unwrap();
    response["data"]["repository"]["pullRequest"]["mergeQueueEntry"] = json!({});
    let data = read(&script(VIEW, &response.to_string()));
    assert_eq!(
        data["merge_queue"],
        json!({"queued":true,"position":null,"state":null,"enqueued_at":null,"head_sha":null})
    );
    assert_eq!(data.get("merge_group"), Some(&Value::Null));
}

#[test]
fn lifecycle_backend_failures_are_not_hidden_as_unsupported() {
    let stub = StubEnv::new().gh_stub(&script(VIEW, QUEUED).replace(
        "\"api graphql\")",
        "\"api graphql\") echo 'permission denied' >&2; exit 1;",
    ));
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "view",
            "7",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
    );
    assert_ne!(out.code, 0);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "backend_error"
    );
}

#[test]
fn lifecycle_text_and_dry_run_do_not_add_provider_reads() {
    let stub = StubEnv::new().gh_stub(&format!(
        "#!/bin/sh\ncase \"$1 $2\" in 'pr view') cat <<'VIEW'\n{VIEW}\nVIEW\n;; *) exit 94;; esac\n"
    ));
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "view",
            "7",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
    assert!(out.stdout.contains("Example change"));
    let dry = StubEnv::new().gh_stub("#!/bin/sh\nexit 95\n");
    let out = run_forge_cli(
        &dry,
        &[
            "pr",
            "view",
            "7",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
            "--dry-run",
        ],
    );
    assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
}

#[test]
fn lifecycle_empty_group_has_no_invented_check_timestamps() {
    let data = read(&script(VIEW, QUEUED).replace(CHECKS, "[{\"check_runs\":[]}]"));
    assert_eq!(data["merge_group"]["started_at"], Value::Null);
    assert_eq!(data["merge_group"]["completed_at"], Value::Null);
    assert_eq!(data["merge_group"]["checks"], json!([]));
}

#[test]
fn lifecycle_merged_pr_does_not_guess_a_historical_group() {
    let mut view: Value = serde_json::from_str(VIEW).unwrap();
    view["state"] = json!("MERGED");
    view["mergedAt"] = json!("2026-01-04T04:00:00Z");
    let data = read(&script(&view.to_string(), NEVER));
    assert_eq!(data["merged_at"], "2026-01-04T04:00:00Z");
    null_stages(&data);
}

#[test]
fn lifecycle_shared_queue_reader_returns_the_view_contract() {
    use forge_cli::{
        backend::{BackendCall, BackendRunner, BackendSuccess},
        error::ForgeError,
        ops::merge_queue,
        provider::{DetectionSource, Provider, ProviderContext},
    };
    struct Fixture;
    impl BackendRunner for Fixture {
        fn run(&self, call: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            assert!(
                call.plan_argv()
                    .iter()
                    .any(|arg| arg.contains("mergeQueueEntry"))
            );
            Ok(BackendSuccess {
                stdout: QUEUED.into(),
                stderr: String::new(),
            })
        }
    }
    let mut context = ProviderContext {
        provider: Provider::GitHub,
        host: "github.com".into(),
        source: DetectionSource::Flag,
        repo: None,
    };
    let queued = merge_queue::read(&Fixture, &context, "acme", "widget", 7)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(queued).unwrap(),
        read(&script(VIEW, QUEUED))["merge_queue"]
    );
    struct Never;
    impl BackendRunner for Never {
        fn run(&self, _: &BackendCall) -> Result<BackendSuccess, ForgeError> {
            panic!("unsupported providers must not call a backend");
        }
    }
    for provider in [Provider::GitLab, Provider::Local] {
        context.provider = provider;
        assert_eq!(
            merge_queue::read(&Never, &context, "acme", "widget", 7).unwrap(),
            None
        );
    }
}

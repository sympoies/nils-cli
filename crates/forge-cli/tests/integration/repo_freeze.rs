//! `repo freeze start|end|status` integration tests. The freeze record is an
//! open GitHub issue labelled `merge-freeze`; `pr merge` enforces it.

use pretty_assertions::assert_eq;

use super::support::{StubEnv, parse_envelope, run_forge_cli};

const ACTIVE_FREEZES: &str = r#"[{"number":42,"title":"Merge freeze: nils-cli 1.29.3 release","url":"https://github.com/acme/widgets/issues/42","author":{"login":"release-bot"},"createdAt":"2026-09-29T00:00:00Z"}]"#;

fn freeze_stub(stub: &StubEnv, label_exists: bool, active: &str) -> String {
    let label_created = stub.tempdir.path().join("label-created");
    let issue_args = stub.tempdir.path().join("issue-create-args");
    let closed = stub.tempdir.path().join("issue-closed");
    let labels = if label_exists {
        r#"[{"name":"merge-freeze"}]"#
    } else {
        "[]"
    };
    format!(
        r#"#!/bin/sh
set -eu
case "$1 $2" in
  "label list") printf '%s\n' '{labels}' ;;
  "label create") touch {label_created} ;;
  "issue create")
    printf '%s\n' "$*" > {issue_args}
    printf '%s\n' 'https://github.com/acme/widgets/issues/43'
    ;;
  "issue list") printf '%s\n' '{active}' ;;
  "issue close") printf '%s\n' "$*" > {closed} ;;
  *) echo "unexpected gh args: $*" >&2; exit 99 ;;
esac
"#,
        labels = labels,
        label_created = label_created.display(),
        issue_args = issue_args.display(),
        active = active,
        closed = closed.display(),
    )
}

#[test]
fn repo_freeze_status_lists_open_merge_freeze_issues() {
    let stub = StubEnv::new();
    let body = freeze_stub(&stub, true, ACTIVE_FREEZES);
    let stub = stub.gh_stub(&body);

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "freeze",
            "status",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["schema_version"], "cli.forge-cli.repo.freeze.v1");
    assert_eq!(env["data"]["active"], true);
    assert_eq!(env["data"]["freezes"][0]["number"], 42);
    assert_eq!(env["data"]["freezes"][0]["author"], "release-bot");
}

#[test]
fn repo_freeze_start_creates_the_label_when_missing_and_opens_the_record() {
    let stub = StubEnv::new();
    let label_created = stub.tempdir.path().join("label-created");
    let issue_args = stub.tempdir.path().join("issue-create-args");
    let body = freeze_stub(&stub, false, "[]");
    let stub = stub.gh_stub(&body);

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "freeze",
            "start",
            "--reason",
            "nils-cli 1.29.3 release",
            "--until",
            "2026-09-29T08:00Z",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["action"], "start");
    assert_eq!(env["data"]["freezes"][0]["number"], 43);
    assert!(label_created.exists(), "the missing label must be created");
    let args = std::fs::read_to_string(issue_args).expect("issue create args");
    assert!(args.contains("--label merge-freeze"), "{args}");
    assert!(args.contains("nils-cli 1.29.3 release"), "{args}");
    assert!(args.contains("2026-09-29T08:00Z"), "{args}");
}

#[test]
fn repo_freeze_start_reuses_an_existing_label() {
    let stub = StubEnv::new();
    let label_created = stub.tempdir.path().join("label-created");
    let body = freeze_stub(&stub, true, "[]");
    let stub = stub.gh_stub(&body);

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "freeze",
            "start",
            "--reason",
            "infra cutover",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert!(
        !label_created.exists(),
        "an existing label must not be recreated"
    );
}

#[test]
fn repo_freeze_end_closes_the_single_active_freeze() {
    let stub = StubEnv::new();
    let closed = stub.tempdir.path().join("issue-closed");
    let body = freeze_stub(&stub, true, ACTIVE_FREEZES);
    let stub = stub.gh_stub(&body);

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "freeze",
            "end",
        ],
    );

    assert_eq!(out.code, 0, "stdout={}\nstderr={}", out.stdout, out.stderr);
    let env = parse_envelope(&out.stdout);
    assert_eq!(env["data"]["action"], "end");
    assert_eq!(env["data"]["freezes"][0]["number"], 42);
    let args = std::fs::read_to_string(closed).expect("issue close args");
    assert!(args.contains("issue close 42"), "{args}");
}

#[test]
fn repo_freeze_end_without_an_active_freeze_fails() {
    let stub = StubEnv::new();
    let body = freeze_stub(&stub, true, "[]");
    let stub = stub.gh_stub(&body);

    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "repo",
            "freeze",
            "end",
        ],
    );

    assert_eq!(out.code, 65, "stdout={}\nstderr={}", out.stdout, out.stderr);
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "merge_freeze_not_active"
    );
}

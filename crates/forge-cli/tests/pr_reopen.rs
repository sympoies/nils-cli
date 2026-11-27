#[allow(dead_code)]
#[path = "integration/support.rs"]
mod support;
use pretty_assertions::assert_eq;
use support::{StubEnv, parse_envelope, run_forge_cli};

#[test]
fn close_then_reopen_reads_back_open_for_both_providers() {
    for provider in ["github", "gitlab"] {
        let script = r##"#!/bin/sh
case "$1 $2" in
  "pr close"|"mr close") echo closed > state ;;
  "pr reopen"|"mr reopen") echo open > state ;;
  "pr view") state=$(cat state); printf '{"number":7,"url":"https://example.com/pr/7","state":"%s","isDraft":false,"title":"t","headRefName":"x","headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","baseRefName":"main","mergeable":"MERGEABLE","mergedAt":null,"labels":[]}' "$state" ;;
  "mr view") state=$(cat state); printf '{"iid":7,"web_url":"https://example.com/mr/7","state":"%s","title":"t","source_branch":"x","sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_branch":"main","merge_status":"can_be_merged","draft":false,"labels":[]}' "$state" ;;
  *) exit 91 ;;
esac
"##;
        let stub = if provider == "github" {
            StubEnv::new().gh_stub(script)
        } else {
            StubEnv::new().glab_stub(script)
        };
        for (command, state) in [("close", "closed"), ("reopen", "open")] {
            let mut args = vec![
                "pr",
                command,
                "7",
                "--provider",
                provider,
                "--repo",
                "acme/widget",
                "--format",
                "json",
            ];
            if command == "reopen" {
                args.extend([
                    "--expected-head",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ]);
            }
            let out = run_forge_cli(&stub, &args);
            assert_eq!(out.code, 0, "{} {}", out.stdout, out.stderr);
            let envelope = parse_envelope(&out.stdout);
            assert_eq!(envelope["data"]["state"], state);
            assert_eq!(
                envelope["schema_version"],
                format!("cli.forge-cli.pr.{command}.v1")
            );
        }
    }
}

#[test]
fn reopen_refuses_wrong_head_before_mutation() {
    let stub = StubEnv::new().gh_stub(r##"#!/bin/sh
case "$1 $2" in
  "pr view") printf '%s' '{"number":7,"url":"https://example.com/pr/7","state":"CLOSED","isDraft":false,"title":"t","headRefName":"x","headRefOid":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","baseRefName":"main","mergedAt":null,"labels":[]}' ;;
  *) echo mutation-was-called >&2; exit 91 ;;
esac
"##);
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "reopen",
            "7",
            "--expected-head",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "--provider",
            "github",
            "--repo",
            "acme/widget",
            "--format",
            "json",
        ],
    );
    assert_ne!(out.code, 0);
    assert!(!out.stderr.contains("mutation-was-called"));
    assert_eq!(
        parse_envelope(&out.stdout)["error"]["code"],
        "pr_head_changed"
    );
}

#[test]
fn reopen_refuses_invalid_states_and_post_mutation_mismatch() {
    for provider in ["github", "gitlab"] {
        for (initial, after, expected_code, calls) in [
            ("open", "open", "pr_already_open", "view\n"),
            ("merged", "merged", "pr_already_merged", "view\n"),
            (
                "closed",
                "closed",
                "pr_reopen_state_mismatch",
                "view\nreopen\nview\n",
            ),
        ] {
            let script = r##"#!/bin/sh
printf '%s\n' "$2" >> calls
case "$2" in
  reopen) echo "$AFTER" > state ;;
  view)
    state="$INITIAL"; [ ! -f state ] || state=$(cat state)
    if [ "$1" = pr ]; then
      printf '{"number":7,"url":"https://example.com/pr/7","state":"%s","isDraft":false,"title":"t","headRefName":"x","headRefOid":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","baseRefName":"main","mergedAt":null,"labels":[]}' "$state"
    else
      printf '{"iid":7,"web_url":"https://example.com/mr/7","state":"%s","title":"t","source_branch":"x","sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","target_branch":"main","draft":false,"labels":[]}' "$state"
    fi ;;
  *) exit 91 ;;
esac
"##;
            let stub = if provider == "github" {
                StubEnv::new().gh_stub(script)
            } else {
                StubEnv::new().glab_stub(script)
            }
            .env("INITIAL", initial)
            .env("AFTER", after);
            let out = run_forge_cli(
                &stub,
                &[
                    "pr",
                    "reopen",
                    "7",
                    "--expected-head",
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "--provider",
                    provider,
                    "--repo",
                    "acme/widget",
                    "--format",
                    "json",
                ],
            );
            assert_ne!(out.code, 0, "{}", out.stdout);
            assert_eq!(parse_envelope(&out.stdout)["error"]["code"], expected_code);
            assert_eq!(
                std::fs::read_to_string(stub.tempdir.path().join("calls")).unwrap(),
                calls
            );
        }
    }
}

#[test]
fn reopen_reports_head_change_after_mutation() {
    let stub = StubEnv::new().gh_stub(r##"#!/bin/sh
case "$1 $2" in
  "pr reopen") touch reopened ;;
  "pr view")
    head=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; state=CLOSED
    if [ -f reopened ]; then head=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb; state=OPEN; fi
    printf '{"number":7,"url":"https://example.com/pr/7","state":"%s","isDraft":false,"title":"t","headRefName":"x","headRefOid":"%s","baseRefName":"main","mergedAt":null,"labels":[]}' "$state" "$head" ;;
  *) exit 91 ;;
esac
"##);
    let out = run_forge_cli(
        &stub,
        &[
            "pr",
            "reopen",
            "7",
            "--expected-head",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
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
        "pr_head_changed"
    );
    assert!(stub.tempdir.path().join("reopened").exists());
}

use super::support::{StubEnv, run_forge_cli};
use std::fs;

#[test]
fn review_edit_ownership_and_dry_run() {
    for (author, dry, succeeds) in [
        ("alice", false, true),
        ("bob", false, false),
        ("alice", true, true),
    ] {
        let stub = StubEnv::new();
        let body = stub.tempdir.path().join("body.md");
        let calls = stub.tempdir.path().join("calls");
        fs::write(&body, "new body\n").unwrap();
        let script = format!(
            r##"#!/bin/sh
printf '%s\n' "$*" >> '{calls}'
case "$*" in
  *'api user'*) echo '{{"login":"alice"}}' ;;
  *'--method PUT'*) for last; do :; done; cat "$last" >> '{calls}'; echo '{{"id":7,"html_url":"https://github.com/acme/widgets/pull/44#pullrequestreview-7"}}' ;;
  *) echo '{{"id":7,"html_url":"https://github.com/acme/widgets/pull/44#pullrequestreview-7","state":"APPROVED","body":"old body","user":{{"login":"{author}"}}}}' ;;
esac
"##,
            calls = calls.display()
        );
        let stub = stub.gh_stub(&script);
        let mut args = vec![
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
        ];
        if dry {
            args.push("--dry-run");
        }
        args.extend([
            "pr",
            "review",
            "edit",
            "44",
            "--review",
            "7",
            "--body-file",
            body.to_str().unwrap(),
        ]);
        let out = run_forge_cli(&stub, &args);
        assert_eq!(out.code == 0, succeeds, "{} {}", out.stdout, out.stderr);
        let invoked = fs::read_to_string(calls).unwrap_or_default();
        assert_eq!(
            invoked.contains("--method PUT"),
            succeeds && !dry,
            "{invoked}"
        );
        if succeeds && !dry {
            assert!(invoked.contains("new body"), "{invoked}");
            assert!(out.stdout.contains("pullrequestreview-7"));
        }
        if dry {
            assert!(
                out.stdout.contains("old body") && out.stdout.contains("new body"),
                "{}",
                out.stdout
            );
        }
        if !succeeds {
            assert!(out.stdout.contains("author") || out.stderr.contains("author"));
        }
    }
}

#[test]
fn review_edit_gitlab_is_unsupported() {
    let out = run_forge_cli(
        &StubEnv::new(),
        &[
            "--provider",
            "gitlab",
            "--repo",
            "group/project",
            "--format",
            "json",
            "pr",
            "review",
            "edit",
            "44",
            "--review",
            "7",
            "--body-file",
            "unused",
        ],
    );
    assert_ne!(out.code, 0);
    assert!(
        out.stdout.contains("provider_unsupported"),
        "{}",
        out.stdout
    );
}

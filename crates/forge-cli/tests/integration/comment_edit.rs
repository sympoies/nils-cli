use super::support::{StubEnv, parse_envelope, run_forge_cli};
use pretty_assertions::assert_eq;
use std::fs;

const FAKE: &str = r##"#!/usr/bin/python3
import json, os, sys
args = sys.argv[1:]
provider = os.path.basename(sys.argv[0])
if provider == 'gh' and '--method' not in args:
    print(json.dumps({'id': 91, 'issue_url': os.environ.get('FAKE_PARENT', 'https://api.github.com/repos/acme/widgets/issues/7')}))
    sys.exit(0)
if os.environ.get('FAKE_REFUSE'):
    print('HTTP 403: permission denied', file=sys.stderr)
    sys.exit(1)
if provider == 'gh':
    assert 'repos/acme/widgets/issues/comments/91' in args
    body = json.load(open(args[args.index('--input') + 1]))['body']
    print(json.dumps({'id': 91, 'body': body}))
else:
    parent = 'issues' if os.environ['FAKE_KIND'] == 'issue' else 'merge_requests'
    assert f'projects/acme%2Fwidgets/{parent}/7/notes/91' in args, args
    body = json.load(open(args[args.index('--input') + 1]))['body']
    print(json.dumps({'id': 91, 'body': body}))
"##;

#[test]
fn edits_preserve_body_and_provider_refusals() {
    for provider in ["github", "gitlab"] {
        for kind in ["issue", "pr"] {
            let stub = if provider == "github" {
                StubEnv::new().gh_stub(FAKE)
            } else {
                StubEnv::new().glab_stub(FAKE)
            };
            let stub = stub.env("FAKE_KIND", kind);
            let body = "Unicode café\nsecond line\n\n";
            let path = stub.tempdir.path().join("body.md");
            fs::write(&path, body).unwrap();
            let path = path.to_string_lossy().into_owned();
            let args = [
                "--provider",
                provider,
                "--repo",
                "acme/widgets",
                "--format",
                "json",
                kind,
                "comment-edit",
                "7",
                "91",
                "--body-file",
                &path,
            ];
            let out = run_forge_cli(&stub, &args);
            assert_eq!(
                out.code, 0,
                "{provider} {kind}: {} {}",
                out.stdout, out.stderr
            );
            assert_eq!(parse_envelope(&out.stdout)["data"]["body"], body);
            let refused = stub.env("FAKE_REFUSE", "1");
            let out = run_forge_cli(&refused, &args);
            assert_ne!(out.code, 0);
            assert!(
                format!("{}{}", out.stdout, out.stderr).contains("403"),
                "{out:?}"
            );
        }
    }
}

#[test]
fn github_rejects_a_comment_from_another_parent() {
    let stub = StubEnv::new().gh_stub(FAKE).env(
        "FAKE_PARENT",
        "https://api.github.com/repos/acme/widgets/issues/8",
    );
    let path = stub.tempdir.path().join("body.md");
    fs::write(&path, "replacement").unwrap();
    let out = run_forge_cli(
        &stub,
        &[
            "--provider",
            "github",
            "--repo",
            "acme/widgets",
            "--format",
            "json",
            "issue",
            "comment-edit",
            "7",
            "91",
            "--body-file",
            path.to_str().unwrap(),
        ],
    );
    assert_ne!(out.code, 0);
    assert!(
        format!("{}{}", out.stdout, out.stderr).contains("comment_parent_mismatch"),
        "{out:?}"
    );
}

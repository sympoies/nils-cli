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
  "pr view") state=$(cat state); printf '{"number":7,"url":"https://example.com/pr/7","state":"%s","isDraft":false,"title":"t","headRefName":"x","baseRefName":"main","mergeable":"MERGEABLE","mergedAt":null,"labels":[]}' "$state" ;;
  "mr view") state=$(cat state); printf '{"iid":7,"web_url":"https://example.com/mr/7","state":"%s","title":"t","source_branch":"x","target_branch":"main","merge_status":"can_be_merged","draft":false,"labels":[]}' "$state" ;;
  *) exit 91 ;;
esac
"##;
        let stub = if provider == "github" {
            StubEnv::new().gh_stub(script)
        } else {
            StubEnv::new().glab_stub(script)
        };
        for (command, state) in [("close", "closed"), ("reopen", "open")] {
            let out = run_forge_cli(
                &stub,
                &[
                    "pr",
                    command,
                    "7",
                    "--provider",
                    provider,
                    "--repo",
                    "acme/widget",
                    "--format",
                    "json",
                ],
            );
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

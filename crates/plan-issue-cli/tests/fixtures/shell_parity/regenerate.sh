#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "${script_dir}/../../../../.." && pwd -P)"
plan_fixture_rel="crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md"

if [[ -z "${AGENT_HOME:-}" ]]; then
  echo "error: AGENT_HOME is required" >&2
  exit 1
fi

cd "$repo_root"

normalize_paths() {
  sed \
    -e "s|${AGENT_HOME%/}|\$AGENT_HOME|g" \
    -e "s|$HOME/.config/agent-kit|\$AGENT_KIT_HOME|g"
}

run_plan_issue_local() {
  cargo run -q -p nils-plan-issue-cli --bin plan-issue-local -- "$@"
}

run_plan_issue_local --help >"$script_dir/help.txt"

run_plan_issue_local multi-sprint-guide \
  --plan "$plan_fixture_rel" \
  --dry-run \
  | normalize_paths >"$script_dir/multi_sprint_guide_dry_run.txt"

run_plan_issue_local start-sprint \
  --plan "$plan_fixture_rel" \
  --issue 999 \
  --sprint 1 \
  --pr-grouping per-sprint \
  --dry-run \
  | awk '
      /^## Sprint 1 Start$/ {capture=1}
      capture {
        if ($0 == "SPRINT_COMMENT_POSTED=0") exit
        print
      }
    ' >"$script_dir/comment_template_start.md"

echo "updated shell parity fixtures in $script_dir"

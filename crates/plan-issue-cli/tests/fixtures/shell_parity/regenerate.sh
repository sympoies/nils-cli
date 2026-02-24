#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
repo_root="$(cd "${script_dir}/../../../../.." && pwd -P)"
plan_issue_script="${AGENT_HOME:-}/skills/automation/plan-issue-delivery-loop/scripts/plan-issue-delivery-loop.sh"
plan_fixture_rel="crates/plan-tooling/tests/fixtures/split_prs/duck-plan.md"

if [[ -z "${AGENT_HOME:-}" ]]; then
  echo "error: AGENT_HOME is required" >&2
  exit 1
fi
if [[ ! -x "$plan_issue_script" ]]; then
  echo "error: missing executable: $plan_issue_script" >&2
  exit 1
fi

cd "$repo_root"

normalize_paths() {
  sed \
    -e "s|${AGENT_HOME%/}|\$AGENT_HOME|g" \
    -e "s|$HOME/.config/agent-kit|\$AGENT_KIT_HOME|g"
}

bash "$plan_issue_script" --help >"$script_dir/help.txt"

bash "$plan_issue_script" multi-sprint-guide \
  --plan "$plan_fixture_rel" \
  --dry-run \
  | normalize_paths >"$script_dir/multi_sprint_guide_dry_run.txt"

bash "$plan_issue_script" start-sprint \
  --plan "$plan_fixture_rel" \
  --issue DRY_RUN_PLAN_ISSUE \
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

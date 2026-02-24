#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
plan_issue_wrapper="${script_dir}/plan-issue"

if [[ -x "$plan_issue_wrapper" ]]; then
  exec "$plan_issue_wrapper" "$@"
fi

if command -v plan-issue >/dev/null 2>&1; then
  exec plan-issue "$@"
fi

if command -v cargo >/dev/null 2>&1; then
  exec cargo run -q -p nils-plan-issue-cli -- "$@"
fi

echo "plan-issue-delivery-loop.sh: plan-issue binary not found (install or build nils-plan-issue-cli)" >&2
exit 1

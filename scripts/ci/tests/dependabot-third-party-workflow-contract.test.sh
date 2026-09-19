#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi
cd "$repo_root"

workflow=.github/workflows/dependabot-third-party-apply.yml
expected_guard='if [[ "${author}" != "dependabot[bot]" && "${author}" != "app/dependabot" ]]; then'
guard_count="$(rg -F -c -- "$expected_guard" "$workflow" || true)"
guard_count="${guard_count:-0}"

if [[ "$guard_count" != "2" ]]; then
  echo "FAIL: expected both privileged Dependabot paths to accept bot and app author identities" >&2
  echo "  found $guard_count matching guards in $workflow; expected 2" >&2
  exit 1
fi

echo "PASS: dependabot-third-party-workflow-contract.test.sh"

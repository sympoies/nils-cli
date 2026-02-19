#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

cd "$repo_root"

if rg -n "codex_cli::" crates/agent-provider-codex/src; then
  echo "FAIL: provider source must not import codex_cli runtime internals" >&2
  exit 1
fi

if rg -n '^codex-cli\s*=' crates/agent-provider-codex/Cargo.toml; then
  echo "FAIL: provider Cargo.toml must not depend on codex-cli" >&2
  exit 1
fi

echo "PASS: codex-core dependency boundary is intact"

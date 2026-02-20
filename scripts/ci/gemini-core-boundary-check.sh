#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

cd "$repo_root"

if rg -n "gemini_cli::" crates/agent-provider-gemini/src; then
  echo "FAIL: provider source must not import gemini_cli runtime internals" >&2
  exit 1
fi

if rg -n "(^gemini-cli\\s*=)|nils-gemini-cli" crates/agent-provider-gemini/Cargo.toml; then
  echo "FAIL: provider Cargo.toml must not depend on gemini-cli package" >&2
  exit 1
fi

echo "PASS: gemini-core dependency boundary is intact"

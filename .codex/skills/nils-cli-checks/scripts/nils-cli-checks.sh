#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  nils-cli-checks.sh [--help]

Runs the required pre-delivery checks from DEVELOPMENT.md:
  - cargo fmt --all -- --check
  - cargo clippy --all-targets --all-features -- -D warnings
  - cargo test --workspace
  - zsh -f tests/zsh/completion.test.zsh

Exit codes:
  0  all checks passed
  1  a check failed
  2  usage error or missing prerequisites
USAGE
}

while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: ${1:-}" >&2
      usage >&2
      exit 2
      ;;
  esac
done

for cmd in git cargo zsh; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: missing required tool on PATH: $cmd" >&2
    exit 2
  fi
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

cd "$repo_root"

run() {
  local -a cmd=( "$@" )
  echo "+ ${cmd[*]}"
  if "${cmd[@]}"; then
    return 0
  else
    local code=$?
    echo "error: check failed (exit $code): ${cmd[*]}" >&2
    exit 1
  fi
}

run cargo fmt --all -- --check
run cargo clippy --all-targets --all-features -- -D warnings
run cargo test --workspace
run zsh -f tests/zsh/completion.test.zsh

echo "ok: all nils-cli checks passed"

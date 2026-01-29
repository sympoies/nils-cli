#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  nils-cli-checks.sh [--help]

Runs the required lint + test commands from DEVELOPMENT.md:
  - cargo fmt --all -- --check
  - cargo clippy --all-targets --all-features -- -D warnings
  - cargo test --workspace
  - zsh -f tests/zsh/completion.test.zsh
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
    echo "error: missing required command on PATH: $cmd" >&2
    exit 2
  fi
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

cd "$repo_root"

run_step() {
  local name="$1"
  shift

  echo "==> $name"
  set +e
  "$@"
  local exit_code=$?
  set -e

  if [[ $exit_code -ne 0 ]]; then
    echo "error: failed ($exit_code): $*" >&2
    exit "$exit_code"
  fi
}

run_step "fmt" cargo fmt --all -- --check
run_step "clippy" cargo clippy --all-targets --all-features -- -D warnings
run_step "test" cargo test --workspace
run_step "zsh completion test" zsh -f tests/zsh/completion.test.zsh

echo "ok: all checks passed"

#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/nils-cli-checks-entrypoint.sh [--xvfb] [--with-coverage] [verify-script args...]

Description:
  Canonical cross-platform entrypoint for CI verification jobs.
  - Runs ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
  - Reuses the current Bash interpreter so nested audit scripts run with the same shell version.
  - Optionally wraps execution with xvfb-run for Linux runners.
  - Optionally runs the local coverage gate and summary after required checks.

Options:
  --xvfb             Run checks under `xvfb-run -a`
  --with-coverage    Run coverage gate after required checks:
                     cargo llvm-cov nextest --profile ci --workspace --lcov \
                       --output-path target/coverage/lcov.info --fail-under-lines <N>
                     bash scripts/ci/coverage-summary.sh target/coverage/lcov.info
                     cargo test --workspace --doc
  -h, --help         Show this help

Environment:
  NILS_CLI_COVERAGE_FAIL_UNDER_LINES
    Override coverage threshold used with --with-coverage (default: 85).
USAGE
}

use_xvfb=0
with_coverage=0
docs_only=0
declare -a verify_args=()
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --xvfb)
      use_xvfb=1
      shift
      ;;
    --with-coverage)
      with_coverage=1
      shift
      ;;
    --docs-only)
      docs_only=1
      verify_args+=("--docs-only")
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      verify_args+=("${1:-}")
      shift
      ;;
  esac
done

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
cd "$repo_root"

verify_script="./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh"
if [[ ! -f "$verify_script" ]]; then
  echo "error: missing required checks script: $verify_script" >&2
  exit 2
fi

bash_bin="${BASH:-}"
if [[ -z "$bash_bin" || ! -x "$bash_bin" ]]; then
  bash_bin="$(command -v bash || true)"
fi
if [[ -z "$bash_bin" || ! -x "$bash_bin" ]]; then
  echo "error: bash not found on PATH" >&2
  exit 2
fi

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

declare -a cmd=( "$bash_bin" "$verify_script" "${verify_args[@]}" )
if [[ "$use_xvfb" -eq 1 ]]; then
  if ! command -v xvfb-run >/dev/null 2>&1; then
    echo "error: --xvfb requested but xvfb-run is not available on PATH" >&2
    exit 2
  fi
  cmd=(xvfb-run -a "${cmd[@]}")
fi

run "${cmd[@]}"

if [[ "$with_coverage" -eq 0 ]]; then
  exit 0
fi

if [[ "$docs_only" -eq 1 ]]; then
  echo "error: --with-coverage cannot be used with --docs-only" >&2
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required for --with-coverage" >&2
  exit 2
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "error: cargo-llvm-cov is required for --with-coverage" >&2
  exit 2
fi

coverage_fail_under="${NILS_CLI_COVERAGE_FAIL_UNDER_LINES:-85}"

run mkdir -p target/coverage
run cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines "$coverage_fail_under"
run bash scripts/ci/coverage-summary.sh target/coverage/lcov.info
run cargo test --workspace --doc

echo "ok: required checks + coverage gate passed"

#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/nils-cli-checks-entrypoint.sh [--xvfb] [verify-script args...]

Description:
  Canonical cross-platform entrypoint for CI verification jobs.
  - Runs ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
  - Reuses the current Bash interpreter so nested audit scripts run with the same shell version.
  - Optionally wraps execution with xvfb-run for Linux runners.

Options:
  --xvfb      Run checks under `xvfb-run -a`
  -h, --help  Show this help
USAGE
}

use_xvfb=0
declare -a verify_args=()
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --xvfb)
      use_xvfb=1
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

declare -a cmd=( "$bash_bin" "$verify_script" "${verify_args[@]}" )
if [[ "$use_xvfb" -eq 1 ]]; then
  if ! command -v xvfb-run >/dev/null 2>&1; then
    echo "error: --xvfb requested but xvfb-run is not available on PATH" >&2
    exit 2
  fi
  cmd=(xvfb-run -a "${cmd[@]}")
fi

echo "+ ${cmd[*]}"
"${cmd[@]}"

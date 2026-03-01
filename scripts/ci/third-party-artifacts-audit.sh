#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/third-party-artifacts-audit.sh [--strict]

Checks third-party artifact freshness and required file presence:
  - validates required artifacts exist
  - runs scripts/generate-third-party-artifacts.sh --check
  - reports drift diagnostics in PASS/WARN/FAIL style

Options:
  --strict   Treat drift/missing artifacts as hard failures (exit 1)
  -h, --help Show this help
USAGE
}

strict=0
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --strict)
      strict=1
      shift
      ;;
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

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
cd "$repo_root"

generator_script="scripts/generate-third-party-artifacts.sh"
required_artifacts=("THIRD_PARTY_LICENSES.md" "THIRD_PARTY_NOTICES.md")

if [[ ! -f "$generator_script" ]]; then
  echo "error: missing generator script: $generator_script" >&2
  exit 2
fi

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/third-party-artifacts-audit.XXXXXX")"
audit_log="${tmp_dir}/generator-check.log"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

severity_prefix="WARN"
exit_code=0
if [[ "$strict" -eq 1 ]]; then
  severity_prefix="FAIL"
  exit_code=1
fi

missing_count=0
for artifact in "${required_artifacts[@]}"; do
  if [[ ! -f "$artifact" ]]; then
    echo "${severity_prefix}: missing required artifact: $artifact"
    missing_count=$((missing_count + 1))
  fi
done

set +e
bash "$generator_script" --check >"$audit_log" 2>&1
generator_exit=$?
set -e

if [[ "$generator_exit" -ne 0 && "$generator_exit" -ne 1 ]]; then
  echo "error: generator check failed unexpectedly (exit ${generator_exit}): bash $generator_script --check" >&2
  cat "$audit_log" >&2
  exit 2
fi

drift_detected=0
if [[ "$generator_exit" -eq 1 ]]; then
  drift_detected=1
fi

if (( missing_count == 0 && drift_detected == 0 )); then
  echo "PASS: third-party artifact audit (strict=${strict}, drift=0, missing=0)"
  exit 0
fi

if [[ -s "$audit_log" ]]; then
  while IFS= read -r line; do
    if [[ "$line" == FAIL:* ]]; then
      echo "${severity_prefix}:${line#FAIL:}"
    elif [[ "$line" == PASS:* ]]; then
      echo "INFO:${line#PASS:}"
    else
      echo "INFO: $line"
    fi
  done <"$audit_log"
fi

echo "${severity_prefix}: third-party artifact audit (strict=${strict}, drift=${drift_detected}, missing=${missing_count})"
exit "$exit_code"

#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/test-stale-audit.sh [--strict]

Runs workspace stale-test inventory and detects stale-test regressions:
  - new orphaned helper candidates (`signal=helper_fanout`, `proposed_action=remove`)
  - deprecated-path leftovers (`signal=deprecated_path_marker`)

Policy baseline:
  scripts/ci/test-stale-audit-baseline.tsv

Options:
  --strict   Treat regressions as hard failures (exit 1)
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

if ! command -v rg >/dev/null 2>&1; then
  echo "error: ripgrep (rg) is required" >&2
  exit 2
fi

if [[ -z "${AGENT_HOME:-}" ]]; then
  if [[ -z "${HOME:-}" ]]; then
    echo "error: AGENT_HOME is not set and HOME is unavailable" >&2
    exit 2
  fi
  AGENT_HOME="${HOME}/.agents"
fi
export AGENT_HOME

baseline_file="scripts/ci/test-stale-audit-baseline.tsv"
if [[ ! -f "$baseline_file" ]]; then
  echo "error: missing baseline file: $baseline_file" >&2
  exit 2
fi

audit_root="${AGENT_HOME}/out/workspace-test-cleanup"
inventory_file="${audit_root}/stale-tests.ci.tsv"
mkdir -p "$audit_root"

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/test-stale-audit.XXXXXX")"
current_orphans="${tmp_dir}/current-orphans.tsv"
baseline_orphans="${tmp_dir}/baseline-orphans.tsv"
new_orphans="${tmp_dir}/new-orphans.tsv"
deprecated_leftovers="${tmp_dir}/deprecated-leftovers.tsv"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

bash scripts/dev/workspace-test-stale-audit.sh --out "$inventory_file" >/dev/null

awk -F '\t' '
  NR > 1 && $5 == "helper_fanout" && $6 == "remove" {
    print $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6
  }
' "$inventory_file" | LC_ALL=C sort -u >"$current_orphans"

awk -F '\t' '
  NR > 1 {
    print $1 "\t" $2 "\t" $3 "\t" $4 "\t" $5
  }
' "$baseline_file" | LC_ALL=C sort -u >"$baseline_orphans"

comm -23 "$current_orphans" "$baseline_orphans" >"$new_orphans"

awk -F '\t' '
  NR > 1 && $5 == "deprecated_path_marker" {
    print $2 "\t" $3 "\t" $4 "\t" $5 "\t" $6
  }
' "$inventory_file" | LC_ALL=C sort -u >"$deprecated_leftovers"

current_count="$(wc -l <"$current_orphans" | tr -d ' ')"
baseline_count="$(wc -l <"$baseline_orphans" | tr -d ' ')"
new_count="$(wc -l <"$new_orphans" | tr -d ' ')"
deprecated_count="$(wc -l <"$deprecated_leftovers" | tr -d ' ')"

echo "INFO: stale-test audit inventory refreshed at $inventory_file"
echo "INFO: orphaned helper candidates current=$current_count baseline=$baseline_count new=$new_count"
echo "INFO: deprecated-path leftovers=$deprecated_count"

report_regressions() {
  local prefix="$1"

  while IFS=$'\t' read -r crate rel_path symbol signal action; do
    [[ -z "$crate" ]] && continue
    echo "${prefix}: stale-test regression type=orphaned-helper crate=${crate} path=${rel_path} symbol=${symbol} signal=${signal} action=${action}"
  done <"$new_orphans"

  while IFS=$'\t' read -r crate rel_path symbol signal action; do
    [[ -z "$crate" ]] && continue
    echo "${prefix}: stale-test regression type=deprecated-path-leftover crate=${crate} path=${rel_path} symbol=${symbol} signal=${signal} action=${action}"
  done <"$deprecated_leftovers"
}

if (( new_count > 0 || deprecated_count > 0 )); then
  if [[ "$strict" -eq 1 ]]; then
    report_regressions "FAIL"
    echo "FAIL: stale-test audit (strict=$strict, regressions=$((new_count + deprecated_count)), new_orphans=$new_count, deprecated_leftovers=$deprecated_count)"
    exit 1
  fi

  report_regressions "WARN"
  echo "WARN: stale-test audit (strict=$strict, regressions=$((new_count + deprecated_count)), new_orphans=$new_count, deprecated_leftovers=$deprecated_count)"
  exit 0
fi

echo "PASS: stale-test audit (strict=$strict, current_orphans=$current_count, baseline_orphans=$baseline_count, new_orphans=0, deprecated_leftovers=0)"

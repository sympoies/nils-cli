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
frozen_baseline_allowlist="${tmp_dir}/frozen-baseline-allowlist.tsv"
invalid_baseline_rows="${tmp_dir}/invalid-baseline-rows.tsv"
unexpected_baseline_rows="${tmp_dir}/unexpected-baseline-rows.tsv"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

write_frozen_baseline_allowlist() {
  cat >"$frozen_baseline_allowlist" <<'EOF'
agent-docs	crates/agent-docs/tests/common.rs	drop (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_commit_hash_missing_ref_errors (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_commit_hash_outputs_sha_for_head (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_commit_hash_resolves_annotated_tag (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_both_outputs_diff_and_status (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_help_prints_usage (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_no_changes_warns_and_exits_1 (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_rejects_conflicting_modes (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_rejects_unknown_arg (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_copy_staged_stdout_outputs_diff_only (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_root_not_in_repo_errors (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_root_prints_message (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_root_shell_outputs_cd_command (fanout=0)	helper_fanout	remove
git-cli	crates/git-cli/tests/utils.rs	utils_zip_creates_backup_zip (fanout=0)	helper_fanout	remove
EOF
  LC_ALL=C sort -u -o "$frozen_baseline_allowlist" "$frozen_baseline_allowlist"
}

validate_baseline_file() {
  local expected_header
  expected_header=$'crate\tpath\tsymbol_or_test\tsignal\tproposed_action'

  if [[ "$(sed -n '1p' "$baseline_file")" != "$expected_header" ]]; then
    echo "error: invalid baseline header in $baseline_file" >&2
    exit 2
  fi

  awk -F '\t' '
    NR > 1 && (NF != 5 || $4 != "helper_fanout" || $5 != "remove") {print $0}
  ' "$baseline_file" | LC_ALL=C sort -u >"$invalid_baseline_rows"

  if [[ -s "$invalid_baseline_rows" ]]; then
    echo "FAIL: baseline contains unsupported stale-test actions (only helper_fanout/remove rows are allowed)." >&2
    cat "$invalid_baseline_rows" >&2
    exit 1
  fi
}

write_frozen_baseline_allowlist
validate_baseline_file

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

comm -23 "$baseline_orphans" "$frozen_baseline_allowlist" >"$unexpected_baseline_rows"
if [[ -s "$unexpected_baseline_rows" ]]; then
  echo "FAIL: baseline contains entries outside the frozen S3T1 allowlist; expand rules in docs/runbooks/test-cleanup-governance.md and docs/specs/workspace-test-cleanup-lane-matrix-v1.md before changing baseline." >&2
  cat "$unexpected_baseline_rows" >&2
  exit 1
fi

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

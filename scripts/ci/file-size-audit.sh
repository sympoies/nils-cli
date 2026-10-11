#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/file-size-audit.sh [--strict] [--update-baseline] [--base <ref>]

Runs the per-file size ratchet over tracked crates/**/*.rs files. Limits:
  - implementation lines: 2,000 per file
  - test lines: 3,000 per file

Regressions:
  - a file over a limit without a baseline row (new-over-limit-file)
  - a baselined file that grew (baselined-file-grew)
  - a baseline row that is stale: the file is gone, is now within the limit,
    or shrank (stale-baseline-row); refresh with --update-baseline
  - a per-kind baseline total larger than at the merge base with <ref>
    (baseline-total-increased); when the change set modifies the measurement
    helper, the combined impl+test total is compared instead

Baseline:
  scripts/ci/file-size-baseline.tsv  (path<TAB>kind<TAB>lines; kind is impl or test)

Options:
  --strict              Treat regressions as hard failures (exit 1)
  --update-baseline     Rewrite the baseline from the current over-limit files
  --base <ref>          Base ref for the baseline total check
                        (default: origin/main)
  -h, --help            Show this help
USAGE
}

strict=0
update=0
base_ref="origin/main"
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --strict)
      strict=1
      shift
      ;;
    --update-baseline)
      update=1
      shift
      ;;
    --base)
      if [[ $# -lt 2 ]]; then
        echo "error: --base requires a value" >&2
        exit 2
      fi
      base_ref="${2:-}"
      shift 2
      ;;
    --base=*)
      base_ref="${1#--base=}"
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

impl_limit=2000
test_limit=3000

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi
cd "$repo_root"

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 is required" >&2
  exit 2
fi

helper="scripts/ci/lib/rust_file_size.py"
baseline_file="scripts/ci/file-size-baseline.tsv"
header=$'path\tkind\tlines'

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/file-size-audit.XXXXXX")"
tracked="${tmp_dir}/tracked.txt"
measured="${tmp_dir}/measured.tsv"
over="${tmp_dir}/over-limit.tsv"
baseline="${tmp_dir}/baseline.tsv"
invalid_rows="${tmp_dir}/invalid-rows.tsv"
regressions="${tmp_dir}/regressions.txt"

cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT

git ls-files -- 'crates/*.rs' | LC_ALL=C sort >"$tracked"
python3 "$helper" <"$tracked" | LC_ALL=C sort >"$measured"

awk -F '\t' -v impl_limit="$impl_limit" -v test_limit="$test_limit" '
  ($2 == "impl" && $3 + 0 > impl_limit) || ($2 == "test" && $3 + 0 > test_limit) { print }
' "$measured" | LC_ALL=C sort >"$over"

if [[ "$update" -eq 1 ]]; then
  {
    printf '%s\n' "$header"
    cat "$over"
  } >"$baseline_file"
  echo "INFO: wrote $baseline_file ($(wc -l <"$over" | tr -d ' ') over-limit rows)"
  exit 0
fi

if [[ ! -f "$baseline_file" ]]; then
  echo "error: missing baseline file: $baseline_file" >&2
  exit 2
fi

if [[ "$(sed -n '1p' "$baseline_file")" != "$header" ]]; then
  echo "error: invalid baseline header in $baseline_file" >&2
  exit 2
fi

tail -n +2 "$baseline_file" | LC_ALL=C sort >"$baseline"

awk -F '\t' '
  NF != 3 || ($2 != "impl" && $2 != "test") || $3 !~ /^[0-9]+$/ || $3 + 0 == 0 { print }
' "$baseline" >"$invalid_rows"
if [[ -s "$invalid_rows" ]]; then
  echo "error: invalid baseline rows in $baseline_file (expected path, impl|test, positive lines):" >&2
  cat "$invalid_rows" >&2
  exit 2
fi

duplicate_rows="$(cut -f1,2 "$baseline" | uniq -d)"
if [[ -n "$duplicate_rows" ]]; then
  echo "error: duplicate baseline rows in $baseline_file:" >&2
  echo "$duplicate_rows" >&2
  exit 2
fi

base_resolves=1
if ! git rev-parse --verify --quiet "${base_ref}^{commit}" >/dev/null; then
  base_resolves=0
  if [[ "$strict" -eq 1 ]]; then
    echo "error: base ref does not resolve to a commit: $base_ref (pass --base <ref>)" >&2
    exit 2
  fi
  echo "WARN: base ref does not resolve to a commit: $base_ref; skipping baseline total check"
fi

awk -F '\t' -v impl_limit="$impl_limit" -v test_limit="$test_limit" '
  function limit_for(kind) {
    return kind == "impl" ? impl_limit : test_limit
  }
  FILENAME == ARGV[1] {
    exists[$0] = 1
    next
  }
  FILENAME == ARGV[2] {
    current[$1 SUBSEP $2] = $3 + 0
    if (($2 == "impl" && $3 + 0 > impl_limit) || ($2 == "test" && $3 + 0 > test_limit)) {
      over[$1 SUBSEP $2] = $3 + 0
    }
    next
  }
  FILENAME == ARGV[3] {
    rows[$1 SUBSEP $2] = $3 + 0
    next
  }
  END {
    for (key in over) {
      if (key in rows) {
        continue
      }
      split(key, part, SUBSEP)
      printf "type=new-over-limit-file path=%s kind=%s baseline=none current=%d limit=%d\n", \
        part[1], part[2], over[key], limit_for(part[2])
    }
    for (key in rows) {
      split(key, part, SUBSEP)
      cur = (key in current) ? current[key] : 0
      base = rows[key]
      lim = limit_for(part[2])
      if (cur > base) {
        printf "type=baselined-file-grew path=%s kind=%s baseline=%d current=%d limit=%d\n", \
          part[1], part[2], base, cur, lim
      } else if (cur < base || cur <= lim) {
        reason = !(part[1] in exists) ? "gone" : (cur <= lim ? "within-limit" : "shrunk")
        printf "type=stale-baseline-row path=%s kind=%s baseline=%d current=%d limit=%d reason=%s\n", \
          part[1], part[2], base, cur, lim, reason
      }
    }
  }
' "$tracked" "$measured" "$baseline" >"$regressions"

if [[ "$base_resolves" -eq 1 ]]; then
  merge_base="$(git merge-base HEAD "$base_ref")"
  if git cat-file -e "${merge_base}:${baseline_file}" 2>/dev/null; then
    base_baseline="${tmp_dir}/base-baseline.tsv"
    git show "${merge_base}:${baseline_file}" >"$base_baseline"
    base_sums="$(awk -F '\t' 'NR > 1 { sum[$2] += $3 } END { print sum["impl"] + 0, sum["test"] + 0 }' "$base_baseline")"
    current_sums="$(awk -F '\t' 'NR > 1 { sum[$2] += $3 } END { print sum["impl"] + 0, sum["test"] + 0 }' "$baseline_file")"
    read -r base_impl base_test <<<"$base_sums"
    read -r current_impl current_test <<<"$current_sums"
    if ! git diff --quiet "$merge_base" -- "$helper"; then
      # A measurement-rule change may move lines between kinds; the combined
      # total still must not increase.
      base_all=$((base_impl + base_test))
      current_all=$((current_impl + current_test))
      if (( current_all > base_all )); then
        echo "type=baseline-total-increased kind=combined base=${base_all} current=${current_all}" >>"$regressions"
      fi
    else
      if (( current_impl > base_impl )); then
        echo "type=baseline-total-increased kind=impl base=${base_impl} current=${current_impl}" >>"$regressions"
      fi
      if (( current_test > base_test )); then
        echo "type=baseline-total-increased kind=test base=${base_test} current=${current_test}" >>"$regressions"
      fi
    fi
  else
    echo "INFO: no $baseline_file at merge base $merge_base; skipping baseline total check"
  fi
fi

over_count="$(wc -l <"$over" | tr -d ' ')"
baseline_count="$(wc -l <"$baseline" | tr -d ' ')"
regression_count="$(wc -l <"$regressions" | tr -d ' ')"
echo "INFO: file-size over-limit files=${over_count} baseline_rows=${baseline_count} (impl>${impl_limit}, test>${test_limit})"

if (( regression_count > 0 )); then
  prefix="WARN"
  if [[ "$strict" -eq 1 ]]; then
    prefix="FAIL"
  fi
  LC_ALL=C sort "$regressions" | while IFS= read -r line; do
    echo "${prefix}: file-size regression ${line}"
  done
  echo "${prefix}: file-size audit (strict=${strict}, regressions=${regression_count})"
  if [[ "$strict" -eq 1 ]]; then
    exit 1
  fi
  exit 0
fi

echo "PASS: file-size audit (strict=${strict}, over_limit=${over_count}, baseline_rows=${baseline_count}, regressions=0)"

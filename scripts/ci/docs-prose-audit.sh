#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/ci/docs-prose-audit.sh [--strict] [--update-baseline] [--base <ref>]

Checks tracked Markdown prose runs longer than 12 lines. Fenced code, blank
lines, table rows, headings, and HTML comment lines end a run; list items start
new runs and their continuation lines remain part of the item.

Options:
  --strict              Treat regressions as hard failures (exit 1)
  --update-baseline     Rewrite the baseline from current over-limit files
  --base <ref>          Base ref for monotonic baseline check (default: origin/main)
  -h, --help            Show this help
USAGE
}

strict=0
update=0
base_ref="origin/main"
while [[ $# -gt 0 ]]; do
  case "${1:-}" in
    --strict) strict=1; shift ;;
    --update-baseline) update=1; shift ;;
    --base)
      if [[ $# -lt 2 ]]; then
        echo "error: --base requires a value" >&2
        exit 2
      fi
      base_ref="${2:-}"
      shift 2
      ;;
    --base=*) base_ref="${1#--base=}"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "error: unknown argument: ${1:-}" >&2; usage >&2; exit 2 ;;
  esac
done

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

helper="scripts/ci/lib/markdown_prose.py"
baseline_file="scripts/ci/docs-prose-baseline.tsv"
header=$'path\truns_over_limit\tlongest_run'
limit=12
tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/docs-prose-audit.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
tracked="$tmp_dir/tracked.txt"
measured="$tmp_dir/measured.tsv"
baseline="$tmp_dir/baseline.tsv"
regressions="$tmp_dir/regressions.txt"

git ls-files -- '*.md' \
  ':!:docs/devlog/**' \
  ':!:THIRD_PARTY_LICENSES.md' \
  ':!:THIRD_PARTY_NOTICES.md' | while IFS= read -r path; do
    [[ -f "$path" ]] && printf '%s\n' "$path"
  done | LC_ALL=C sort >"$tracked"
python3 "$helper" <"$tracked" | LC_ALL=C sort >"$measured"

if [[ "$update" -eq 1 ]]; then
  {
    printf '%s\n' "$header"
    cat "$measured"
  } >"$baseline_file"
  echo "INFO: wrote $baseline_file ($(wc -l <"$measured" | tr -d ' ') over-limit rows)"
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
  NF != 3 || $2 !~ /^[0-9]+$/ || $2 + 0 == 0 || $3 !~ /^[0-9]+$/ || $3 + 0 <= 12 { print }
' "$baseline" >"$tmp_dir/invalid-rows.tsv"
if [[ -s "$tmp_dir/invalid-rows.tsv" ]]; then
  echo "error: invalid baseline rows in $baseline_file (expected path, positive runs_over_limit, longest_run > 12):" >&2
  cat "$tmp_dir/invalid-rows.tsv" >&2
  exit 2
fi
duplicates="$(cut -f1 "$baseline" | uniq -d)"
if [[ -n "$duplicates" ]]; then
  echo "error: duplicate baseline paths in $baseline_file:" >&2
  echo "$duplicates" >&2
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

awk -F '\t' '
  FILENAME == ARGV[1] { exists[$0] = 1; next }
  FILENAME == ARGV[2] {
    current[$1] = $2 + 0
    longest[$1] = $3 + 0
    next
  }
  FILENAME == ARGV[3] {
    rows[$1] = $2 + 0
    row_longest[$1] = $3 + 0
    next
  }
  END {
    for (path in current) {
      if (!(path in rows)) {
        printf "type=new-over-limit-file path=%s baseline=none current_runs=%d current_longest=%d limit=12\n", path, current[path], longest[path]
      } else if (current[path] > rows[path] || longest[path] > row_longest[path]) {
        printf "type=baselined-file-grew path=%s baseline_runs=%d current_runs=%d baseline_longest=%d current_longest=%d\n", path, rows[path], current[path], row_longest[path], longest[path]
      }
    }
    for (path in rows) {
      if (!(path in current)) {
        reason = !(path in exists) ? "gone" : "improved"
        printf "type=stale-baseline-row path=%s baseline_runs=%d baseline_longest=%d reason=%s\n", path, rows[path], row_longest[path], reason
      } else if (current[path] < rows[path] || longest[path] < row_longest[path]) {
        printf "type=stale-baseline-row path=%s baseline_runs=%d current_runs=%d baseline_longest=%d current_longest=%d reason=improved\n", path, rows[path], current[path], row_longest[path], longest[path]
      }
    }
  }
' "$tracked" "$measured" "$baseline" >"$regressions"

if [[ "$base_resolves" -eq 1 ]]; then
  merge_base="$(git merge-base HEAD "$base_ref")"
  if git cat-file -e "${merge_base}:${baseline_file}" 2>/dev/null; then
    base_baseline="$tmp_dir/base-baseline.tsv"
    git show "${merge_base}:${baseline_file}" >"$base_baseline"
    base_total="$(awk -F '\t' 'NR > 1 { sum += $2 } END { print sum + 0 }' "$base_baseline")"
    current_total="$(awk -F '\t' 'NR > 1 { sum += $2 } END { print sum + 0 }' "$baseline_file")"
    if (( current_total > base_total )); then
      echo "type=baseline-total-increased base=$base_total current=$current_total" >>"$regressions"
    fi
  else
    echo "INFO: no $baseline_file at merge base $merge_base; skipping baseline total check"
  fi
fi

over_count="$(wc -l <"$measured" | tr -d ' ')"
baseline_total="$(awk -F '\t' 'NR > 1 { sum += $2 } END { print sum + 0 }' "$baseline_file")"
regression_count="$(wc -l <"$regressions" | tr -d ' ')"
echo "INFO: docs-prose over-limit files=$over_count runs=$baseline_total (limit=$limit)"
if (( regression_count > 0 )); then
  prefix="WARN"
  if [[ "$strict" -eq 1 ]]; then prefix="FAIL"; fi
  LC_ALL=C sort "$regressions" | while IFS= read -r line; do
    echo "$prefix: docs-prose regression $line"
  done
  echo "$prefix: docs-prose audit (strict=$strict, regressions=$regression_count)"
  if [[ "$strict" -eq 1 ]]; then exit 1; fi
  exit 0
fi
echo "PASS: docs-prose audit"

#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside a git work tree" >&2
  exit 2
fi

script="$repo_root/scripts/ci/docs-prose-audit.sh"
helper="$repo_root/scripts/ci/lib/markdown_prose.py"
for required in "$script" "$helper"; do
  if [[ ! -f "$required" ]]; then
    echo "FAIL: missing required file: $required" >&2
    exit 1
  fi
done

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/docs-prose-audit-test.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
T=$'\t'
output=""
status=0
base_ref=HEAD

fail() {
  echo "FAIL: $*" >&2
  echo "$output" >&2
  exit 1
}

make_repo() {
  local dir="$tmp_dir/$1"
  mkdir -p "$dir/scripts/ci/lib"
  cp "$script" "$dir/scripts/ci/docs-prose-audit.sh"
  cp "$helper" "$dir/scripts/ci/lib/markdown_prose.py"
  git init -q "$dir"
  printf 'path\truns_over_limit\tlongest_run\n' >"$dir/scripts/ci/docs-prose-baseline.tsv"
  printf '# Fixture\n' >"$dir/README.md"
  git -C "$dir" add -A
  git -C "$dir" -c user.name=fixture -c user.email=fixture@example.invalid \
    -c commit.gpgsign=false commit -q -m base
  printf '%s' "$dir"
}

commit_all() {
  git -C "$1" add -A
  git -C "$1" -c user.name=fixture -c user.email=fixture@example.invalid \
    -c commit.gpgsign=false commit -q -m "$2"
}

run_audit() {
  local dir="$1"
  shift
  set +e
  output="$(cd "$dir" && bash scripts/ci/docs-prose-audit.sh --base "$base_ref" "$@" 2>&1)"
  status=$?
  set -e
}

expect_status() {
  [[ "$status" -eq "$2" ]] || fail "$1: exit $status, expected $2"
}

expect_contains() {
  grep -qF -- "$2" <<<"$output" || fail "$1: missing '$2'"
}

gen_lines() {
  awk -v count="$1" 'BEGIN { for (i = 1; i <= count; i++) print "Prose line " i "." }'
}

case_new_long_paragraph_fails() {
  echo '== new long paragraph fails =='
  local dir
  dir="$(make_repo new-paragraph)"
  gen_lines 13 >"$dir/docs.md"
  git -C "$dir" add docs.md
  run_audit "$dir" --strict
  expect_status 'new long paragraph' 1
  expect_contains 'new file finding' 'type=new-over-limit-file path=docs.md'
  echo ok
}

case_excluded_structures_pass() {
  echo '== long table and fenced block pass; short list items pass =='
  local dir
  dir="$(make_repo excluded-structures)"
  {
    for _ in {1..20}; do printf '| cell | cell |\n'; done
    printf '\n```text\n'
    gen_lines 20
    printf '```\n\n'
    for i in {1..20}; do printf -- '- short item %s\n' "$i"; done
  } >"$dir/docs.md"
  git -C "$dir" add docs.md
  run_audit "$dir" --strict
  expect_status 'excluded structures' 0
  expect_contains 'pass marker' 'PASS: docs-prose audit'
  echo ok
}

case_long_list_item_fails() {
  echo '== one list item with long continuation fails =='
  local dir
  dir="$(make_repo long-list-item)"
  {
    printf -- '- item starts here\n'
    gen_lines 12
  } >"$dir/docs.md"
  git -C "$dir" add docs.md
  run_audit "$dir" --strict
  expect_status 'long list item' 1
  expect_contains 'list finding' 'type=new-over-limit-file path=docs.md'
  echo ok
}

case_growth_and_improvement_require_refresh() {
  echo '== baseline growth and improvement require update =='
  local dir
  dir="$(make_repo baseline-changes)"
  gen_lines 14 >"$dir/docs.md"
  printf 'docs.md\t1\t14\n' >>"$dir/scripts/ci/docs-prose-baseline.tsv"
  commit_all "$dir" baseline
  gen_lines 15 >"$dir/docs.md"
  run_audit "$dir" --strict
  expect_status 'growth' 1
  expect_contains 'growth finding' 'type=baselined-file-grew path=docs.md'

  gen_lines 13 >"$dir/docs.md"
  run_audit "$dir" --strict
  expect_status 'improvement before update' 1
  expect_contains 'stale finding' 'type=stale-baseline-row path=docs.md'

  run_audit "$dir" --update-baseline
  expect_status 'update baseline' 0
  grep -qxF "docs.md${T}1${T}13" "$dir/scripts/ci/docs-prose-baseline.tsv" \
    || fail 'updated row does not record 1 run of 13'
  commit_all "$dir" baseline-update
  run_audit "$dir" --strict
  expect_status 'refreshed baseline' 0
  echo ok
}

case_monotonic_total_cannot_increase() {
  echo '== monotonic baseline total cannot increase =='
  local dir
  dir="$(make_repo monotonic-total)"
  gen_lines 13 >"$dir/docs.md"
  printf 'docs.md\t1\t13\n' >>"$dir/scripts/ci/docs-prose-baseline.tsv"
  commit_all "$dir" baseline
  printf 'other.md\t1\t13\n' >>"$dir/scripts/ci/docs-prose-baseline.tsv"
  commit_all "$dir" grew-total
  base_ref=HEAD~1
  run_audit "$dir" --strict
  base_ref=HEAD
  expect_status 'increased total' 1
  expect_contains 'monotonic finding' 'type=baseline-total-increased base=1 current=2'
  echo ok
}

case_new_long_paragraph_fails
case_excluded_structures_pass
case_long_list_item_fails
case_growth_and_improvement_require_refresh
case_monotonic_total_cannot_increase

echo 'PASS: docs-prose audit tests'

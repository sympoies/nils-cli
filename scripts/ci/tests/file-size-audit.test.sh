#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/file-size-audit.sh.
#
# Each case builds a throwaway git repository that carries copies of the audit
# script and its Rust measurement helper, then runs the audit black-box. The
# limits are the production values (2,000 implementation / 3,000 test lines),
# so fixtures generate lines just over (or exactly at) each limit.
#
# Coverage:
#   - a new over-limit file fails, and a file exactly at the limit passes;
#   - a baselined file that grows fails;
#   - a shrink, or a file that drops under the limit, fails until
#     --update-baseline records it;
#   - a baseline row for a deleted file fails;
#   - the per-kind baseline total cannot increase against the base ref, while a
#     rename that moves its row passes;
#   - in-file #[cfg(test)] items, test-only files under tests/, and out-of-line
#     #[cfg(test)] mod files are counted as test lines;
#   - a raw string containing a column-0 `}` does not end a test module early;
#   - non-strict mode warns without failing, and an unresolvable base ref fails
#     strict mode instead of silently skipping the monotonic check.

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi

script="$repo_root/scripts/ci/file-size-audit.sh"
helper="$repo_root/scripts/ci/lib/rust_file_size.py"
for required in "$script" "$helper"; do
  if [[ ! -f "$required" ]]; then
    echo "FAIL: missing required file: $required" >&2
    exit 1
  fi
done

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/file-size-audit-test.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

T=$'\t'
base_ref=HEAD
audit_output=""
status=0

fail() {
  echo "FAIL: $*" >&2
  echo "$audit_output" >&2
  exit 1
}

# make_repo <name> -> prints a fresh git repo carrying the audit and helper.
make_repo() {
  local dir="$tmp_dir/$1"
  mkdir -p "$dir/scripts/ci/lib"
  cp "$script" "$dir/scripts/ci/file-size-audit.sh"
  cp "$helper" "$dir/scripts/ci/lib/rust_file_size.py"
  git init -q "$dir"
  write_baseline "$dir"
  printf '%s' "$dir"
}

# write_baseline <dir> [row...] -> writes the baseline with the given rows.
write_baseline() {
  local dir="$1"
  shift
  {
    printf 'path\tkind\tlines\n'
    local row
    for row in "$@"; do
      printf '%s\n' "$row"
    done
  } >"$dir/scripts/ci/file-size-baseline.tsv"
}

# gen_lines <count> -> prints <count> Rust comment lines.
gen_lines() {
  awk -v count="$1" 'BEGIN { for (i = 1; i <= count; i++) print "// line " i }'
}

commit_all() {
  git -C "$1" add -A
  git -C "$1" -c user.name=fixture -c user.email=fixture@example.invalid \
    -c commit.gpgsign=false commit -q --allow-empty -m "$2"
}

# run_audit <dir> [audit args...] -> sets globals `audit_output` and `status`.
run_audit() {
  local dir="$1"
  shift
  set +e
  audit_output="$(cd "$dir" && bash scripts/ci/file-size-audit.sh --base "$base_ref" "$@" 2>&1)"
  status=$?
  set -e
}

expect_status() {
  local label="$1" want="$2"
  if [[ "$status" -ne "$want" ]]; then
    fail "$label: exit status $status, want $want"
  fi
}

expect_contains() {
  local label="$1" needle="$2"
  if ! grep -qF -- "$needle" <<<"$audit_output"; then
    fail "$label: missing '$needle'"
  fi
}

expect_not_contains() {
  local label="$1" needle="$2"
  if grep -qF -- "$needle" <<<"$audit_output"; then
    fail "$label: unexpected '$needle'"
  fi
}

# baseline_has <dir> <exact row> -> succeeds when the baseline has that line.
baseline_has() {
  grep -qxF -- "$2" "$1/scripts/ci/file-size-baseline.tsv"
}

case_new_over_limit_impl_fails() {
  echo "== new over-limit implementation file fails; exact limit passes =="
  local dir
  dir="$(make_repo new-over-limit)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2001 >"$dir/crates/demo/src/big.rs"
  gen_lines 2000 >"$dir/crates/demo/src/edge.rs"
  commit_all "$dir" base

  run_audit "$dir" --strict
  expect_status "new over-limit file" 1
  expect_contains "new file reported" "type=new-over-limit-file path=crates/demo/src/big.rs kind=impl"
  expect_not_contains "exact limit is not over" "path=crates/demo/src/edge.rs"
  echo "ok"
}

case_baselined_growth_fails() {
  echo "== baselined file that grows fails =="
  local dir
  dir="$(make_repo growth)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2002 >"$dir/crates/demo/src/big.rs"
  write_baseline "$dir" "crates/demo/src/big.rs${T}impl${T}2001"
  commit_all "$dir" base

  run_audit "$dir" --strict
  expect_status "baselined growth" 1
  expect_contains "growth reported" "type=baselined-file-grew path=crates/demo/src/big.rs kind=impl baseline=2001 current=2002"
  echo "ok"
}

case_shrink_requires_baseline_update() {
  echo "== shrink needs --update-baseline; dropping under the limit removes the row =="
  local dir
  dir="$(make_repo shrink)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2100 >"$dir/crates/demo/src/big.rs"
  write_baseline "$dir" "crates/demo/src/big.rs${T}impl${T}2500"
  commit_all "$dir" base

  run_audit "$dir" --strict
  expect_status "shrink without update" 1
  expect_contains "stale row reported" "type=stale-baseline-row path=crates/demo/src/big.rs kind=impl"

  run_audit "$dir" --update-baseline
  expect_status "update baseline" 0
  baseline_has "$dir" "crates/demo/src/big.rs${T}impl${T}2100" || fail "update did not record 2100"
  commit_all "$dir" shrink
  run_audit "$dir" --strict
  expect_status "shrink after update" 0

  gen_lines 1900 >"$dir/crates/demo/src/big.rs"
  commit_all "$dir" under-limit
  run_audit "$dir" --strict
  expect_status "under-limit without update" 1
  expect_contains "under-limit row stale" "type=stale-baseline-row path=crates/demo/src/big.rs kind=impl"

  run_audit "$dir" --update-baseline
  expect_status "update removes row" 0
  if grep -q 'big.rs' "$dir/scripts/ci/file-size-baseline.tsv"; then
    fail "under-limit file still has a baseline row"
  fi
  commit_all "$dir" under-limit-update
  run_audit "$dir" --strict
  expect_status "under-limit after update" 0
  echo "ok"
}

case_baseline_row_for_deleted_file_is_stale() {
  echo "== baseline row for a deleted file is stale =="
  local dir
  dir="$(make_repo deleted)"
  write_baseline "$dir" "crates/demo/src/gone.rs${T}impl${T}2500"
  commit_all "$dir" base

  run_audit "$dir" --strict
  expect_status "deleted file row" 1
  expect_contains "deleted row reported" "type=stale-baseline-row path=crates/demo/src/gone.rs kind=impl"
  echo "ok"
}

case_baseline_total_cannot_increase() {
  echo "== baseline total per kind cannot increase against the base ref =="
  local dir
  base_ref=HEAD~1
  dir="$(make_repo total-increase)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2001 >"$dir/crates/demo/src/big.rs"
  write_baseline "$dir" "crates/demo/src/big.rs${T}impl${T}2001"
  commit_all "$dir" base

  gen_lines 2001 >"$dir/crates/demo/src/other.rs"
  write_baseline "$dir" "crates/demo/src/big.rs${T}impl${T}2001" "crates/demo/src/other.rs${T}impl${T}2001"
  commit_all "$dir" raise-allowance

  run_audit "$dir" --strict
  expect_status "baseline total increase" 1
  expect_contains "total increase reported" "type=baseline-total-increased kind=impl base=2001 current=4002"
  base_ref=HEAD
  echo "ok"
}

case_rename_moves_row_without_raising_total() {
  echo "== rename that moves its baseline row passes =="
  local dir
  base_ref=HEAD~1
  dir="$(make_repo rename)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2001 >"$dir/crates/demo/src/big.rs"
  write_baseline "$dir" "crates/demo/src/big.rs${T}impl${T}2001"
  commit_all "$dir" base

  git -C "$dir" mv crates/demo/src/big.rs crates/demo/src/moved.rs
  write_baseline "$dir" "crates/demo/src/moved.rs${T}impl${T}2001"
  commit_all "$dir" rename

  run_audit "$dir" --strict
  expect_status "rename with row transfer" 0
  expect_contains "pass reported" "PASS: file-size audit"
  base_ref=HEAD
  echo "ok"
}

case_in_file_test_module_counts_as_test_lines() {
  echo "== in-file #[cfg(test)] module counts as test lines =="
  local dir i
  dir="$(make_repo in-file-tests)"
  mkdir -p "$dir/crates/demo/src"
  for i in 1 2 3 4 5; do
    echo "fn a$i() {}"
  done >"$dir/crates/demo/src/with_tests.rs"
  {
    echo "#[cfg(test)]"
    echo "mod tests {"
    gen_lines 3000
    echo "}"
  } >>"$dir/crates/demo/src/with_tests.rs"
  commit_all "$dir" base

  run_audit "$dir" --update-baseline
  expect_status "update baseline" 0
  baseline_has "$dir" "crates/demo/src/with_tests.rs${T}test${T}3003" || fail "expected 3003 test lines"
  if grep -q "with_tests.rs${T}impl" "$dir/scripts/ci/file-size-baseline.tsv"; then
    fail "impl lines of the in-file test module leaked into impl"
  fi
  echo "ok"
}

case_raw_string_column_zero_brace_does_not_end_module() {
  echo "== raw string with a column-0 brace does not end a test module early =="
  local dir total
  dir="$(make_repo raw-string)"
  mkdir -p "$dir/crates/demo/src"
  {
    echo "fn a() {}"
    echo "#[cfg(test)]"
    echo "mod tests {"
    echo '    const SAMPLE: &str = r#"'
    echo '}'
    echo '"#;'
    gen_lines 3000
    echo "}"
  } >"$dir/crates/demo/src/raw.rs"
  total="$(wc -l <"$dir/crates/demo/src/raw.rs" | tr -d ' ')"
  commit_all "$dir" base

  run_audit "$dir" --update-baseline
  expect_status "update baseline" 0
  baseline_has "$dir" "crates/demo/src/raw.rs${T}test${T}$((total - 1))" \
    || fail "expected $((total - 1)) test lines (module must not end at the raw-string brace)"
  echo "ok"
}

case_out_of_line_and_tests_dir_files_are_test_only() {
  echo "== out-of-line #[cfg(test)] mod file and tests/ file count as test lines =="
  local dir
  dir="$(make_repo test-only-files)"
  mkdir -p "$dir/crates/demo/src/widget" "$dir/crates/demo/tests"
  printf 'pub fn widget() {}\n#[cfg(test)]\nmod tests;\n' >"$dir/crates/demo/src/widget.rs"
  gen_lines 3001 >"$dir/crates/demo/src/widget/tests.rs"
  gen_lines 3001 >"$dir/crates/demo/tests/integration.rs"
  commit_all "$dir" base

  run_audit "$dir" --update-baseline
  expect_status "update baseline" 0
  baseline_has "$dir" "crates/demo/src/widget/tests.rs${T}test${T}3001" || fail "out-of-line module not test-only"
  baseline_has "$dir" "crates/demo/tests/integration.rs${T}test${T}3001" || fail "tests/ file not test-only"
  if grep -q "widget.rs${T}" "$dir/scripts/ci/file-size-baseline.tsv"; then
    fail "parent file with a 'mod tests;' declaration should be under the limit"
  fi
  echo "ok"
}

case_non_strict_warns_without_failing() {
  echo "== non-strict mode reports regressions as warnings =="
  local dir
  dir="$(make_repo non-strict)"
  mkdir -p "$dir/crates/demo/src"
  gen_lines 2001 >"$dir/crates/demo/src/big.rs"
  commit_all "$dir" base

  run_audit "$dir"
  expect_status "non-strict" 0
  expect_contains "warning reported" "WARN: file-size regression"
  echo "ok"
}

case_unresolvable_base_fails_strict() {
  echo "== unresolvable base ref fails strict mode =="
  local dir
  base_ref=refs/heads/does-not-exist
  dir="$(make_repo bad-base)"
  commit_all "$dir" base

  run_audit "$dir" --strict
  expect_status "unresolvable base" 2
  expect_contains "base error reported" "base ref does not resolve"
  base_ref=HEAD
  echo "ok"
}

case_new_over_limit_impl_fails
case_baselined_growth_fails
case_shrink_requires_baseline_update
case_baseline_row_for_deleted_file_is_stale
case_baseline_total_cannot_increase
case_rename_moves_row_without_raising_total
case_in_file_test_module_counts_as_test_lines
case_raw_string_column_zero_brace_does_not_end_module
case_out_of_line_and_tests_dir_files_are_test_only
case_non_strict_warns_without_failing
case_unresolvable_base_fails_strict

echo "PASS: file-size audit self-test"

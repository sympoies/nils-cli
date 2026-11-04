#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/lib/ci-build-guard.sh: the build retry fires once on
# a linker crash signature and never on test failures or compile errors.

repo_root="$(git rev-parse --show-toplevel)"
source "$repo_root/scripts/ci/lib/ci-build-guard.sh"

work="$(mktemp -d "${TMPDIR:-/tmp}/ci-build-guard-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT

# A fake build that fails with FAKE_OUTPUT on its first FAKE_FAIL_RUNS runs,
# then succeeds. Each run appends a line to FAKE_COUNTER.
fake_build="$work/fake-build.sh"
printf '%s\n' \
  '#!/usr/bin/env bash' \
  'echo run >>"$FAKE_COUNTER"' \
  'runs="$(wc -l <"$FAKE_COUNTER")"' \
  'if [[ "$runs" -le "$FAKE_FAIL_RUNS" ]]; then echo "$FAKE_OUTPUT"; exit 1; fi' \
  'echo "build ok"' >"$fake_build"
chmod +x "$fake_build"

# attempt <name> <output> <fail_runs> <expected_status> <expected_runs>
attempt() {
  local name="$1" output="$2" fail_runs="$3" want_status="$4" want_runs="$5"
  local status=0 runs
  : >"$work/counter"
  FAKE_COUNTER="$work/counter" FAKE_OUTPUT="$output" FAKE_FAIL_RUNS="$fail_runs" \
    GITHUB_STEP_SUMMARY="$work/summary" \
    ci_build_with_linker_retry "$fake_build" >"$work/out" 2>&1 || status=$?
  runs="$(wc -l <"$work/counter" | tr -d ' ')"
  if [[ "$status" -ne "$want_status" || "$runs" -ne "$want_runs" ]]; then
    echo "FAIL: $name: status=$status runs=$runs (want status=$want_status runs=$want_runs)" >&2
    cat "$work/out" >&2
    exit 1
  fi
}

: >"$work/summary"
bus='collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped'
killed='collect2: fatal error: ld terminated with signal 9 [Killed]'

attempt "signal 7 retries once and recovers" "$bus" 1 0 2
grep -q 'Linker crash retry' "$work/summary" || { echo "FAIL: retry not in summary" >&2; exit 1; }
attempt "signal 9 retries once and recovers" "$killed" 1 0 2
attempt "second linker crash is not retried again" "$bus" 5 1 2
attempt "compile error is not retried" 'error[E0308]: mismatched types' 5 1 1
attempt "test failure is not retried" 'FAIL [ 0.1s] some::test (ld terminated with signal 7)' 5 1 1
attempt "other linker signal is not retried" 'collect2: fatal error: ld terminated with signal 11 [Segmentation fault]' 5 1 1
attempt "success runs once" 'unused' 0 0 1

echo "ok: ci-build-guard tests passed"

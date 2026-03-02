#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
GENERATOR_SCRIPT="$REPO_ROOT/scripts/generate-third-party-artifacts.sh"
LICENSES_FILE="$REPO_ROOT/THIRD_PARTY_LICENSES.md"
NOTICES_FILE="$REPO_ROOT/THIRD_PARTY_NOTICES.md"

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  local context="$3"
  if [[ "$haystack" != *"$needle"* ]]; then
    fail "$context missing '$needle'"
  fi
}

run_generator() {
  local mode="$1"
  local stdout_file="$2"
  local stderr_file="$3"
  (
    cd "$REPO_ROOT"
    bash "$GENERATOR_SCRIPT" "$mode"
  ) >"$stdout_file" 2>"$stderr_file"
}

[[ -x "$GENERATOR_SCRIPT" ]] || fail "missing executable script: $GENERATOR_SCRIPT"
[[ -f "$LICENSES_FILE" ]] || fail "missing licenses artifact: $LICENSES_FILE"
[[ -f "$NOTICES_FILE" ]] || fail "missing notices artifact: $NOTICES_FILE"

TMP_DIR="$(mktemp -d)"
cp "$LICENSES_FILE" "$TMP_DIR/licenses.original"
cp "$NOTICES_FILE" "$TMP_DIR/notices.original"

cleanup() {
  if [[ -f "$TMP_DIR/licenses.original" ]]; then
    cp "$TMP_DIR/licenses.original" "$LICENSES_FILE"
  fi
  if [[ -f "$TMP_DIR/notices.original" ]]; then
    cp "$TMP_DIR/notices.original" "$NOTICES_FILE"
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# Determinism check: two consecutive --write runs must produce byte-identical artifacts.
run_generator --write "$TMP_DIR/write-first.stdout" "$TMP_DIR/write-first.stderr"
cp "$LICENSES_FILE" "$TMP_DIR/licenses.after-first-write"
cp "$NOTICES_FILE" "$TMP_DIR/notices.after-first-write"

run_generator --write "$TMP_DIR/write-second.stdout" "$TMP_DIR/write-second.stderr"
cmp -s "$LICENSES_FILE" "$TMP_DIR/licenses.after-first-write" || fail "--write is not deterministic for THIRD_PARTY_LICENSES.md"
cmp -s "$NOTICES_FILE" "$TMP_DIR/notices.after-first-write" || fail "--write is not deterministic for THIRD_PARTY_NOTICES.md"

run_generator --check "$TMP_DIR/check-clean.stdout" "$TMP_DIR/check-clean.stderr"
CHECK_CLEAN_STDOUT="$(cat "$TMP_DIR/check-clean.stdout")"
assert_contains "$CHECK_CLEAN_STDOUT" "PASS: third-party artifacts are up-to-date" "clean --check output"
NOTICES_CONTENT="$(cat "$NOTICES_FILE")"
assert_contains "$NOTICES_CONTENT" "### option-ext 0.2.0" "mpl crate section"
assert_contains "$NOTICES_CONTENT" "- Source URL: <https://crates.io/crates/option-ext/0.2.0>" "mpl source url line"
assert_contains "$NOTICES_CONTENT" "- License text (MPL-2.0): <https://mozilla.org/MPL/2.0/>" "mpl license text line"

# Drift detection path: mutate one artifact and ensure --check fails with actionable guidance.
printf '\n<!-- test drift marker -->\n' >>"$LICENSES_FILE"

set +e
run_generator --check "$TMP_DIR/check-drift.stdout" "$TMP_DIR/check-drift.stderr"
DRIFT_RC=$?
set -e
[[ "$DRIFT_RC" -ne 0 ]] || fail "expected --check to fail after introducing drift"

CHECK_DRIFT_STDERR="$(cat "$TMP_DIR/check-drift.stderr")"
assert_contains "$CHECK_DRIFT_STDERR" "FAIL: artifact drift detected: THIRD_PARTY_LICENSES.md" "drift --check diagnostics"
assert_contains \
  "$CHECK_DRIFT_STDERR" \
  "FAIL: third-party artifacts are stale; run: bash scripts/generate-third-party-artifacts.sh --write" \
  "drift --check remediation hint"

# Recovery path: re-run --write and verify --check returns to success.
run_generator --write "$TMP_DIR/write-repair.stdout" "$TMP_DIR/write-repair.stderr"
run_generator --check "$TMP_DIR/check-repaired.stdout" "$TMP_DIR/check-repaired.stderr"
CHECK_REPAIRED_STDOUT="$(cat "$TMP_DIR/check-repaired.stdout")"
assert_contains "$CHECK_REPAIRED_STDOUT" "PASS: third-party artifacts are up-to-date" "repaired --check output"

printf 'OK\n'

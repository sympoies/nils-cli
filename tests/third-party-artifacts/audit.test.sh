#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
AUDIT_SCRIPT="$REPO_ROOT/scripts/ci/third-party-artifacts-audit.sh"
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

run_audit() {
  local mode="${1:-}"
  local output_file="$2"
  local rc_file="$3"
  (
    cd "$REPO_ROOT"
    if [[ -z "$mode" ]]; then
      bash "$AUDIT_SCRIPT"
    else
      bash "$AUDIT_SCRIPT" "$mode"
    fi
  ) >"$output_file" 2>&1
  printf '%s\n' "$?" >"$rc_file"
}

[[ -f "$AUDIT_SCRIPT" ]] || fail "missing audit script: $AUDIT_SCRIPT"
[[ -f "$GENERATOR_SCRIPT" ]] || fail "missing generator script: $GENERATOR_SCRIPT"
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

(
  cd "$REPO_ROOT"
  bash "$GENERATOR_SCRIPT" --write
) >/dev/null 2>&1

# Clean path: strict audit should pass and emit a PASS summary.
run_audit "--strict" "$TMP_DIR/clean-strict.out" "$TMP_DIR/clean-strict.rc"
[[ "$(cat "$TMP_DIR/clean-strict.rc")" -eq 0 ]] || fail "clean strict audit should exit 0"
assert_contains \
  "$(cat "$TMP_DIR/clean-strict.out")" \
  "PASS: third-party artifact audit (strict=1, drift=0, missing=0)" \
  "clean strict audit output"

# Drift path: mutate one artifact and verify non-strict warns while strict fails.
printf '\n<!-- audit test drift marker -->\n' >>"$LICENSES_FILE"

run_audit "" "$TMP_DIR/drift-nonstrict.out" "$TMP_DIR/drift-nonstrict.rc"
[[ "$(cat "$TMP_DIR/drift-nonstrict.rc")" -eq 0 ]] || fail "drift non-strict audit should exit 0"
DRIFT_NONSTRICT_OUTPUT="$(cat "$TMP_DIR/drift-nonstrict.out")"
assert_contains "$DRIFT_NONSTRICT_OUTPUT" "WARN: artifact drift detected: THIRD_PARTY_LICENSES.md" "drift non-strict diagnostics"
assert_contains "$DRIFT_NONSTRICT_OUTPUT" "WARN: third-party artifact audit (strict=0, drift=1, missing=0)" "drift non-strict summary"

set +e
run_audit "--strict" "$TMP_DIR/drift-strict.out" "$TMP_DIR/drift-strict.rc"
set -e
[[ "$(cat "$TMP_DIR/drift-strict.rc")" -eq 1 ]] || fail "drift strict audit should exit 1"
DRIFT_STRICT_OUTPUT="$(cat "$TMP_DIR/drift-strict.out")"
assert_contains "$DRIFT_STRICT_OUTPUT" "FAIL: artifact drift detected: THIRD_PARTY_LICENSES.md" "drift strict diagnostics"
assert_contains "$DRIFT_STRICT_OUTPUT" "FAIL: third-party artifact audit (strict=1, drift=1, missing=0)" "drift strict summary"

printf 'OK\n'

#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
AUDIT_SCRIPT="$REPO_ROOT/scripts/ci/release-tarball-third-party-audit.sh"
LICENSES_FILE="$REPO_ROOT/THIRD_PARTY_LICENSES.md"
NOTICES_FILE="$REPO_ROOT/THIRD_PARTY_NOTICES.md"
README_FILE="$REPO_ROOT/README.md"
LICENSE_FILE="$REPO_ROOT/LICENSE"

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
  local target="$1"
  local tag="$2"
  local dist_dir="$3"
  local output_file="$4"
  local rc_file="$5"
  (
    cd "$REPO_ROOT"
    bash "$AUDIT_SCRIPT" --target "$target" --tag "$tag" --dist-dir "$dist_dir"
  ) >"$output_file" 2>&1
  printf '%s\n' "$?" >"$rc_file"
}

package_fixture() {
  local dist_dir="$1"
  local tag="$2"
  local target="$3"
  local include_notices="$4"
  local package_dir="$dist_dir/nils-cli-${tag}-${target}"
  local tarball="$dist_dir/nils-cli-${tag}-${target}.tar.gz"

  rm -rf "$package_dir"
  mkdir -p "$package_dir/bin"
  printf '#!/usr/bin/env bash\nexit 0\n' >"$package_dir/bin/nils-cli-smoke"
  chmod 0755 "$package_dir/bin/nils-cli-smoke"

  cp "$README_FILE" "$LICENSE_FILE" "$LICENSES_FILE" "$package_dir/"
  if [[ "$include_notices" == "1" ]]; then
    cp "$NOTICES_FILE" "$package_dir/"
  fi

  tar -C "$dist_dir" -czf "$tarball" "nils-cli-${tag}-${target}"
}

[[ -f "$AUDIT_SCRIPT" ]] || fail "missing audit script: $AUDIT_SCRIPT"
[[ -f "$LICENSES_FILE" ]] || fail "missing licenses artifact: $LICENSES_FILE"
[[ -f "$NOTICES_FILE" ]] || fail "missing notices artifact: $NOTICES_FILE"

TMP_DIR="$(mktemp -d)"
TARGET="x86_64-unknown-linux-gnu"
TAG="v0.0.0-test"
DIST_DIR="$TMP_DIR/dist"
mkdir -p "$DIST_DIR"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# Happy path: package includes both third-party artifacts and audit passes.
package_fixture "$DIST_DIR" "$TAG" "$TARGET" "1"
run_audit "$TARGET" "$TAG" "$DIST_DIR" "$TMP_DIR/audit-pass.out" "$TMP_DIR/audit-pass.rc"
[[ "$(cat "$TMP_DIR/audit-pass.rc")" -eq 0 ]] || fail "audit should pass for complete package"
AUDIT_PASS_OUTPUT="$(cat "$TMP_DIR/audit-pass.out")"
assert_contains "$AUDIT_PASS_OUTPUT" "PASS: release tarball third-party audit (target=${TARGET}, missing=0" "audit pass output"

EXTRACT_DIR="$TMP_DIR/extract"
mkdir -p "$EXTRACT_DIR"
tar -C "$EXTRACT_DIR" -xzf "$DIST_DIR/nils-cli-${TAG}-${TARGET}.tar.gz"
EXTRACT_ROOT="$EXTRACT_DIR/nils-cli-${TAG}-${TARGET}"
[[ -f "$EXTRACT_ROOT/THIRD_PARTY_LICENSES.md" ]] || fail "extracted archive missing THIRD_PARTY_LICENSES.md"
[[ -f "$EXTRACT_ROOT/THIRD_PARTY_NOTICES.md" ]] || fail "extracted archive missing THIRD_PARTY_NOTICES.md"

# Failure diagnostics path: missing notices artifact should fail with clear messaging.
package_fixture "$DIST_DIR" "$TAG" "$TARGET" "0"
set +e
run_audit "$TARGET" "$TAG" "$DIST_DIR" "$TMP_DIR/audit-fail.out" "$TMP_DIR/audit-fail.rc"
set -e
[[ "$(cat "$TMP_DIR/audit-fail.rc")" -eq 1 ]] || fail "audit should fail when notices artifact is omitted"
AUDIT_FAIL_OUTPUT="$(cat "$TMP_DIR/audit-fail.out")"
assert_contains "$AUDIT_FAIL_OUTPUT" "FAIL: missing required file in tarball: THIRD_PARTY_NOTICES.md" "audit fail diagnostics"
assert_contains "$AUDIT_FAIL_OUTPUT" "FAIL: release tarball third-party audit (target=${TARGET}, missing=1" "audit fail summary"

printf 'OK\n'

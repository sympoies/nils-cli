#!/usr/bin/env bash
set -euo pipefail

# Self-test for scripts/ci/test-env-isolation-guard.sh.
#
# The guard runs `cargo` under a poisoned environment, so each case puts a fake
# `cargo` first on a PATH that contains no other cargo and checks the guard's
# verdict:
#   - a run that touches nothing passes, and sees none of the caller's variables;
#   - a write under the poisoned HOME, AGENT_HOME, or XDG_STATE_HOME fails;
#   - a run of the inherited auth-sync or remote-auth hook command fails;
#   - a run that could not build fails the guard instead of passing vacuously, while
#     plain test failures (nextest exit 100) are left to the regular gate.

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
guard="$repo_root/scripts/ci/test-env-isolation-guard.sh"
if [[ -z "$repo_root" || ! -f "$guard" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/env-guard-selftest.XXXXXX")"
trap 'rm -rf -- "$work"' EXIT
fake_bin="$work/bin"
mkdir -p "$fake_bin" "$work/home" "$work/cargo-home"

# Only the fake cargo may resolve: a failed exec of the fake must not fall
# through to a real toolchain. Tools the guard itself needs live in /usr/bin:/bin.
export PATH="$fake_bin:/usr/bin:/bin"
export HOME="$work/home"
export CARGO_HOME="$work/cargo-home"
export RUSTUP_HOME="$work/cargo-home"
export CARGO_TARGET_DIR="$work/target"

# The fake answers `nextest --version` and `build` quietly; only the test run
# (`cargo test ...` or `cargo nextest run ...`) executes the case body.
write_fake_cargo() {
  printf '#!/bin/sh\n[ "$1" = test ] || [ "$2" = run ] || exit 0\n%s\n' "$1" >"$fake_bin/cargo"
  chmod +x "$fake_bin/cargo"
  if [[ "$(command -v cargo)" != "$fake_bin/cargo" ]]; then
    echo "FAIL: fake cargo is not the only cargo on PATH" >&2
    exit 1
  fi
}

run_guard() {
  bash "$guard" >"$work/out" 2>"$work/err" && return 0
  return 1
}

expect_pass() {
  local name="$1"
  if ! run_guard; then
    echo "FAIL: $name: guard rejected a hermetic run" >&2
    cat "$work/err" >&2
    exit 1
  fi
  echo "ok: $name"
}

expect_fail() {
  local name="$1" needle="$2"
  if run_guard; then
    echo "FAIL: $name: guard accepted a leaking run" >&2
    exit 1
  fi
  if ! grep -q -- "$needle" "$work/err"; then
    echo "FAIL: $name: missing '$needle' in guard output" >&2
    cat "$work/err" >&2
    exit 1
  fi
  echo "ok: $name"
}

export ENV_GUARD_OUTER_VARIABLE=caller-value
write_fake_cargo 'test -z "${ENV_GUARD_OUTER_VARIABLE-}" && exit 0
exit 3'
expect_pass "hermetic run sees a cleared environment"

write_fake_cargo 'printf x > "$AGENT_HOME/leak"; exit 0'
expect_fail "write under AGENT_HOME is a leak" "changed the poisoned sentinel"

write_fake_cargo 'printf x > "$HOME/leak"; exit 0'
expect_fail "write under HOME is a leak" "changed the poisoned sentinel"

write_fake_cargo 'mkdir -p "$XDG_STATE_HOME/tool" && : > "$XDG_STATE_HOME/tool/lock"; exit 0'
expect_fail "write under XDG_STATE_HOME is a leak" "changed the poisoned sentinel"

write_fake_cargo 'printf x >> "$CODEX_AUTH_FILE"; exit 0'
expect_fail "rewritten credential file is a leak" "changed the poisoned sentinel"

write_fake_cargo '"$CODEX_PROXY_AUTH_SYNC_COMMAND"; exit 0'
expect_fail "inherited auth-sync command ran" "hook command ran"

write_fake_cargo '"$CODEX_AUTH_REMOTE_SSH" host; exit 0'
expect_fail "inherited remote-auth command ran" "hook command ran"

write_fake_cargo 'exit 101'
expect_fail "a run that could not build fails the guard" "exited 101"

write_fake_cargo 'exit 100'
expect_pass "failing tests alone are left to the regular gate"

echo "ok: test environment isolation guard"

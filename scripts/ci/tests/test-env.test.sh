#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel)"
source "$repo_root/scripts/ci/test-env.sh"
run() { "$@"; }

export FORGE_IDENTITY_PRINCIPAL=fixture-parent
export FORGE_IDENTITY_SESSION=fixture-session
export FORGE_IDENTITY_TEST_PROBE=fixture-probe
export TEST_ENV_ORDINARY=fixture-ordinary

run_test bash -eu -c '
  [[ -z "${!FORGE_IDENTITY_@}" ]]
  [[ "$TEST_ENV_ORDINARY" == fixture-ordinary ]]
'
[[ "$FORGE_IDENTITY_PRINCIPAL" == fixture-parent ]]
[[ "$FORGE_IDENTITY_SESSION" == fixture-session ]]
[[ "$FORGE_IDENTITY_TEST_PROBE" == fixture-probe ]]

# Fixtures supply explicit variables inside the isolated test process.
run_test env FORGE_IDENTITY_PRINCIPAL=fixture-explicit bash -eu -c '
  [[ "$FORGE_IDENTITY_PRINCIPAL" == fixture-explicit ]]
'

# Preserve command failure status for the caller's normal gate handling.
if run_test bash -c 'exit 23'; then
  echo "FAIL: failing test command succeeded" >&2
  exit 1
else
  [[ "$?" -eq 23 ]]
fi

# An already isolated runner also works under nounset.
run_test bash -eu -c '
  source "$1"
  run() { "$@"; }
  run_test test -z "${!FORGE_IDENTITY_@}"
' bash "$repo_root/scripts/ci/test-env.sh"

# Linux CI moves test temp directories to tmpfs only when it has room, and the
# local gate and a caller-chosen TMPDIR are left alone.
shm_fixture="$(mktemp -d)"
trap 'rm -rf "$shm_fixture"' EXIT
child_tmpdir='printf "%s" "${TMPDIR:-unset}"'
export NILS_CLI_TEST_TMPFS_DIR="$shm_fixture"
(
  unset TMPDIR GITHUB_ACTIONS
  [[ "$(run_test bash -c "$child_tmpdir")" == unset ]]
)
(
  unset TMPDIR
  export GITHUB_ACTIONS=true
  if [[ "$(uname -s)" == Linux ]]; then
    seen="$(run_test bash -c "$child_tmpdir" 2>/dev/null)"
    [[ "$seen" == "$shm_fixture"/nils-cli-test-tmp.* ]]
    [[ -z "$(ls -A "$shm_fixture")" ]]
    log="$(NILS_CLI_TEST_TMPFS_MIN_FREE_KIB=999999999999 run_test bash -c "$child_tmpdir" 2>&1)"
    [[ "$log" == *"keeping the default TMPDIR"*"need 999999999999"*unset ]]
    status=0
    run_test bash -c 'exit 17' 2>/dev/null || status=$?
    [[ "$status" -eq 17 ]]
    [[ -z "$(ls -A "$shm_fixture")" ]]
  fi
)
(
  export GITHUB_ACTIONS=true TMPDIR=/caller/chosen
  [[ "$(run_test bash -c "$child_tmpdir")" == /caller/chosen ]]
)

echo "ok: test identity environment isolation"

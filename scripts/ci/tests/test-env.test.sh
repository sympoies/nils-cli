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

echo "ok: test identity environment isolation"

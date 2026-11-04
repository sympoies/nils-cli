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

# Linux CI moves the temp directory of nextest test runs to tmpfs only when it
# has room; builds, other commands, the local gate and a caller-chosen TMPDIR
# are left alone. A stub cargo records how it was invoked.
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir "$work/bin" "$work/shm"
cat > "$work/bin/cargo" <<'STUB'
#!/usr/bin/env bash
echo "$* TMPDIR=${TMPDIR:-unset}" >> "$CARGO_STUB_LOG"
[[ -z "${CARGO_STUB_EXIT:-}" ]] || exit "$CARGO_STUB_EXIT"
STUB
chmod +x "$work/bin/cargo"
export PATH="$work/bin:$PATH" CARGO_STUB_LOG="$work/log"
export NILS_CLI_TEST_TMPFS_DIR="$work/shm" NILS_CLI_REPORT_DIR="$work/reports"
nextest_run() { run_test cargo nextest run --profile ci -p fixture 2>"$work/err"; }

(
  unset TMPDIR GITHUB_ACTIONS
  : > "$CARGO_STUB_LOG"
  nextest_run
  [[ "$(wc -l < "$CARGO_STUB_LOG")" -eq 1 ]]
  grep -q 'TMPDIR=unset$' "$CARGO_STUB_LOG"
)
if [[ "$(uname -s)" == Linux ]]; then
  (
    unset TMPDIR
    export GITHUB_ACTIONS=true
    : > "$CARGO_STUB_LOG"
    nextest_run
    # The build runs first on the normal TMPDIR, then the tests on tmpfs.
    [[ "$(sed -n 1p "$CARGO_STUB_LOG")" == *"--no-run TMPDIR=unset" ]]
    [[ "$(sed -n 2p "$CARGO_STUB_LOG")" == *"--tool-config-file"*"TMPDIR=$work/shm/nils-cli-test-tmp."* ]]
    [[ "$(wc -l < "$CARGO_STUB_LOG")" -eq 2 ]]
    [[ -z "$(ls -A "$work/shm")" ]]
    # Other commands keep the normal TMPDIR.
    : > "$CARGO_STUB_LOG"
    run_test cargo test --workspace --doc
    [[ "$(cat "$CARGO_STUB_LOG")" == "test --workspace --doc TMPDIR=unset" ]]
    # Not enough free space: logged fallback, no failure, no redirect.
    : > "$CARGO_STUB_LOG"
    NILS_CLI_TEST_TMPFS_MIN_FREE_KIB=999999999999 nextest_run
    grep -q "keeping the default TMPDIR.*need 999999999999" "$work/err"
    ! grep -q "$work/shm" "$CARGO_STUB_LOG"
    # A sampler that dies at once (a full tmpfs, a failing df) must not turn a
    # passing run into a failure or leak the directory.
    (
      test_env_sample_tmpfs_peak() { exit 1; }
      : > "$CARGO_STUB_LOG"
      nextest_run
      [[ -z "$(ls -A "$work/shm")" ]]
    )
    # A failing test run keeps its status and still cleans up.
    status=0
    CARGO_STUB_EXIT=17 nextest_run || status=$?
    [[ "$status" -eq 17 ]]
    [[ -z "$(ls -A "$work/shm")" ]]
  )
fi
(
  export GITHUB_ACTIONS=true TMPDIR=/caller/chosen
  : > "$CARGO_STUB_LOG"
  nextest_run
  [[ "$(wc -l < "$CARGO_STUB_LOG")" -eq 1 ]]
  grep -q 'TMPDIR=/caller/chosen$' "$CARGO_STUB_LOG"
)

echo "ok: test identity environment isolation"

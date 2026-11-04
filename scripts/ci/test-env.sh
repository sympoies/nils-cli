#!/usr/bin/env bash
# Run tests without inheriting the caller's forge identity selection or broker.
# Test fixtures may still supply their own identity variables after launch.
# The caller provides run() to retain its command tracing and failure handling.
#
# On GitHub Linux runners the tests' temporary directories move to tmpfs. State
# writes fsync every file and its directory, which dominates the state-heavy
# `nils-agent-session` tests on a real disk. tmpfs pages count as memory, so the
# redirect needs NILS_CLI_TEST_TMPFS_MIN_FREE_KIB free (default 2 GiB) and falls
# back to the existing TMPDIR, with a logged reason, when that does not hold.
# `/dev/shm` is outside every git work tree, as docs/specs/test-temp-directory-
# policy.md requires. The local gate and macOS never take this path.
test_env_would_use_tmpfs() {
  [[ -z "${TMPDIR:-}" && "${GITHUB_ACTIONS:-}" == true && "$(uname -s)" == Linux ]]
}

test_env_use_tmpfs() {
  test_env_would_use_tmpfs || return 1
  local shm="${NILS_CLI_TEST_TMPFS_DIR:-/dev/shm}"
  local min_free="${NILS_CLI_TEST_TMPFS_MIN_FREE_KIB:-2097152}"
  local free
  if [[ ! -d "$shm" || ! -w "$shm" ]]; then
    echo "test-env: keeping the default TMPDIR: $shm is not a writable directory" >&2
    return 1
  fi
  free="$(df -Pk "$shm" | awk 'NR == 2 { print $4 }')"
  if [[ ! "$free" =~ ^[0-9]+$ ]] || (( free < min_free )); then
    echo "test-env: keeping the default TMPDIR: $shm has ${free:-unknown} KiB free, need $min_free" >&2
    return 1
  fi
  TMPDIR="$(mktemp -d "$shm/nils-cli-test-tmp.XXXXXX")" || return 1
  export TMPDIR
  echo "test-env: TMPDIR=$TMPDIR (tmpfs, $free KiB free, minimum $min_free KiB)" >&2
}

# Record the largest `used` KiB seen on the tmpfs while the test command ran.
# Sampling every 0.5 s makes this a lower bound: shorter bursts can be missed.
test_env_sample_tmpfs_peak() {
  local shm="$1" peak=0 used
  while sleep 0.5; do
    used="$(df -Pk "$shm" | awk 'NR == 2 { print $3 }')"
    [[ "$used" =~ ^[0-9]+$ ]] && (( used > peak )) && peak="$used"
    echo "$peak" > "$TMPDIR.peak-used-kib"
  done
}

run_test() (
  local test_identity_key
  for test_identity_key in "${!FORGE_IDENTITY_@}"; do
    unset "$test_identity_key"
  done
  local status=0 sampler="" argument no_run=0 nextest_run=0
  for argument in "$@"; do
    [[ "$argument" == --no-run ]] && no_run=1
  done
  [[ "${1##*/}" == cargo && "${2:-}" == nextest && "${3:-}" == run ]] && nextest_run=1
  # Only nextest test processes use tmpfs. Compile and link, which handle large
  # files, and doc tests keep the normal TMPDIR; tmpfs pages count as memory.
  if (( nextest_run && ! no_run )) && test_env_would_use_tmpfs; then
    local build_args=() index=0
    for argument in "$@"; do
      [[ "$argument" == -- ]] && break
      index=$((index + 1))
    done
    build_args=("${@:1:index}" --no-run "${@:index+1}")
    run python3 "$(dirname "${BASH_SOURCE[0]}")/gate-reports.py" "${build_args[@]}" ||
      exit "$?"
  fi
  if (( nextest_run && ! no_run )) && test_env_use_tmpfs; then
    local shm="${NILS_CLI_TEST_TMPFS_DIR:-/dev/shm}"
    test_env_sample_tmpfs_peak "$shm" &
    sampler=$!
    # shellcheck disable=SC2064
    trap "set +e; kill $sampler 2>/dev/null; [[ -f \"$TMPDIR.peak-used-kib\" ]] && echo \"test-env: peak \$(cat \"$TMPDIR.peak-used-kib\") KiB used on $shm\" >&2; rm -rf \"$TMPDIR\" \"$TMPDIR.peak-used-kib\"" EXIT
  fi
  if [[ "${1##*/}" == cargo && ( "${2:-}" == nextest ||
        ( "${2:-}" == llvm-cov && "${3:-}" == nextest ) ) ]]; then
    run python3 "$(dirname "${BASH_SOURCE[0]}")/gate-reports.py" "$@" || status=$?
  else
    run "$@" || status=$?
  fi
  exit "$status"
)

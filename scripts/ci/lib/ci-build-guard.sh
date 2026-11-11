# shellcheck shell=bash
# Shared by the CI check scripts: disk diagnostics and a single retry of a
# build step that died to a linker crash. Source it; it defines functions only.

# Free space on the file system holding the cargo target directory, and the
# size of that directory, so a full disk is visible in the job log.
ci_disk_report() {
  local label="$1"
  local target="${CARGO_TARGET_DIR:-target}"
  echo "disk[$label]: free space and target size"
  df -h "$PWD" 2>&1 | sed 's/^/disk: /' || true
  if [[ -d "$target" ]]; then
    echo "disk: target size $(du -sh "$target" 2>/dev/null | cut -f1) ($target)"
  else
    echo "disk: target directory absent ($target)"
  fi
}

# Report disk state when the script exits nonzero. Install once per script.
ci_disk_report_on_failure() {
  local status=$?
  if [[ "$status" -ne 0 ]]; then
    ci_disk_report "failure (exit $status)" || true
  fi
  return "$status"
}

# True when the log holds a linker crash signature: `ld terminated with
# signal 7` (bus error) or 9 (killed). Ordinary compile errors and test
# failures never match.
ci_log_has_linker_crash() {
  grep -Eq 'collect2: fatal error: ld terminated with signal (7|9)( |$)' "$1"
}

# Run a build command; when it fails with a linker crash signature, report the
# disk state and run it once more. Any other failure, or a second failure,
# returns the command's status. Only pass build-only commands (no tests run).
ci_build_with_linker_retry() {
  local log status errexit=0
  [[ $- == *e* ]] && errexit=1
  log="$(mktemp "${TMPDIR:-/tmp}/ci-build-guard.XXXXXX")"
  set +e
  "$@" 2>&1 | tee "$log"
  status="${PIPESTATUS[0]}"
  if [[ "$status" -ne 0 ]] && ci_log_has_linker_crash "$log"; then
    local note="linker crash (exit $status); retrying the build once: $*"
    echo "warning: $note"
    if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
      echo "- Linker crash retry: $note" >>"$GITHUB_STEP_SUMMARY"
    fi
    ci_disk_report "before linker retry"
    echo "+ $*"
    "$@" 2>&1 | tee "$log"
    status="${PIPESTATUS[0]}"
  fi
  rm -f "$log"
  [[ "$errexit" -eq 1 ]] && set -e
  return "$status"
}

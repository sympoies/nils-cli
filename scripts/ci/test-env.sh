#!/usr/bin/env bash
# Run tests without inheriting the caller's forge identity selection or broker.
# Test fixtures may still supply their own identity variables after launch.
# The caller provides run() to retain its command tracing and failure handling.
run_test() (
  local test_identity_key
  for test_identity_key in "${!FORGE_IDENTITY_@}"; do
    unset "$test_identity_key"
  done
  if [[ "${1##*/}" == cargo && ( "${2:-}" == nextest ||
        ( "${2:-}" == llvm-cov && "${3:-}" == nextest ) ) ]]; then
    run python3 "$(dirname "${BASH_SOURCE[0]}")/gate-reports.py" "$@"
  else
    run "$@"
  fi
)

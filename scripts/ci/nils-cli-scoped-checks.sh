#!/usr/bin/env bash
set -euo pipefail

# Package selection is supplied only by the trusted-base merge-plan consumer.
# This executor does not select scope or treat an empty list as a PASS.
if [[ "$#" -eq 0 ]]; then
  echo "error: scoped CI needs selected packages" >&2
  exit 2
fi
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
source scripts/ci/test-env.sh

run() {
  echo "+ $*"
  "$@"
}

package_args=()
for package in "$@"; do
  if [[ ! "$package" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]]; then
    echo "error: invalid scoped package" >&2
    exit 2
  fi
  package_args+=(-p "$package")
done

# These non-compiling audits retain workspace contracts without building every
# unrelated binary. Changes to scripts/completions/shared fixtures escalate FULL
# in the accepted policy and execute the complete existing audit stack there.
run bash scripts/ci/docs-placement-audit.sh --strict
run bash scripts/ci/docs-hygiene-audit.sh --strict
run bash scripts/ci/docs-prose-audit.sh --strict
run bash scripts/ci/file-size-audit.sh --strict
run bash scripts/ci/tempdir-leak-audit.sh
run cargo fmt --all -- --check
run cargo clippy "${package_args[@]}" --all-targets --all-features -- -D warnings
run cargo build "${package_args[@]}" --bins

case "${NILS_CLI_TEST_RUNNER:-nextest}" in
  nextest)
    run_test cargo nextest run --profile ci "${package_args[@]}" --all-features
    ;;
  llvm-cov)
    # -p selects tests, not the coverage corpus. Exclude unselected workspace
    # crates explicitly so the unchanged floor measures the declared scope.
    report_excludes="$(cargo metadata --offline --locked --no-deps --format-version 1 | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
members = set(metadata["workspace_members"])
selected = set(sys.argv[1:])
names = {p["name"] for p in metadata["packages"] if p["id"] in members}
if not selected <= names:
    raise SystemExit("selected coverage package missing from workspace")
print("\n".join(sorted(names - selected)))
' "$@")"
    coverage_args=()
    while IFS= read -r excluded; do
      if [[ -n "$excluded" ]]; then
        coverage_args+=(--exclude-from-report "$excluded")
      fi
    done <<< "$report_excludes"
    run mkdir -p target/coverage
    run_test cargo llvm-cov nextest --profile ci "${package_args[@]}" --all-features \
      "${coverage_args[@]}" \
      --no-fail-fast --lcov --output-path target/coverage/lcov.info \
      --fail-under-lines "${NILS_CLI_COVERAGE_FAIL_UNDER_LINES:-85}"
    run cargo llvm-cov report --html --output-dir target/coverage "${coverage_args[@]}"
    ;;
  *)
    echo "error: unsupported scoped CI test runner" >&2
    exit 2
    ;;
esac
if [[ "${NILS_CLI_SKIP_DOCTESTS:-0}" != "1" ]]; then
  run_test cargo test "${package_args[@]}" --all-features --doc
fi
echo "ok: selected CI packages passed ($# package(s))"

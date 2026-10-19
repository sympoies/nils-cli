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

# The base worker measures only the immutable baseline; candidate audits and
# compilation stay on the candidate worker. Both use the same corpus below.
if [[ "${NILS_CLI_SCOPED_COVERAGE_SIDE:-both}" != "base" ]]; then
# These non-compiling audits retain workspace contracts without building every
# unrelated binary. Changes to scripts/completions/shared fixtures escalate FULL
# in the accepted policy and execute the complete existing audit stack there.
run bash scripts/ci/docs-placement-audit.sh --strict
run bash scripts/ci/docs-hygiene-audit.sh --strict
run bash scripts/ci/docs-prose-audit.sh --strict
run bash scripts/ci/file-size-audit.sh --strict
run bash scripts/ci/tempdir-leak-audit.sh
run python3 scripts/ci/assert-import-audit.py
run python3 scripts/ci/tests/assert-import-audit.test.py
run cargo fmt --all -- --check
run cargo clippy "${package_args[@]}" --all-targets --all-features -- -D warnings
run cargo build "${package_args[@]}" --bins --all-features

# Source-only CLI changes can invalidate generated assets without triggering
# FULL. Cover selected required binaries using the existing freshness audit;
# never let its default workspace build expand this selected execution.
completion_bins="$(cargo metadata --offline --locked --no-deps --format-version 1 | python3 -c '
import json, sys
from pathlib import Path
metadata = json.load(sys.stdin)
selected = set(sys.argv[1:])
members = set(metadata["workspace_members"])
packages = [p for p in metadata["packages"] if p["id"] in members]
if not selected <= {p["name"] for p in packages}:
    raise SystemExit("selected completion package missing from workspace")
bins = {t["name"] for p in packages if p["name"] in selected
        for t in p["targets"] if "bin" in t["kind"]}
if bins:
    rows = [line.split("|") for line in Path("docs/specs/completion-coverage-matrix-v1.md").read_text().splitlines()]
    required = {row[1].strip().strip("`") for row in rows
                if len(row) > 2 and row[2].strip() == "`required`"}
    if not required:
        raise SystemExit("completion matrix has no required binaries")
    print("\n".join(sorted(bins & required)))
' "$@")"
completion_args=()
while IFS= read -r binary; do
  if [[ -n "$binary" ]]; then
    completion_args+=(--bin "$binary")
  fi
done <<< "$completion_bins"
if (( ${#completion_args[@]} > 0 )); then
  run bash scripts/ci/completion-freshness-audit.sh --strict --skip-build "${completion_args[@]}"
fi

fi

case "${NILS_CLI_TEST_RUNNER:-nextest}" in
  nextest)
    run_test cargo nextest run --profile ci "${package_args[@]}" --all-features
    ;;
  llvm-cov)
    # -p selects tests, not the report corpus. The exact selected corpus is
    # measured at immutable base and candidate; the full gate retains 85%.
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
    run_test python3 scripts/ci/scoped-coverage-ratchet.py \
      --base "${NILS_CLI_SCOPED_COVERAGE_BASE:?missing immutable scoped coverage base}" \
      "${coverage_args[@]}" "$@"
    if [[ "${NILS_CLI_SCOPED_COVERAGE_SIDE:-both}" != "base" ]]; then
      run cargo llvm-cov report --html --output-dir target/coverage "${package_args[@]}"
    fi
    ;;
  *)
    echo "error: unsupported scoped CI test runner" >&2
    exit 2
    ;;
esac
if [[ "${NILS_CLI_SKIP_DOCTESTS:-0}" != "1" && "${NILS_CLI_SCOPED_COVERAGE_SIDE:-both}" != "base" ]]; then
  run_test cargo test "${package_args[@]}" --all-features --doc
fi
echo "ok: selected CI packages passed ($# package(s))"

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
# unrelated binary. Scripts, shared fixtures, and unmapped completions escalate
# FULL in the accepted policy and execute the complete existing audit stack
# there. A crate-only change no longer escalates, so every cheap audit it can
# break runs here too; the baseline files are audit-only and read by these.
run bash scripts/ci/docs-placement-audit.sh --strict
run bash scripts/ci/docs-hygiene-audit.sh --strict
run bash scripts/ci/markdownlint-audit.sh --strict
run bash scripts/ci/cli-output-contract-lint.sh --strict
run bash scripts/ci/forge-cli-fixture-lint.sh --strict
run bash scripts/ci/docs-prose-audit.sh --strict
run bash scripts/ci/file-size-audit.sh --strict
run bash scripts/ci/test-stale-audit.sh --strict
run bash scripts/ci/tempdir-leak-audit.sh
run bash scripts/ci/completion-asset-audit.sh --strict
run zsh -f tests/zsh/completion.test.zsh
run python3 scripts/ci/assert-import-audit.py
run python3 scripts/ci/tests/assert-import-audit.test.py
run cargo fmt --all -- --check
run cargo clippy "${package_args[@]}" --all-targets --all-features -- -D warnings

# A selected package's tests can run a binary owned by an unselected package
# (forge-cli runs semantic-commit); a -p run builds only the selected packages'
# own binaries. Build, but never test, the owners the tests resolve, as
# local-fast does (scripts/ci/lib/sibling_bins.py).
sibling_names="$(cargo metadata --offline --locked --no-deps --format-version 1 | python3 -c '
import json, sys
sys.path.insert(0, "scripts/ci/lib")
from sibling_bins import sibling_packages
metadata = json.load(sys.stdin)
members = set(metadata["workspace_members"])
packages = [p for p in metadata["packages"] if p["id"] in members]
selected = set(sys.argv[1:])
if not selected <= {p["name"] for p in packages}:
    raise SystemExit("selected sibling package missing from workspace")
print("\n".join(sorted(sibling_packages(packages, selected))))
' "$@")"
build_args=("${package_args[@]}")
while IFS= read -r sibling; do
  if [[ -n "$sibling" ]]; then
    build_args+=(-p "$sibling")
  fi
done <<< "$sibling_names"
run cargo build "${build_args[@]}" --bins --all-features

# Source-only CLI changes can invalidate generated assets without triggering
# FULL. Cover selected required binaries using the existing freshness and flag
# parity audits; never let their default workspace build expand this execution.
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
  run bash scripts/ci/completion-flag-parity-audit.sh --strict --skip-build "${completion_args[@]}"
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
    run cargo llvm-cov report --html --output-dir target/coverage "${package_args[@]}"
    ;;
  *)
    echo "error: unsupported scoped CI test runner" >&2
    exit 2
    ;;
esac
if [[ "${NILS_CLI_SKIP_DOCTESTS:-0}" != "1" ]]; then
  doctest_packages="$(cargo metadata --offline --locked --no-deps --format-version 1 | python3 -c '
import json, sys
metadata = json.load(sys.stdin)
selected = set(sys.argv[1:])
members = set(metadata["workspace_members"])
packages = [p for p in metadata["packages"] if p["id"] in members]
names = {p["name"] for p in packages}
if not selected <= names:
    raise SystemExit("selected doctest package missing from workspace")
has_library = lambda package: any(
    kind in ("lib", "proc-macro")
    for target in package["targets"]
    for kind in target["kind"])
print("\n".join(sorted(p["name"] for p in packages
                     if p["name"] in selected and has_library(p))))
' "$@")"
  doctest_args=()
  while IFS= read -r package; do
    if [[ -n "$package" ]]; then
      doctest_args+=(-p "$package")
    fi
  done <<< "$doctest_packages"
  if (( ${#doctest_args[@]} > 0 )); then
    run_test cargo test "${doctest_args[@]}" --all-features --doc
  fi
fi
echo "ok: selected CI packages passed ($# package(s))"

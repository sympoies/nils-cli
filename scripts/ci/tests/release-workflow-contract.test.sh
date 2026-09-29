#!/usr/bin/env bash
set -euo pipefail

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || true)"
if [[ -z "$repo_root" || ! -d "$repo_root" ]]; then
  echo "error: must run inside the nils-cli git work tree" >&2
  exit 2
fi
cd "$repo_root"

assert_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if ! rg -q --fixed-strings -- "$pattern" "$file"; then
    echo "FAIL: $label" >&2
    echo "  missing from $file: $pattern" >&2
    exit 1
  fi
  echo "ok: $label"
}

assert_not_contains() {
  local file="$1"
  local pattern="$2"
  local label="$3"
  if rg -q --fixed-strings -- "$pattern" "$file"; then
    echo "FAIL: $label" >&2
    echo "  unexpected in $file: $pattern" >&2
    exit 1
  fi
  echo "ok: $label"
}

assert_contains .github/workflows/release.yml \
  'require("./.github/scripts/release-ci-gate.cjs")' \
  "release workflow uses checked-in provenance gate"
assert_contains .github/workflows/release.yml "workflow_dispatch:" \
  "release workflow supports exact-tag recovery dispatch"
assert_contains .github/workflows/release.yml "RELEASE_TRIGGERING_ACTOR" \
  "release reruns bind the triggering identity"
assert_contains .github/workflows/release.yml \
  'bash .github/scripts/validate-release-invocation.sh' \
  "release recovery uses the tested invocation validator"

validator=.github/scripts/validate-release-invocation.sh
bash "$validator" push graysurf graysurf 1 refs/tags/v1.22.10
bash "$validator" workflow_dispatch xsin4880 xsin4880 1 refs/tags/v1.22.10
bash "$validator" workflow_dispatch 'dobi-bot[bot]' 'dobi-bot[bot]' 1 refs/tags/v1.22.10
bash "$validator" push graysurf xsin4880 2 refs/tags/v1.22.10
assert_invocation_rejected() {
  if bash "$validator" "$@" >/dev/null 2>&1; then
    echo "FAIL: release invocation validator accepted: $*" >&2
    exit 1
  fi
}
assert_invocation_rejected workflow_dispatch xsin4880 untrusted-writer 2 refs/tags/v1.22.10
assert_invocation_rejected workflow_dispatch untrusted-writer untrusted-writer 1 refs/tags/v1.22.10
assert_invocation_rejected workflow_dispatch xsin4880 "" 1 refs/tags/v1.22.10
assert_invocation_rejected push graysurf untrusted-writer 2 refs/tags/v1.22.10
assert_invocation_rejected schedule xsin4880 xsin4880 1 refs/tags/v1.22.10
for invalid_ref in refs/heads/main refs/tags/v1.22.10-rc.1 refs/tags/1.22.10; do
  if bash "$validator" push graysurf graysurf 1 "$invalid_ref" >/dev/null 2>&1; then
    echo "FAIL: release ref validator accepted $invalid_ref" >&2
    exit 1
  fi
done
assert_contains .github/workflows/release.yml "pull-requests: read" \
  "release gate can verify the canonical merged PR"
assert_contains .github/workflows/release.yml "- runs_on: ubuntu-24.04-arm" \
  "Linux ARM64 release uses the native GitHub-hosted runner"
assert_contains .github/workflows/release.yml 'run: cargo build --release --workspace --locked --target ${{ matrix.target }}' \
  "release workflow uses the locked native cargo build"
assert_not_contains .github/workflows/release.yml "tool: cross" \
  "release workflow does not install cross"
assert_not_contains .github/workflows/release.yml "cross build" \
  "release workflow does not invoke cross"
assert_contains .github/workflows/ci.yml "cancel-in-progress: \${{ github.event_name == 'pull_request' }}" \
  "CI cancels only superseded pull request runs"
assert_contains .github/workflows/ci.yml "|| github.run_id }}" \
  "main push CI runs never share a concurrency group, so release provenance is never cancelled"
assert_contains .github/workflows/ci.yml "  merge_group:" \
  "CI runs on merge groups, so the required checks report for a required merge queue"
assert_contains .github/workflows/ci.yml "base=\"\${MERGE_GROUP_BASE_SHA}\"" \
  "CI change detection resolves a merge group's base instead of falling back to an empty push base"
assert_contains .github/workflows/ci.yml "save-if: \${{ github.ref == 'refs/heads/main' }}" \
  "CI saves the Rust cache only from main"
assert_contains .github/workflows/release.yml "save-if: false" \
  "release builds do not save tag-scoped Rust caches"
assert_contains .github/workflows/release.yml "shared-key: release-\${{ matrix.target }}" \
  "release builds restore the per-target dependency cache warmed on main"
assert_contains .github/workflows/release-cache.yml "shared-key: release-\${{ matrix.target }}" \
  "the release cache warmer saves under the key release builds restore"
assert_contains .github/workflows/release-cache.yml "save-if: \${{ github.ref == 'refs/heads/main' }}" \
  "the release cache warmer saves only from main, where tag runs can read it"
assert_contains .github/workflows/release-cache.yml "cargo build --release --workspace --locked --target \${{ matrix.target }}" \
  "the release cache warmer runs the release build command"
# rust-cache keys include the runner OS/arch and the rustc version, so a target,
# runner, or toolchain step that differs between the two workflows makes every
# release restore miss without failing anything.
release_build_signature() {
  rg --no-filename '^\s*(- runs_on:|target:|uses: dtolnay/rust-toolchain@|targets:)' "$1" |
    sed -E 's/^[[:space:]]*(- )?//'
}
if [[ "$(release_build_signature .github/workflows/release.yml)" != \
      "$(release_build_signature .github/workflows/release-cache.yml)" ]]; then
  echo "FAIL: the release cache warmer builds the same targets, runners, and toolchain as release.yml" >&2
  diff <(release_build_signature .github/workflows/release.yml) \
    <(release_build_signature .github/workflows/release-cache.yml) >&2 || true
  exit 1
fi
echo "ok: the release cache warmer builds the same targets, runners, and toolchain as release.yml"
assert_contains .github/workflows/ci.yml "key: llvm-cov" \
  "the instrumented macOS job caches its llvm-cov dependencies under their own key"
assert_contains .github/workflows/ci.yml "NILS_CLI_TEST_RUNNER: llvm-cov" \
  "the macOS full lane runs the instrumented tests that enforce the coverage floor"
assert_contains .github/workflows/ci.yml "needs: [changes, test_macos, test_containment]" \
  "coverage reports from the macOS instrumented run instead of repeating it"
# test_containment is not a required check; only the coverage job's own guard
# connects it to merge and the release gate, so assert inside that job.
coverage_job="$(mktemp "${TMPDIR:-/tmp}/ci-coverage-job.XXXXXX")"
linux_test_job="$(mktemp "${TMPDIR:-/tmp}/ci-test-job.XXXXXX")"
macos_job="$(mktemp "${TMPDIR:-/tmp}/ci-macos-job.XXXXXX")"
trap 'rm -f "$coverage_job" "$linux_test_job" "$macos_job"' EXIT
awk '/^  coverage:$/ {in_job = 1; print; next} in_job && /^  [a-z_]+:$/ {exit} in_job {print}' \
  .github/workflows/ci.yml >"$coverage_job"
assert_contains "$coverage_job" "if: \${{ !cancelled() }}" \
  "coverage runs after a failed upstream lane so it reports failure instead of a passing skip"
assert_contains "$coverage_job" "TEST_CONTAINMENT_RESULT: \${{ needs.test_containment.result }}" \
  "coverage reads the parallel containment canaries' result"
assert_contains "$coverage_job" "[ \"\${TEST_CONTAINMENT_RESULT}\" != \"success\" ]; then" \
  "coverage fails closed unless the parallel containment canaries succeeded"
# The stale-test and completion freshness/parity audits give the same answer
# on every OS, so only the Linux `test` job runs them.
awk '/^  test:$/ {in_job = 1; print; next} in_job && /^  [a-z_]+:$/ {exit} in_job {print}' \
  .github/workflows/ci.yml >"$linux_test_job"
awk '/^  test_macos:$/ {in_job = 1; print; next} in_job && /^  [a-z_]+:$/ {exit} in_job {print}' \
  .github/workflows/ci.yml >"$macos_job"
assert_contains "$macos_job" "NILS_CLI_SKIP_OS_INDEPENDENT_AUDITS: \"1\"" \
  "the macOS lane leaves the OS-independent audits to the Linux lane"
assert_contains "$linux_test_job" "NILS_CLI_TEST_RUNNER: nextest" \
  "the Linux test job block was extracted"
assert_not_contains "$linux_test_job" "NILS_CLI_SKIP_OS_INDEPENDENT_AUDITS" \
  "the Linux test lane runs the OS-independent audits"
assert_contains .agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh \
  "NILS_CLI_SKIP_OS_INDEPENDENT_AUDITS" \
  "the required-checks runner honours the OS-independent audit skip"
assert_contains .github/workflows/ci.yml "NILS_CLI_SKIP_DOCTESTS: \"1\"" \
  "the macOS lane leaves the workspace doc tests to the Linux lane"
assert_contains .agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh \
  "NILS_CLI_SKIP_DOCTESTS" \
  "the required-checks runner honours the doc-test skip"
assert_contains .github/workflows/ci.yml "test -s target/coverage/lcov.info" \
  "coverage fails closed when the instrumented run produced no LCOV"
assert_contains .agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh \
  "--fail-under-lines \"\${NILS_CLI_COVERAGE_FAIL_UNDER_LINES:-85}\"" \
  "the llvm-cov runner enforces the 85% coverage floor"
assert_contains .github/workflows/ci.yml "NILS_CLI_COVERAGE_FAIL_UNDER_LINES: \"85\"" \
  "CI pins the 85% coverage floor on the instrumented macOS run"
assert_contains .github/workflows/ci.yml "release_only:" \
  "CI publishes the release-only decision"
assert_contains .github/workflows/ci.yml "scripts/ci/detect-release-only.sh" \
  "CI uses the semantic release classifier"
assert_contains .github/workflows/ci.yml "git show \"\${base}:scripts/ci/detect-release-only.sh\"" \
  "release-only classification loads protected base policy"
assert_contains .github/workflows/ci.yml "\${{ needs.changes.outputs.base_sha }}:scripts/ci/release-only-checks.sh" \
  "reduced checks load the exact base checker"
assert_contains .github/workflows/ci.yml "findTrustedMainCi" \
  "release-only CI requires exact-base full CI proof"
assert_contains .github/workflows/ci.yml "waitForTrustedMainCi" \
  "release-only CI waits for an in-flight exact-base full CI run"
assert_contains .github/workflows/ci.yml "Full validation marker" \
  "full CI is distinguishable from reduced lanes"
assert_contains .github/workflows/ci.yml "scripts/ci/release-only-checks.sh" \
  "Linux and macOS checks expose the reduced lane"
assert_contains .github/workflows/ci.yml "needs.changes.outputs.release_only != 'true'" \
  "coverage work is skipped only after fail-closed classification"
assert_contains .agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh \
  "node scripts/ci/tests/release-ci-gate.test.cjs" \
  "release gate unit tests are in the required suite"
assert_contains docs/runbooks/workspace-maintenance-reference.md \
  "bash scripts/ci/tests/detect-release-only.test.sh" \
  "workspace maintenance reference lists the release classifier tests"
assert_contains docs/specs/workspace-ci-entrypoint-inventory-v1.md \
  "release_candidate" \
  "CI inventory records the semantic release candidate output"
assert_contains docs/specs/workspace-ci-entrypoint-inventory-v1.md \
  "release_only=true" \
  "CI inventory records the trusted reduced-lane decision"
assert_contains docs/specs/workspace-ci-entrypoint-inventory-v1.md \
  ".github/scripts/release-ci-gate.cjs" \
  "CI inventory owns the shared release gate module"
assert_contains docs/specs/workspace-ci-entrypoint-inventory-v1.md \
  "scripts/ci/detect-release-only.sh" \
  "CI inventory owns the semantic release detector"
assert_contains docs/specs/workspace-ci-entrypoint-inventory-v1.md \
  "scripts/ci/release-only-checks.sh" \
  "CI inventory owns the reduced release checker"
assert_contains .agents/skills/project-bump-version-tag-release/SKILL.md \
  "--prepare-only" \
  "release skill documents the internal producer contract mode"
assert_contains .agents/skills/project-bump-version-tag-release/SKILL.md \
  "falls back to full PR CI" \
  "release skill documents the fail-closed full-CI fallback"
assert_contains .agents/skills/project-bump-version-tag-release/SKILL.md \
  "cargo update --workspace" \
  "release skill documents the implemented lockfile refresh command"
assert_contains .agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh \
  "reuses that exact-SHA PR CI" \
  "release helper help documents tag-gate CI reuse"

# Both ends of the tap handoff must verify a receiver-side fact. The dispatches
# endpoint answers 204 with no workflow listening, and a tap run's name is a
# sender-chosen string whose shape differs per trigger path, so neither is proof
# that the formula moved.
assert_contains .github/workflows/release.yml \
  "listWorkflowRuns" \
  "release dispatch job confirms a tap run exists instead of trusting HTTP 204"
assert_contains .agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh \
  "read_tap_formula_version" \
  "tap wait gates on the version published in the tap formula"

# PR-mode delivery must leave a window between opening the PR and merging it: the
# ledger merge gate needs an observation at the current head, and the chain
# refuses an append at a stale head.
assert_contains .agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh \
  "record_release_review_genesis" \
  "release PR delivery records the review-loop ledger genesis before merging"
assert_contains .agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh \
  "mode delivery" \
  "release genesis envelope comes from the review-specialists delivery generator"

echo
echo "PASS: release-workflow-contract.test.sh"

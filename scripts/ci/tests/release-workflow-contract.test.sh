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

assert_contains .github/workflows/full-validation.yml \
  'require("./.github/scripts/full-validation-gate.cjs")' \
  "release workflow requires full exact-commit validation"
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
assert_contains .github/workflows/release.yml "- runs_on: ubuntu-24.04-arm" \
  "Linux ARM64 release uses the native GitHub-hosted runner"
assert_contains .github/workflows/release.yml 'run: cargo build --release --workspace --locked --target ${{ matrix.target }}' \
  "release workflow uses the locked native cargo build"
assert_not_contains .github/workflows/release.yml "tool: cross" \
  "release workflow does not install cross"
assert_not_contains .github/workflows/release.yml "cross build" \
  "release workflow does not invoke cross"
assert_contains .github/workflows/ci.yml "cancel-in-progress: \${{ github.event_name == 'pull_request' }}" \
  "CI cancels only superseded pull request runs, never a main push run in progress"
assert_contains .github/workflows/ci.yml "'nightly-main' || github.run_id)) }}" \
  "main push runs share one group while merge-group runs never share one"
assert_contains .github/workflows/ci.yml "  merge_group:" \
  "CI runs on merge groups, so the required checks report for a required merge queue"
assert_contains .github/workflows/ci.yml '    - cron: "23 5 * * *"' \
  "CI runs the full workflow nightly"
assert_contains .github/workflows/ci.yml "  workflow_dispatch:" \
  "CI supports a manual full run"
assert_contains .github/workflows/ci.yml "if: github.event_name != 'push'" \
  "main pushes skip the full CI lanes"
assert_contains .github/workflows/ci.yml "  main_push:" \
  "main pushes retain a lightweight status workflow"
assert_contains .github/workflows/ci.yml "  nightly_report:" \
  "nightly full CI publishes a failure alert"
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
# Pin the supported asset set independently of matrix/cache agreement: both
# workflows could otherwise reintroduce the same retired target together.
expected_targets=$'aarch64-apple-darwin\naarch64-unknown-linux-gnu\nx86_64-unknown-linux-gnu'
for workflow in .github/workflows/release.yml .github/workflows/release-cache.yml; do
  actual_targets="$(sed -nE 's/^[[:space:]]*target: ([a-z0-9_-]+)$/\1/p' "$workflow" | sort)"
  if [[ "$actual_targets" != "$expected_targets" ]]; then
    echo "FAIL: $workflow must build exactly the three supported release targets" >&2
    exit 1
  fi
done
echo "ok: release and cache matrices contain exactly the three supported targets"
assert_contains .github/workflows/release.yml 'sha256sum "${tarball}"' \
  "checksums are generated from each built target archive"
assert_contains .github/workflows/release.yml 'files: dist/**/*.tar.gz*' \
  "release publication discovers built archives and checksum sidecars"
# Retain the Linux PR lane's existing outputs and protected reduced checker.
# Full-validation contracts cover a different workflow and cannot replace these.
assert_contains .github/workflows/ci.yml 'release_only: ${{ steps.finalize.outputs.release_only }}' \
  "CI publishes the release-only decision"
assert_contains .github/workflows/ci.yml 'release_mode: ${{ steps.detect.outputs.release_mode }}' \
  "CI preserves the release-mode output"
assert_contains .github/workflows/ci.yml 'git show "${base}:scripts/ci/detect-release-only.sh"' \
  "release classification loads exact-base policy"
assert_not_contains .github/workflows/ci.yml 'findTrustedMainCi' \
  "the reduced release lane needs no full base-CI receipt"
assert_not_contains .github/workflows/ci.yml 'waitForTrustedMainCi' \
  "the reduced release lane never waits for a base-CI receipt"
assert_contains scripts/ci/release-only-checks.sh 'RELEASE_ONLY_DETECTOR' \
  "the reduced checker accepts a protected detector"
assert_contains scripts/ci/release-only-checks.sh '--merge-group' \
  "the protected checker understands merge-group mode"
workflow_job() {
  awk -v job="$2" '$0 == "  " job ":" { inside = 1; print; next }
    inside && /^  [a-z_]+:$/ { exit } inside { print }' "$1"
}
linux_job="$(workflow_job .github/workflows/ci.yml test)"
assert_contains <(printf '%s\n' "$linux_job") \
  'git show "${{ needs.changes.outputs.base_sha }}:scripts/ci/detect-release-only.sh"' \
  "Linux reduced validation loads the exact-base detector"
assert_contains <(printf '%s\n' "$linux_job") \
  'git show "${{ needs.changes.outputs.base_sha }}:scripts/ci/release-only-checks.sh"' \
  "Linux reduced validation loads the exact-base checker"
assert_contains <(printf '%s\n' "$linux_job") 'mode_args=(--merge-group)' \
  "Linux reduced validation preserves merge-group mode"
assert_contains <(printf '%s\n' "$linux_job") 'RELEASE_ONLY_DETECTOR="${policy_dir}/detect-release-only.sh"' \
  "Linux reduced validation uses the protected detector"
assert_contains <(printf '%s\n' "$linux_job") '"${mode_args[@]}"' \
  "Linux reduced validation passes its selected mode to the checker"
assert_contains <(printf '%s\n' "$linux_job") '    timeout-minutes: 50' \
  "Linux validation retains its 50-minute backstop"
assert_contains <(workflow_job .github/workflows/ci.yml test_containment) '    timeout-minutes: 25' \
  "Linux containment retains its 25-minute backstop"
assert_contains <(workflow_job .github/workflows/ci.yml cargo_deny) '    timeout-minutes: 15' \
  "dependency policy retains its 15-minute backstop"
assert_contains <(workflow_job .github/workflows/ci.yml changes) '    timeout-minutes: 60' \
  "change detection retains its 60-minute backstop"

# The shared full workflow owns the former merge/release coverage contract.
assert_contains .github/workflows/full-validation.yml 'NILS_CLI_COVERAGE_FAIL_UNDER_LINES: "85"' \
  "full release/nightly coverage retains the 85% floor"
assert_contains .github/workflows/full-validation.yml 'NILS_CLI_SKIP_OS_INDEPENDENT_AUDITS: "1"' \
  "macOS delegates OS-independent audits to full Linux"
assert_contains .github/workflows/full-validation.yml 'NILS_CLI_SKIP_DOCTESTS: "1"' \
  "macOS delegates doctests to full Linux"
assert_contains .config/nextest.toml 'terminate-after = ' \
  "nextest terminates hung tests"
assert_contains .agents/skills/project-verify-required-checks/scripts/project-verify-required-checks.sh \
  '--fail-under-lines "${NILS_CLI_COVERAGE_FAIL_UNDER_LINES:-85}"' \
  "the llvm-cov executor enforces the workspace floor"
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

# The bump script and its /release wrapper are preparation-only: the release
# broker owns tagging, publishing, the tap, and fleet convergence, so neither
# file may carry a path that can reach a remote or a release.
for prepare_only_file in \
  .agents/skills/project-bump-version-tag-release/scripts/project-bump-version-tag-release.sh \
  .agents/scripts/release.sh; do
  # README tag-example patterns legitimately mention `git tag -a v` and
  # `git push origin v`, so match the mutating invocations, not the words.
  for forbidden in 'tag -a "$tag"' 'git push -u' 'git push origin HEAD' 'push origin "$tag"' 'forge-cli' 'homebrew' 'brew ' 'gh run' 'gh api' \
    'wait_for_release_run' 'run_tap_stage' 'read_tap_formula_version' \
    'record_release_review_genesis'; do
    if grep -Fq -- "$forbidden" "$prepare_only_file"; then
      echo "FAIL: $prepare_only_file must stay preparation-only, found: $forbidden" >&2
      exit 1
    fi
  done
done
echo "ok: release bump script is preparation-only"

echo
echo "PASS: release-workflow-contract.test.sh"

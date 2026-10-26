# Workspace CI Entrypoint Inventory v1

## Purpose

This inventory records the canonical owner for each active CI/release workflow
check, the default local validation path, and keep/delete criteria for helper
scripts.
It is the source of truth for Sprint 1 CI entrypoint consolidation.

## Keep/Delete Criteria

A helper path is `keep` only when at least one active caller is discoverable by repository search in one of:

- GitHub workflow YAML (`.github/workflows/*.yml`)
- Canonical contributor docs (`README.md`, `DEVELOPMENT.md`, `docs/runbooks/**`, crate READMEs)
- Canonical runtime entrypoints (`scripts/**`, `.agents/**`, `wrappers/**`) used by active workflows/docs

A helper path is `delete-candidate` when no active caller exists outside historical/transient
planning artifacts. Removal of a `delete-candidate` script is a separate code-change task and is
out of scope for this governance inventory; flag it as a follow-up.

## Canonical Workflow Owners

### `.github/workflows/ci.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| (workflow) | `concurrency` | workflow-local expression | keep | Groups `pull_request` runs by PR number and cancels a superseded run; `merge_group` runs get a unique `run_id` group. Main pushes use the `main-push` group and run only the lightweight `main_push` status job. Scheduled nightly and manual runs execute the full validation on main, serialized as `nightly-main`. |
| `changes` | `Detect change set` | `scripts/ci/detect-docs-only.sh` plus exact-base `scripts/ci/detect-release-only.sh` policy | keep | Emits `base_sha`, `docs_only`, `head_sha`, `release_branch`, and `release_candidate`. Release policy is loaded from the exact base SHA; missing or invalid policy emits `release_candidate=false`. |
| `changes` | `Finalize fail-closed CI lane` | workflow-local boolean | keep | Emits `release_only=true` only when the exact-base detector proved a canonical release bump (`release_candidate=true`); every other state emits false. No full base-CI receipt is required. Release PRs keep the reduced preparation lane; release-candidate merge groups run shared full validation on the combined candidate, and required Linux `test` waits for its success. Ordinary scoped PR/queue checks alone supply no reusable full-validation evidence. |
| `test` | Linux affected checks | `scripts/ci/nils-cli-checks-entrypoint.sh --ci-merge-plan` | keep | PR and combined merge-group candidate; accepted-base plan, strict binding and full Linux fallback. Docs/release PR shortcuts retain their proven audits. |
| `test` | change-detection failure guard | workflow-local result check | keep | Runs with `always()` so failed change detection produces a failed required test rather than an accepted skip. |
| `macos_clippy` | affected-package compile and lint | `scripts/ci/nils-cli-macos-clippy.py` | keep | Merge groups only on `macos-latest` (30 minutes); PRs skip it successfully. Runs clippy with all targets and features and no test execution, and uploads `merge-validation-macos-clippy`. Required Linux `test` needs it and fails closed on a merge group unless it succeeds, so no separate ruleset context is required. |
| `test_containment` | Linux cgroup canary | workflow-local test command and systemd setup | keep | Required alongside `test`; steps skip on docs-only/proven release-only PRs. |
| `cargo_deny` | dependency policy | `scripts/ci/cargo-deny-audit.sh` | keep | Required name is `cargo-deny (advisories + bans)`. |
| `full_validation` | nightly/manual and release-candidate queue full run | `.github/workflows/full-validation.yml` | keep | Runs daily at 05:23 UTC and on dispatch on main, and for release-candidate merge groups on the exact combined candidate SHA. Required Linux `test` waits for this result and fails closed unless it succeeds for a release-candidate group. Ordinary PRs and non-release queue groups do not run this full suite. |
| `nightly_report` | failure tracking and recovery | `.github/scripts/nightly-tracker.cjs` | keep | Serialized bot-owned issue, failed SHA and last green scheduled range; closes on full recovery, reopens on later failure. |
| `main_push` | lightweight main status | workflow-local summary | keep | Main pushes do not repeat the test matrix. |
| `coverage_badge` | badge generation/publish | `scripts/ci/coverage-badge.sh` | keep | Successful nightly/manual full validation supplies LCOV. |

### `.github/workflows/full-validation.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `test` | full Linux checks/doctests | canonical full checks entrypoint | keep | 50 minutes, nextest; caller event SHA, full audit stack. |
| `test_containment` | Linux cgroup canary | workflow-local test command and systemd setup | keep | 25 minutes, required by final full gate. |
| `cargo_deny` | dependency policy | `scripts/ci/cargo-deny-audit.sh` | keep | 15 minutes, required by final full gate. |
| `test_macos` | instrumented workspace tests | full checks entrypoint, llvm-cov | keep | 75 minutes, 85% workspace floor; skips only OS-independent audits and doctests already covered by Linux. LCOV, HTML and JUnit artifacts retained. |
| `coverage` | LCOV and summary | `scripts/ci/coverage-summary.sh` | keep | 10 minutes, requires nonempty macOS report. |
| `gate` | exact-commit all-lane guard | `.github/scripts/full-validation-gate.cjs` | keep | 5 minutes, `always()` requires success from every lane; no skipped/cancelled/missing result admits publication. |

### `.github/workflows/release.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `verify_ci` | authorize release invocation and find reusable full validation | `.github/scripts/validate-release-invocation.sh` + `.github/scripts/release-validation-reuse.cjs` | keep | Authorizes the stable tag and trusted invocation/rerun first, then looks for successful same-repository `ci.yml` merge-group evidence at the exact release commit or a commit whose tree is verified identical through the commit API. Reuse requires exactly one completed successful `full_validation /` job for each of `test`, `test_macos`, `coverage`, `test_containment`, `cargo-deny (advisories + bans)`, and `gate`, plus successful full-validation markers in `test`, `test_macos`, and `coverage`; PR leaf checks or names alone are insufficient. |
| `full_validation` | fallback full tagged-commit tests | `.github/workflows/full-validation.yml` | keep | After authorization, runs on the immutable tag SHA unless complete reusable evidence was verified. Missing, failed, incomplete, unverifiable, or unavailable evidence falls back to full tag validation. |
| `validation_gate` | require authorized full validation | `.github/scripts/release-validation-reuse.cjs` `requireReleaseValidation` | keep | Runs with `always()` and requires successful authorization plus either verified reuse with the fallback skipped, or a successful full tag fallback. Builds depend on this gate. |
| `build` | ripgrep bootstrap | inline OS bootstrap shell | keep | Platform package manager differences remain workflow-local. |
| `build` | `Regenerate third-party artifacts` | `scripts/generate-third-party-artifacts.sh` | keep | Canonical artifact generation gate. |
| `build` | `Package` | `scripts/workspace-bins.sh` + inline packaging shell | keep | Packaging owns workspace binary discovery through script entrypoint. |
| `build` | `Audit release tarball third-party artifacts` | `scripts/ci/release-tarball-third-party-audit.sh` | keep | Canonical release artifact audit. |
| `release` | publish GitHub release | `softprops/action-gh-release@v2` | keep | Standard release publication action. |
| `build` | `Cache cargo` | `Swatinem/rust-cache` restore-only (`save-if: false`) | keep | Restores `shared-key: release-<target>`, which `.github/workflows/release-cache.yml` saves on `main`. A tag run can restore default-branch caches, but not caches saved on another tag. |

### `.github/workflows/release-cache.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `warm` | `Cache cargo`, `Build (release)` | `Swatinem/rust-cache` + the release build command | keep | Runs daily, on dispatch, and when the workflow itself changes on `main`. For each `release.yml` target it runs `cargo build --release --workspace --locked --target <target>`, saving the dependency cache under `release-<target>`. On an exact cache hit it skips the build. It does not trigger on `Cargo.lock`, because every release bump rewrites it, and warming then would start two macOS jobs beside the release. The restore key omits the lockfile hash, so the cache stays mostly warm across dependency bumps. |

### `.github/workflows/peekaboo-release-check.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `check` | `Check stable official releases` | `.github/scripts/peekaboo-release-check.cjs` | keep | Daily/manual, serialized default-branch discovery. Summary and JSON retain last-checked/current/newest/candidate status. Upserts one candidate issue per tag across all issue states without changing the accepted lock. |
| `check` | `Test release discovery fixtures` | `scripts/ci/tests/peekaboo-release-check.test.cjs` | keep | Also run by the full required-checks entrypoint; tests stable filtering, repeat/closed candidates, failures and workflow routing without network/provider writes. |

### `.github/workflows/publish-crates.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `publish` | `Publish selected crates` | `scripts/publish-crates.sh` | keep | Canonical crates publish/dry-run entrypoint. |

## Required-Checks Script Ownership

`./scripts/ci/nils-cli-verification.sh` is canonical for full CI verification ordering:

1. docs/stale/third-party/completion audits (`scripts/ci/*`)
2. completion registration/parity checks
3. compile/test gates (`cargo fmt`, `cargo clippy`, workspace tests)

No workflow may duplicate these audit commands as independent pre-steps unless
the required-checks entrypoint is updated first. Day-to-day local development
uses `bash scripts/ci/nils-cli-checks-entrypoint.sh --local-fast`, which
escalates to the workspace Rust gate when changed files can affect reverse
dependencies.

## Helper Surface Decisions (Sprint 1 Scope)

This section enumerates every script under `scripts/ci/*.sh` (matches `ls scripts/ci/*.sh`) and
records the keep/delete decision plus the active caller evidence.

| Path | Decision | Active caller evidence |
| --- | --- | --- |
| `scripts/ci/cargo-deny-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` supply-chain audit section + `.github/workflows/ci.yml` `cargo-deny` job |
| `scripts/ci/cli-output-contract-lint.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` docs-only/full checks list + `nils-cli-verification.sh` docs-only and full passes |
| `scripts/ci/completion-asset-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/completion-freshness-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/completion-flag-parity-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/coverage-badge.sh` | keep | `.github/workflows/ci.yml` `coverage_badge` job |
| `scripts/ci/coverage-summary.sh` | keep | `.github/workflows/ci.yml` `coverage` job + `docs/runbooks/workspace-maintenance-reference.md` coverage flow |
| `scripts/ci/crate-naming-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/detect-docs-only.sh` | keep | `.github/workflows/ci.yml` `changes` job + `scripts/ci/tests/detect-docs-only.test.sh` |
| `scripts/ci/detect-release-only.sh` | keep | exact-base policy loaded by `.github/workflows/ci.yml` + `scripts/ci/release-only-checks.sh` + `scripts/ci/tests/detect-release-only.test.sh` |
| `scripts/ci/docs-hygiene-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` docs-only and full passes + `scripts/ci/tests/docs-hygiene-audit.test.sh` |
| `scripts/ci/docs-placement-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` docs-only and full passes |
| `scripts/ci/forge-cli-fixture-lint.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/markdownlint-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` docs-only and full passes |
| `scripts/ci/nils-cli-checks-entrypoint.sh` | keep | `.github/workflows/ci.yml` Linux `test` job and shared full-validation jobs + `docs/runbooks/workspace-maintenance-reference.md` local-fast and CI/full commands |
| `scripts/ci/nils-cli-local-fast.sh` | keep | `scripts/ci/nils-cli-checks-entrypoint.sh --local-fast` delegates changed-scope planning/execution here |
| `scripts/ci/publish-order-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step + `scripts/ci/tests/publish-order-audit.test.sh` |
| `scripts/ci/release-tarball-third-party-audit.sh` | keep | `.github/workflows/release.yml` `build` job |
| `scripts/ci/release-only-checks.sh` | keep | exact-base reduced PR lane in `.github/workflows/ci.yml` Linux `test` job + `scripts/ci/tests/release-workflow-contract.test.sh` |
| `scripts/ci/skill-shell-suites.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step (runs every `.agents/skills/*/tests/test_*.sh` suite) |
| `scripts/ci/tempdir-leak-audit.sh` | keep | `scripts/ci/nils-cli-local-fast.sh` unconditional static leak gate + `docs/specs/test-temp-directory-policy.md` |
| `scripts/ci/tempdir-leak-probe.sh` | keep | `scripts/ci/nils-cli-local-fast.sh` workspace-test isolation + `docs/specs/test-temp-directory-policy.md` |
| `scripts/ci/test-stale-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |
| `scripts/ci/third-party-artifacts-audit.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step + dependabot bump skill |
| `scripts/ci/verify-signed-commits.sh` | keep | `lefthook.yml` pre-push hook |
| `scripts/ci/workspace-version-lockstep.sh` | keep | `docs/runbooks/workspace-maintenance-reference.md` full checks list + `nils-cli-verification.sh` step |

## Auxiliary Wrapper / Tooling Decisions

| Path | Decision | Active caller evidence |
| --- | --- | --- |
| `.github/scripts/release-ci-gate.cjs` | keep | historical receipt compatibility helpers + `scripts/ci/tests/release-ci-gate.test.cjs` |
| `.github/scripts/release-validation-reuse.cjs` | keep | `.github/workflows/release.yml` authorization/reuse and final validation gate + `scripts/ci/tests/release-ci-gate.test.cjs` |
| `scripts/ci/lib/cargo-target-dir.sh` | keep | sourced by `scripts/ci/completion-freshness-audit.sh` + `scripts/ci/completion-flag-parity-audit.sh` to locate binaries under `CARGO_TARGET_DIR` / `cargo metadata` `target_directory` |
| `scripts/ci/lib/doc_classify.py` | keep | imported by `scripts/ci/nils-cli-local-fast.sh` planner + `scripts/ci/detect-docs-only.sh` |
| `wrappers/git-cli` | keep | `README.md` wrapper contributor flow + `git-cli` wrapper behavior |

## Validation Commands

```bash
test -f docs/specs/workspace-ci-entrypoint-inventory-v1.md
ls scripts/ci/*.sh
rg -n 'scripts/ci/|nils-cli-verification' \
  .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/publish-crates.yml \
  DEVELOPMENT.md docs/runbooks/workspace-maintenance-reference.md \
  scripts/ci/nils-cli-verification.sh
rg -n 'canonical|delete|delete-candidate|keep|workflow' docs/specs/workspace-ci-entrypoint-inventory-v1.md
```

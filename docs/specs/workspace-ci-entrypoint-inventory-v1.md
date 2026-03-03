# Workspace CI Entrypoint Inventory v1

## Purpose

This inventory records the canonical owner for each active CI/release workflow check and defines keep/delete criteria for helper scripts.
It is the source of truth for Sprint 1 CI entrypoint consolidation.

## Keep/Delete Criteria

A helper path is `keep` only when at least one active caller is discoverable by repository search in one of:

- GitHub workflow YAML (`.github/workflows/*.yml`)
- Canonical contributor docs (`README.md`, `DEVELOPMENT.md`, `docs/runbooks/**`, crate READMEs)
- Canonical runtime entrypoints (`scripts/**`, `.agents/**`, `wrappers/**`) used by active workflows/docs

A helper path is `delete` when no active caller exists outside historical/transient planning artifacts.

## Canonical Workflow Owners

### `.github/workflows/ci.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `test`, `test_macos` | `Checkout`, `Set up Rust`, `Cache cargo`, `Set up Node.js`, tool bootstrap | Upstream GitHub Actions + runner bootstrap shell | keep | Platform bootstrap stays in workflow. |
| `test`, `test_macos` | `Nils CLI checks (includes third-party-artifacts-audit, Completion asset audit, docs-hygiene-audit, test-stale-audit)` | `scripts/ci/nils-cli-checks-entrypoint.sh` -> `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` | keep | Canonical verification contract after setup. |
| `test`, `test_macos` | `Third-party artifact audit` (removed) | replaced by required-checks script ordering | delete | Duplicate workflow fragment removed. |
| `test`, `test_macos` | `Completion asset audit` (removed) | replaced by required-checks script ordering | delete | Duplicate workflow fragment removed. |
| `test`, `test_macos` | `Publish JUnit report`, `Upload JUnit XML` | upstream Actions artifacts/reporting | keep | Post-check reporting only. |
| `coverage` | coverage generation/report/upload/cleanup steps | `cargo llvm-cov` + `scripts/ci/coverage-summary.sh` + upload/comment actions | keep | Coverage artifacts are created and cleaned only in this job. |
| `coverage_badge` | badge generation/publish | `scripts/ci/coverage-badge.sh` + git push flow | keep | Push-only automation path. |

### `.github/workflows/release.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `build` | ripgrep bootstrap | inline OS bootstrap shell | keep | Platform package manager differences remain workflow-local. |
| `build` | `Regenerate third-party artifacts` | `scripts/generate-third-party-artifacts.sh` | keep | Canonical artifact generation gate. |
| `build` | `Package` | `scripts/workspace-bins.sh` + inline packaging shell | keep | Packaging owns workspace binary discovery through script entrypoint. |
| `build` | `Audit release tarball third-party artifacts` | `scripts/ci/release-tarball-third-party-audit.sh` | keep | Canonical release artifact audit. |
| `release` | publish GitHub release | `softprops/action-gh-release@v2` | keep | Standard release publication action. |

### `.github/workflows/publish-crates.yml`

| Job | Step | Canonical owner | Decision | Notes |
| --- | --- | --- | --- | --- |
| `publish` | `Publish selected crates` | `scripts/publish-crates.sh` | keep | Canonical crates publish/dry-run entrypoint. |

## Required-Checks Script Ownership

`./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh` is canonical for verification ordering:

1. docs/stale/third-party/completion audits (`scripts/ci/*`)
2. completion registration/parity checks
3. compile/test gates (`cargo fmt`, `cargo clippy`, workspace tests)

No workflow may duplicate these audit commands as independent pre-steps unless the required-checks entrypoint is updated first.

## Helper Surface Decisions (Sprint 1 Scope)

| Path | Decision | Active caller evidence |
| --- | --- | --- |
| `scripts/ci/wrapper-mode-smoke.sh` | keep | `README.md` wrapper contributor flow + wrapper smoke command examples |
| `scripts/ci/agent-docs-snapshots.sh` | keep | `crates/agent-docs/README.md` snapshot workflow |
| obsolete completion matrix shell helper | delete | no active caller outside itself |
| obsolete release package shell helper | delete | no active CI/release/contributor caller; replaced by canonical release tarball audit script |
| `wrappers/plan-tooling` | keep | `README.md` wrapper contributor flow + runbook wrapper scope includes `plan-tooling` |
| `wrappers/git-cli` | keep | `README.md` wrapper contributor flow includes `git-cli` wrapper behavior |

## Validation Commands

```bash
test -f docs/specs/workspace-ci-entrypoint-inventory-v1.md
rg -n 'scripts/ci/|nils-cli-verify-required-checks' \
  .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/publish-crates.yml \
  DEVELOPMENT.md .agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh
rg -n 'canonical|delete|keep|workflow' docs/specs/workspace-ci-entrypoint-inventory-v1.md
```

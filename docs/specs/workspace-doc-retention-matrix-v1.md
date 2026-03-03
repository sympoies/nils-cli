# Workspace Doc Retention Matrix v1

## Purpose

This matrix records finalized documentation ownership and retention decisions for the simplified
workspace architecture.

Decision fields:

- `scope`: `workspace-level` | `crate-local` | `transient-dev-record`
- `lifecycle`: `canonical` | `delete`
- `decision`: `keep` | `delete` | `move`

## Workspace-Level Inventory (Keep)

| Path | Scope | Lifecycle | Decision | Rationale |
| --- | --- | --- | --- | --- |
| `README.md` | `workspace-level` | `canonical` | `keep` | Workspace overview and contributor entrypoint. |
| `DEVELOPMENT.md` | `workspace-level` | `canonical` | `keep` | Required checks and contributor workflow contract. |
| `AGENTS.md` | `workspace-level` | `canonical` | `keep` | Agent execution policy for this repository. |
| `BINARY_DEPENDENCIES.md` | `workspace-level` | `canonical` | `keep` | Shared runtime/tooling dependency contract. |
| `docs/runbooks/cli-completion-development-standard.md` | `workspace-level` | `canonical` | `keep` | Canonical completion architecture and checks. |
| `docs/runbooks/crates-io-status-script-runbook.md` | `workspace-level` | `canonical` | `keep` | Workspace crates.io status workflow. |
| `docs/runbooks/new-cli-crate-development-standard.md` | `workspace-level` | `canonical` | `keep` | New CLI crate standards. |
| `docs/runbooks/test-cleanup-governance.md` | `workspace-level` | `canonical` | `keep` | Stale-test lifecycle and CI guardrails. |
| `docs/specs/cli-service-json-contract-guideline-v1.md` | `workspace-level` | `canonical` | `keep` | Service-consumed CLI JSON contract guidance. |
| `docs/specs/codex-gemini-cli-parity-contract-v1.md` | `workspace-level` | `canonical` | `keep` | Shared Codex/Gemini parity contract. |
| `docs/specs/codex-gemini-runtime-contract.md` | `workspace-level` | `canonical` | `keep` | Shared provider runtime contract. |
| `docs/specs/completion-contract-template.md` | `workspace-level` | `canonical` | `keep` | Per-crate completion migration contract template. |
| `docs/specs/completion-coverage-matrix-v1.md` | `workspace-level` | `canonical` | `keep` | Completion obligations and enforcement metadata matrix. |
| `docs/specs/crate-docs-placement-policy.md` | `workspace-level` | `canonical` | `keep` | Workspace docs placement policy. |
| `docs/specs/third-party-artifacts-contract-v1.md` | `workspace-level` | `canonical` | `keep` | Third-party artifacts generation contract. |
| `docs/specs/workspace-ci-entrypoint-inventory-v1.md` | `workspace-level` | `canonical` | `keep` | CI owner-script inventory and keep/delete criteria. |
| `docs/specs/workspace-shared-crate-boundary-v1.md` | `workspace-level` | `canonical` | `keep` | Shared crate ownership boundaries. |
| `docs/specs/workspace-test-cleanup-lane-matrix-v1.md` | `workspace-level` | `canonical` | `keep` | Test cleanup sequencing and lane policy. |
| `docs/specs/workspace-doc-retention-matrix-v1.md` | `workspace-level` | `canonical` | `keep` | Doc ownership and retention source of truth (this file). |

## Crate-Local Inventory (Keep)

All paths below are classified as `scope=crate-local`, `lifecycle=canonical`, `decision=keep`.
Rationale: each file is owned by one crate and lives under `crates/<crate>/docs/**`.

- `crates/agent-docs/docs/README.md`
- `crates/api-gql/docs/README.md`
- `crates/api-grpc/docs/README.md`
- `crates/api-rest/docs/README.md`
- `crates/api-test/docs/README.md`
- `crates/api-testing-core/docs/README.md`
- `crates/api-websocket/docs/README.md`
- `crates/api-websocket/docs/specs/websocket-cli-contract-v1.md`
- `crates/api-websocket/docs/specs/websocket-request-schema-v1.md`
- `crates/cli-template/docs/README.md`
- `crates/codex-cli/docs/README.md`
- `crates/codex-cli/docs/runbooks/json-consumers.md`
- `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
- `crates/fzf-cli/docs/README.md`
- `crates/gemini-cli/docs/README.md`
- `crates/gemini-cli/docs/runbooks/json-consumers.md`
- `crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md`
- `crates/git-cli/docs/README.md`
- `crates/git-lock/docs/README.md`
- `crates/git-scope/docs/README.md`
- `crates/git-summary/docs/README.md`
- `crates/image-processing/docs/README.md`
- `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
- `crates/macos-agent/docs/README.md`
- `crates/memo-cli/docs/README.md`
- `crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md`
- `crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
- `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
- `crates/memo-cli/docs/specs/memo-cli-release-policy.md`
- `crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
- `crates/memo-cli/docs/specs/memo-cli-workflow-extension-contract-v1.md`
- `crates/nils-common/docs/README.md`
- `crates/nils-common/docs/specs/markdown-helpers-contract-v1.md`
- `crates/nils-term/docs/README.md`
- `crates/nils-test-support/docs/README.md`
- `crates/plan-issue-cli/docs/README.md`
- `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v2.md`
- `crates/plan-issue-cli/docs/specs/plan-issue-gate-matrix-v1.md`
- `crates/plan-issue-cli/docs/specs/plan-issue-state-machine-v1.md`
- `crates/plan-tooling/docs/README.md`
- `crates/plan-tooling/docs/runbooks/split-prs-build-task-spec-cutover.md`
- `crates/plan-tooling/docs/specs/split-prs-contract-v1.md`
- `crates/plan-tooling/docs/specs/split-prs-contract-v2.md`
- `crates/screen-record/docs/README.md`
- `crates/semantic-commit/docs/README.md`

## Transient/Obsolete Inventory (Delete or Move)

| Path | Scope | Lifecycle | Decision | Reason | Inbound-reference proof |
| --- | --- | --- | --- | --- | --- |
| `docs/plans/markdown-gh-handling-audit-remediation-plan.md` | `transient-dev-record` | `delete` | `delete` | Completed migration plan; no active workflow caller remains. | `rg -n 'markdown-gh-handling-audit-remediation-plan\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `docs/plans/third-party-licenses-notices-release-packaging-plan.md` | `transient-dev-record` | `delete` | `delete` | Completed migration plan; contract moved to canonical spec + scripts. | `rg -n 'third-party-licenses-notices-release-packaging-plan\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `docs/reports/completion-coverage-matrix.md` | `workspace-level` | `delete` | `move` | Promoted from report to canonical workspace spec. | `rg -n 'docs/reports/completion-coverage-matrix\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `docs/runbooks/wrappers-mode-usage.md` | `transient-dev-record` | `delete` | `delete` | Compatibility-only wrapper-mode runbook superseded by canonical README guidance. | `rg -n 'wrappers-mode-usage\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `docs/specs/markdown-github-handling-audit-v1.md` | `transient-dev-record` | `delete` | `delete` | Audit artifact completed; no active policy gate depends on it. | `rg -n 'markdown-github-handling-audit-v1\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `crates/plan-tooling/docs/runbooks/split-prs-migration.md` | `crate-local` | `delete` | `delete` | Migration-only runbook superseded by cutover runbook + v2 contract docs. | `rg -n 'split-prs-migration\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |
| `crates/plan-issue-cli/docs/specs/plan-issue-cli-contract-v1.md` | `crate-local` | `delete` | `delete` | Compatibility-era contract superseded by active v2 contract. | `rg -n 'plan-issue-cli-contract-v1\\.md' README.md DEVELOPMENT.md AGENTS.md BINARY_DEPENDENCIES.md docs crates scripts tests .github` -> no matches. |

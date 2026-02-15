# Plan: nils-cli crate docs migration and governance

## Overview
This plan migrates crate-owned documentation in the `nils-cli` repository to canonical crate-local paths under `crates/<crate>/docs/` and establishes enforceable governance so future docs follow the same rule.
The approach is migration-safe: first define policy and inventory, then move crate-owned root docs with compatibility stubs, then enforce with audit automation in local checks and CI.
A secondary goal is baseline consistency: every workspace crate should expose a crate-local docs index (`docs/README.md`) so contributors have one predictable documentation entrypoint per crate.

## Scope
- In scope: classify root `docs/` files into workspace-level vs crate-owned.
- In scope: migrate crate-owned root docs into crate-local canonical paths under `crates/`.
- In scope: scaffold missing `docs/README.md` indexes for workspace crates.
- In scope: update development standards and automated checks to enforce placement rules.
- In scope: maintain compatibility stubs for legacy root doc paths.
- Out of scope: CLI behavior or feature changes.
- Out of scope: broad rewrites of historical plan content beyond link hygiene.

## Assumptions (if any)
1. `docs/runbooks/codex-cli-json-consumers.md` is owned by `crates/codex-cli` and should be crate-local canonical documentation.
2. `docs/runbooks/image-processing-llm-svg.md` is owned by `crates/image-processing` and should be crate-local canonical documentation.
3. Root `docs/runbooks/new-cli-crate-development-standard.md`, `docs/runbooks/provider-onboarding.md`, and `docs/specs/cli-service-json-contract-guideline-v1.md` remain workspace-level.
4. Compatibility stubs are acceptable for legacy root paths while references are gradually normalized.

## Success Criteria
- Crate-owned root docs are migrated to canonical crate-local locations under `crates/`.
- All workspace crates have `docs/README.md` crate-local docs indexes.
- Development standards explicitly require crate-local docs placement for new crate docs.
- A docs-placement audit exists and is wired into `nils-cli-checks` and CI.
- Legacy root paths are short compatibility stubs only (no duplicated full content).

## Document Inventory To Reorganize

### Crate-owned root docs to migrate into `crates/`
- `docs/runbooks/codex-cli-json-consumers.md` -> `crates/codex-cli/docs/runbooks/json-consumers.md` (reference hits: 7)
- `docs/runbooks/image-processing-llm-svg.md` -> `crates/image-processing/docs/runbooks/llm-svg-workflow.md` (reference hits: 5)

### Workspace-level root docs that should remain under `docs/`
- `docs/runbooks/new-cli-crate-development-standard.md`
- `docs/runbooks/provider-onboarding.md`
- `docs/runbooks/crates-io-status-script-runbook.md`
- `docs/runbooks/wrappers-mode-usage.md`
- `docs/specs/cli-service-json-contract-guideline-v1.md`
- `docs/plans/*.md`

### Crate docs baseline gaps
All current workspace crates are missing `docs/README.md` indexes and need crate-local index scaffolding:
- `agent-docs`, `agent-provider-claude`, `agent-provider-codex`, `agent-provider-gemini`, `agent-runtime-core`, `agentctl`
- `api-gql`, `api-rest`, `api-test`, `api-testing-core`
- `cli-template`, `codex-cli`, `fzf-cli`, `git-cli`, `git-lock`, `git-scope`, `git-summary`
- `image-processing`, `macos-agent`, `memo-cli`
- `nils-common`, `nils-term`, `nils-test-support`
- `plan-tooling`, `screen-record`, `semantic-commit`

### High-impact reference hotspots to update
- `crates/codex-cli/README.md`
- `docs/plans/codex-cli-service-consumable-diag-auth-plan.md`
- `docs/plans/codex-auth-login-save-plan.md`
- `docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`

## Dependency & Parallelization Map
- Critical path: `Task 1.1 -> Task 1.2 -> Task 1.3 -> Task 2.1 -> Task 2.3 -> Task 3.1 -> Task 3.2 -> Task 4.1`.
- Parallel track A: `Task 1.4` can run after `Task 1.1` in parallel with `Task 1.3`.
- Parallel track B: `Task 2.2` can run after `Task 1.3` in parallel with `Task 2.1`.
- Parallel track C: `Task 3.3` can run after `Task 3.2` in parallel with Sprint 4 prep work.
- Parallel track D: `Task 4.2` and `Task 4.3` can run after `Task 4.1`.

## Sprint 1: Policy baseline and crate docs index scaffolding
**Goal**: define placement policy, produce migration inventory, and create crate-local docs indexes as the baseline.
**Demo/Validation**:
- Command(s): `plan-tooling validate --file docs/plans/nils-cli-crate-docs-migration-governance-plan.md`, `find crates -maxdepth 3 -type f -path '*/docs/README.md' | sort`
- Verify: policy and inventory exist, and every workspace crate has crate-local docs index scaffold.

### Task 1.1: Author crate docs placement policy for nils-cli
- **Location**:
  - `docs/specs/crate-docs-placement-policy.md`
  - `DEVELOPMENT.md`
- **Description**: Define normative rules for crate-owned vs workspace-level docs, canonical crate-local paths, and required contributor behavior for new docs.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Policy defines allowed root docs categories and disallowed crate-owned root docs patterns.
  - Policy defines canonical crate-local examples using current repository crates.
  - Development guide references this policy under required checks.
- **Validation**:
  - `test -f docs/specs/crate-docs-placement-policy.md`
  - `rg -n "crate-local|workspace-level|disallowed|canonical" docs/specs/crate-docs-placement-policy.md`
  - `rg -n "crate-docs-placement-policy|Documentation placement" DEVELOPMENT.md`

### Task 1.2: Create migration inventory report with ownership and target paths
- **Location**:
  - `docs/reports/crate-doc-migration-inventory.md`
- **Description**: Record root docs classification, crate ownership mapping, target paths, reference hotspots, migration order, risk level, and status.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Report includes migrated candidates, retained workspace-level docs, and crate docs baseline gap summary.
  - Each migration row includes current path, owner crate, final canonical path, risk level, and status.
  - Report is executable as migration checklist.
- **Validation**:
  - `test -f docs/reports/crate-doc-migration-inventory.md`
  - `rg -n "Owner crate|Final canonical path|Risk level|Migration order|Status" docs/reports/crate-doc-migration-inventory.md`
  - `rg -n "codex-cli-json-consumers|image-processing-llm-svg" docs/reports/crate-doc-migration-inventory.md`

### Task 1.3: Scaffold crate-local docs indexes for all workspace crates
- **Location**:
  - `crates/agent-docs/docs/README.md`
  - `crates/agent-provider-claude/docs/README.md`
  - `crates/agent-provider-codex/docs/README.md`
  - `crates/agent-provider-gemini/docs/README.md`
  - `crates/agent-runtime-core/docs/README.md`
  - `crates/agentctl/docs/README.md`
  - `crates/api-gql/docs/README.md`
  - `crates/api-rest/docs/README.md`
  - `crates/api-test/docs/README.md`
  - `crates/api-testing-core/docs/README.md`
  - `crates/cli-template/docs/README.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/fzf-cli/docs/README.md`
  - `crates/git-cli/docs/README.md`
  - `crates/git-lock/docs/README.md`
  - `crates/git-scope/docs/README.md`
  - `crates/git-summary/docs/README.md`
  - `crates/image-processing/docs/README.md`
  - `crates/macos-agent/docs/README.md`
  - `crates/memo-cli/docs/README.md`
  - `crates/nils-common/docs/README.md`
  - `crates/nils-term/docs/README.md`
  - `crates/nils-test-support/docs/README.md`
  - `crates/plan-tooling/docs/README.md`
  - `crates/screen-record/docs/README.md`
  - `crates/semantic-commit/docs/README.md`
- **Description**: Add missing crate-local docs indexes and ensure each index points to crate README and canonical crate docs sections.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Every workspace crate has `docs/README.md`.
  - Each docs index includes ownership and canonical docs links.
  - Crate root README references `docs/README.md`.
- **Validation**:
  - `for p in crates/agent-docs/docs/README.md crates/agent-provider-claude/docs/README.md crates/agent-provider-codex/docs/README.md crates/agent-provider-gemini/docs/README.md crates/agent-runtime-core/docs/README.md crates/agentctl/docs/README.md crates/api-gql/docs/README.md crates/api-rest/docs/README.md crates/api-test/docs/README.md crates/api-testing-core/docs/README.md crates/cli-template/docs/README.md crates/codex-cli/docs/README.md crates/fzf-cli/docs/README.md crates/git-cli/docs/README.md crates/git-lock/docs/README.md crates/git-scope/docs/README.md crates/git-summary/docs/README.md crates/image-processing/docs/README.md crates/macos-agent/docs/README.md crates/memo-cli/docs/README.md crates/nils-common/docs/README.md crates/nils-term/docs/README.md crates/nils-test-support/docs/README.md crates/plan-tooling/docs/README.md crates/screen-record/docs/README.md crates/semantic-commit/docs/README.md; do test -f "$p"; done`
  - `rg -n "docs/README.md" crates/*/README.md`

### Task 1.4: Update new crate development standard with docs placement rules
- **Location**:
  - `docs/runbooks/new-cli-crate-development-standard.md`
- **Description**: Add explicit crate-local docs placement requirements and mandatory checklist items for new crate/new markdown changes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 4
- **Acceptance criteria**:
  - Standard explicitly requires crate-local docs under `crates/`.
  - Standard includes pre-commit docs placement audit command.
  - Standard clarifies workspace-level exceptions.
- **Validation**:
  - `rg -n "crate-local|docs/README.md|docs placement" docs/runbooks/new-cli-crate-development-standard.md`
  - `rg -n "docs-placement-audit\\.sh --strict|pre-commit" docs/runbooks/new-cli-crate-development-standard.md`
  - `rg -n "workspace-level exception|workspace-level docs|exception list" docs/runbooks/new-cli-crate-development-standard.md`

## Sprint 2: Crate-owned root docs migration with compatibility stubs
**Goal**: migrate crate-owned root docs to crate-local canonical paths and preserve compatibility via stubs.
**Demo/Validation**:
- Command(s): `bash scripts/ci/docs-placement-audit.sh --strict`, `rg -n "Moved to" docs/runbooks/codex-cli-json-consumers.md docs/runbooks/image-processing-llm-svg.md`
- Verify: canonical docs exist under crates and legacy root paths are stub pointers only.

### Task 2.1: Migrate codex-cli JSON consumers runbook to crate-local docs
- **Location**:
  - `docs/runbooks/codex-cli-json-consumers.md`
  - `crates/codex-cli/docs/runbooks/json-consumers.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/codex-cli/README.md`
- **Description**: Move canonical content into crate-local runbook path and replace root file with compatibility stub including migration metadata.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Canonical codex-cli runbook exists under `crates/codex-cli/docs/runbooks/`.
  - Root runbook becomes short `Moved to` stub only.
  - Codex CLI docs index and README point to canonical runbook path.
- **Validation**:
  - `test -f crates/codex-cli/docs/runbooks/json-consumers.md`
  - `rg -n "Moved to|Migration date" docs/runbooks/codex-cli-json-consumers.md`
  - `rg -n "docs/runbooks/json-consumers.md" crates/codex-cli/docs/README.md crates/codex-cli/README.md`

### Task 2.2: Migrate image-processing LLM SVG runbook to crate-local docs
- **Location**:
  - `docs/runbooks/image-processing-llm-svg.md`
  - `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
  - `crates/image-processing/docs/README.md`
  - `crates/image-processing/README.md`
- **Description**: Move canonical content into crate-local runbook path and replace root file with compatibility stub including migration metadata.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Canonical image-processing runbook exists under `crates/image-processing/docs/runbooks/`.
  - Root runbook becomes short `Moved to` stub only.
  - Image-processing docs index and README point to canonical runbook path.
- **Validation**:
  - `test -f crates/image-processing/docs/runbooks/llm-svg-workflow.md`
  - `rg -n "Moved to|Migration date" docs/runbooks/image-processing-llm-svg.md`
  - `rg -n "docs/runbooks/llm-svg-workflow.md" crates/image-processing/docs/README.md crates/image-processing/README.md`

### Task 2.3: Update high-impact references to canonical crate-local paths
- **Location**:
  - `docs/plans/codex-cli-service-consumable-diag-auth-plan.md`
  - `docs/plans/codex-auth-login-save-plan.md`
  - `docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`
  - `crates/codex-cli/README.md`
- **Description**: Update active references from old root runbook paths to new crate-local canonical paths while leaving archived plans unchanged unless directly impacted.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Active references use crate-local canonical paths.
  - No active operational docs depend on old runbook canonical content.
  - Root paths remain only as compatibility stubs.
- **Validation**:
  - `! rg -n "docs/runbooks/codex-cli-json-consumers.md|docs/runbooks/image-processing-llm-svg.md" crates/codex-cli/README.md docs/plans/codex-cli-service-consumable-diag-auth-plan.md docs/plans/codex-auth-login-save-plan.md docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`
  - `rg -n "crates/codex-cli/docs/runbooks/json-consumers.md|crates/image-processing/docs/runbooks/llm-svg-workflow.md" crates/codex-cli/README.md docs/plans/codex-cli-service-consumable-diag-auth-plan.md docs/plans/codex-auth-login-save-plan.md docs/plans/image-processing-from-svg-llm-tooling-migration-plan.md`

## Sprint 3: Governance automation and CI enforcement
**Goal**: enforce docs placement rules automatically in local checks and CI.
**Demo/Validation**:
- Command(s): `bash scripts/ci/docs-placement-audit.sh --strict`, `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: docs placement policy is automatically checked before merge.

### Task 3.1: Implement docs placement audit for nils-cli
- **Location**:
  - `scripts/ci/docs-placement-audit.sh`
  - `release/crates-io-publish-order.txt`
- **Description**: Add deterministic docs audit checking crate docs indexes presence and disallowed crate-owned root docs patterns outside approved workspace-level categories.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
  - Task 2.2
- **Complexity**: 8
- **Acceptance criteria**:
  - Script detects disallowed new crate-owned root docs.
  - Script verifies required docs index presence for workspace crates.
  - Script provides CI-friendly PASS/FAIL output.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-placement-audit.sh --strict | rg -q "PASS|FAIL"`
  - `rg -n "docs/README.md|missing docs index|required docs index" scripts/ci/docs-placement-audit.sh`
  - `bash -c 'set +e; tmp_file="docs/runbooks/_tmp-test-contract.md"; printf "# temp\n" > "$tmp_file"; bash scripts/ci/docs-placement-audit.sh --strict >/dev/null 2>&1; rc=$?; rm -f "$tmp_file"; test "$rc" -ne 0'`

### Task 3.2: Wire docs placement audit into nils-cli checks and CI
- **Location**:
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `.github/workflows/ci.yml`
  - `DEVELOPMENT.md`
- **Description**: Ensure docs placement audit is part of required local checks and CI pipeline documentation.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `nils-cli-checks` runs docs placement audit.
  - CI job runs docs placement audit through checks pipeline.
  - Development guide includes docs audit in required checks.
- **Validation**:
  - `rg -n "docs-placement-audit" .agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh .github/workflows/ci.yml DEVELOPMENT.md`
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

### Task 3.3: Register docs placement policy in agent-docs required docs
- **Location**:
  - `AGENT_DOCS.toml`
- **Description**: Add docs placement policy file as required for project-dev context so implementation sessions always load the policy.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `AGENT_DOCS.toml` includes policy path in project-dev required docs.
  - `agent-docs resolve --context project-dev --strict` reports policy as present.
- **Validation**:
  - `rg -n "crate-docs-placement-policy.md" AGENT_DOCS.toml`
  - `agent-docs resolve --context project-dev --strict --format checklist`

## Sprint 4: Closure, lifecycle decision, and maintainer handoff
**Goal**: finalize migration state, document stub lifecycle, and publish maintainer-facing summary.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`, `cargo test --workspace`
- Verify: migration and governance are complete with stable operational guidance.

### Task 4.1: Full regression and high-impact link hygiene pass
- **Location**:
  - `README.md`
  - `docs/reports/crate-doc-migration-inventory.md`
  - `DEVELOPMENT.md`
- **Description**: Run full required checks and ensure high-impact active docs do not rely on old canonical root runbook paths.
- **Dependencies**:
  - Task 3.2
- **Complexity**: 5
- **Acceptance criteria**:
  - Required check suite passes.
  - Inventory report status is updated for all migration items.
  - High-impact docs point to canonical crate-local paths.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo test --workspace`
  - `! rg -n "docs/runbooks/codex-cli-json-consumers.md|docs/runbooks/image-processing-llm-svg.md" README.md DEVELOPMENT.md crates/codex-cli/README.md`

### Task 4.2: Stub lifecycle decision and policy finalization
- **Location**:
  - `docs/specs/crate-docs-placement-policy.md`
  - `docs/reports/crate-doc-migration-inventory.md`
- **Description**: Define and document root compatibility stub lifecycle policy, including whether stubs are permanent redirects or sunset with date.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Policy documents one explicit stub lifecycle decision.
  - Inventory records current status for each compatibility stub.
  - Decision is reflected in development standards references.
- **Validation**:
  - `rg -n "stub|deprecation|sunset|redirect" docs/specs/crate-docs-placement-policy.md docs/reports/crate-doc-migration-inventory.md DEVELOPMENT.md`

### Task 4.3: Publish maintainer migration summary
- **Location**:
  - `docs/reports/crate-doc-migration-summary.md`
- **Description**: Publish concise maintainer summary covering before/after paths, enforcement model, and contributor required steps.
- **Dependencies**:
  - Task 4.1
- **Complexity**: 3
- **Acceptance criteria**:
  - Summary includes before/after examples.
  - Summary includes mandatory pre-commit checks.
  - Summary links to policy and inventory docs.
- **Validation**:
  - `test -f docs/reports/crate-doc-migration-summary.md`
  - `rg -n "before|after|pre-commit|docs-placement-audit|crate-docs-placement-policy|crate-doc-migration-inventory" docs/reports/crate-doc-migration-summary.md`

## Testing Strategy
- Policy/document checks: `bash scripts/ci/docs-placement-audit.sh --strict`.
- Required local gate: `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`.
- Workspace regression: `cargo test --workspace` and `zsh -f tests/zsh/completion.test.zsh`.
- Link hygiene checks: targeted `rg -n` checks for old root runbook paths in high-impact docs.

## Risks & gotchas
- Historical `docs/plans/*.md` references are high volume; broad rewrites can create noisy diffs.
- Misclassifying cross-crate runbooks as crate-owned can fragment shared guidance.
- CI integration via hidden skill scripts can drift if script paths change without audits.
- Stubs without explicit lifecycle policy can accumulate and obscure canonical doc paths.

## Rollback plan
- Use two-phase rollback boundary:
  1. Revert enforcement wiring (`docs-placement-audit` in checks/CI) if it blocks urgent development.
  2. Revert specific docs migration commits while preserving inventory and policy records.
- Keep root compatibility stubs in place during rollback to avoid broken links.
- If migration introduces contributor confusion, temporarily keep both references (canonical + stub) in high-impact docs and re-run link hygiene checks before final cleanup.

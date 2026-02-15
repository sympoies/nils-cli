# Plan: New CLI crate development standardization via agent-docs

## Overview
This plan defines a project-level development standard for creating new CLI crates in this
workspace and registers that standard into `agent-docs` so it is always discoverable in
`project-dev` flows. The standard must require two parallel output contracts: human-readable CLI
output and service-consumable JSON contracts with stable schema/versioning expectations. It also
must enforce publish-ready crate conventions already used by current publishable crates in this
repo, including Cargo metadata consistency, release-order integration, and required validation
gates.

## Scope
- In scope: authoring a canonical "new CLI crate standard" document, defining JSON contract policy
  (including error envelopes), wiring the document into `AGENT_DOCS.toml` for `project-dev`,
  adding lightweight enforcement checks, and documenting publish-readiness requirements.
- Out of scope: implementing a specific new CLI crate feature set, changing existing CLI behavior,
  or altering crates.io/publishing infrastructure beyond policy-check integration.

## Assumptions (if any)
1. The standard should apply to all future publishable CLI crates, not just one crate.
2. Default human-readable CLI output remains required, and JSON mode is an explicit machine-facing
   contract rather than a replacement.
3. `agent-docs` project-level strict resolution is the required discovery mechanism for this
   guidance.
4. Existing repository release flow (`scripts/publish-crates.sh`, `release/crates-io-publish-order.txt`)
   remains authoritative.

## Success Criteria
1. `project-dev` strict resolve includes the new standard document with `status=present`.
2. The standard explicitly defines both human-readable and JSON contract requirements for new CLI
   crates.
3. The standard includes publish-ready checklist items aligned with current workspace conventions.
4. Required repo checks continue to pass with the new policy wiring.

## Sprint 1: Baseline inventory and contract decisions
**Goal**: Freeze what "new CLI crate standard" means in this repository before writing policy docs.
**Demo/Validation**:
- Command(s): `plan-tooling validate --file docs/plans/new-cli-crate-agent-docs-standardization-plan.md`
- Verify: plan validates with complete dependencies and no placeholders.

### Task 1.1: Inventory current publishable crate conventions
- **Location**:
  - `Cargo.toml`
  - `crates/cli-template/Cargo.toml`
  - `crates/codex-cli/Cargo.toml`
  - `release/crates-io-publish-order.txt`
  - `scripts/publish-crates.sh`
- **Description**: Capture the repository’s publishable crate baseline (package metadata fields,
  naming/version conventions, workspace dependency pinning style, release-order requirements, and
  dry-run publish workflow) and define which items are mandatory for new CLI crates.
- **Dependencies**:
  - none
- **Complexity**: 5
- **Acceptance criteria**:
  - Mandatory Cargo metadata fields are explicitly enumerated.
  - Policy distinguishes publishable crates from `publish = false` support crates.
  - Release-order and dry-run commands are explicitly included.
- **Validation**:
  - `rg -n "edition.workspace|license.workspace|description|repository|publish = false" Cargo.toml crates/*/Cargo.toml`
  - `rg -n "release/crates-io-publish-order.txt|--dry-run|--publish" scripts/publish-crates.sh`

### Task 1.2: Define dual output contract policy (human + JSON)
- **Location**:
  - `crates/codex-cli/README.md`
  - `docs/plans/codex-cli-service-consumable-diag-auth-plan.md`
- **Description**: Extract reusable policy rules for requiring both human-readable output and
  machine-consumable JSON contracts: opt-in JSON mode, schema versioning, stable keys, and
  structured error envelopes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Policy defines JSON contract minimum fields and failure envelope fields.
  - Policy defines when text mode and JSON mode are each required.
  - Policy requires contract tests for JSON-facing commands.
- **Validation**:
  - `rg -n "schema_version|machine-consumable|error envelope|human-readable" crates/codex-cli/README.md docs/plans/codex-cli-service-consumable-diag-auth-plan.md`

### Task 1.3: Define agent-docs integration strategy for the new standard
- **Location**:
  - `AGENT_DOCS.toml`
  - `crates/agent-docs/README.md`
- **Description**: Decide the exact registration model for the new standard document in
  `project-dev` context (`required`, `scope`, `when`, and `notes`) so strict preflight always
  exposes it.
- **Dependencies**:
  - Task 1.1
  - Task 1.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Registration strategy specifies exact `agent-docs add` parameters.
  - Strategy explains why the document should be required vs optional.
  - Strategy includes strict resolve verification command.
- **Validation**:
  - `rg -n "\\[\\[document\\]\\]|context = \"project-dev\"|required = true" AGENT_DOCS.toml`
  - `rg -n "agent-docs add|resolve --context project-dev --strict" crates/agent-docs/README.md`

### Task 1.4: Create a policy coverage matrix for new CLI crates
- **Location**:
  - `docs/runbooks/new-cli-crate-development-standard.md`
- **Description**: Define the section matrix that the standard must cover: crate scaffold,
  argument/help behavior, text output rules, JSON contract rules, completion/wrapper expectations,
  test/coverage gates, and publish-readiness checklist.
- **Dependencies**:
  - Task 1.2
  - Task 1.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Matrix includes both behavioral and operational requirements.
  - Matrix includes explicit verification commands for each section.
  - Matrix contains no unresolved placeholders.
- **Validation**:
  - `rg -n "crate scaffold|JSON contract|publish-readiness|completion|coverage" docs/runbooks/new-cli-crate-development-standard.md`

## Sprint 2: Author canonical standards and register in agent-docs
**Goal**: Produce the actual standard documents and make them mandatory in project-dev discovery.
**Demo/Validation**:
- Command(s): `agent-docs resolve --context project-dev --strict --format checklist`
- Verify: checklist includes the new standard doc as required and present.

### Task 2.1: Author canonical new CLI crate development standard runbook
- **Location**:
  - `docs/runbooks/new-cli-crate-development-standard.md`
- **Description**: Write the canonical runbook for creating a new CLI crate in this repo, including:
  scaffold sequence, CLI UX requirements, output/exit-code policy, JSON contract expectations,
  testing/coverage requirements, and publish-readiness checklist.
- **Dependencies**:
  - Task 1.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Runbook includes a complete step-by-step workflow from crate creation to release readiness.
  - Runbook includes mandatory check commands from `DEVELOPMENT.md`.
  - Runbook includes explicit "human output + JSON contract" requirements.
- **Validation**:
  - `rg -n "^## Workflow$|^## Output contracts$|^## Publish readiness$|cargo fmt --all -- --check|cargo clippy --all-targets --all-features -- -D warnings|cargo test --workspace" docs/runbooks/new-cli-crate-development-standard.md`

### Task 2.2: Author reusable JSON contract guideline for CLI services
- **Location**:
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
- **Description**: Create a reusable JSON contract guideline document for CLI commands consumed by
  services, including schema versioning strategy, required envelope fields, error payload format,
  compatibility policy, and contract-test expectations.
- **Dependencies**:
  - Task 1.2
  - Task 2.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Guideline defines mandatory top-level fields and structured error fields.
  - Guideline defines backward-compatibility rules for key changes.
  - Guideline includes example success/failure payloads.
- **Validation**:
  - `rg -n "\"schema_version\"|\"ok\"|\"results\"|\"code\"|\"message\"|compatibility" docs/specs/cli-service-json-contract-guideline-v1.md`

### Task 2.3: Register the standard doc in project AGENT_DOCS
- **Location**:
  - `AGENT_DOCS.toml`
- **Description**: Add a required project-level `project-dev` document entry for the new standard
  so strict agent-doc preflight always loads it.
- **Dependencies**:
  - Task 1.3
  - Task 2.1
- **Complexity**: 3
- **Acceptance criteria**:
  - `AGENT_DOCS.toml` contains one required entry for the new standard document.
  - The entry uses project scope and a clear note for new CLI crate work.
  - Strict resolve reports the document as present.
- **Validation**:
  - `agent-docs resolve --context project-dev --strict --format checklist | rg "new-cli-crate-development-standard\\.md|REQUIRED_DOCS_END"`

### Task 2.4: Update project onboarding docs to reference the standard
- **Location**:
  - `DEVELOPMENT.md`
  - `AGENTS.md`
- **Description**: Add explicit references to the new runbook and JSON guideline in project docs so
  contributors can discover the policy without needing prior context.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
  - Task 2.3
- **Complexity**: 4
- **Acceptance criteria**:
  - `DEVELOPMENT.md` links to the new standard runbook and JSON guideline.
  - `AGENTS.md` points to agent-docs-based retrieval for the new standard.
  - References are concise and non-duplicative.
- **Validation**:
  - `rg -n "new-cli-crate-development-standard|cli-service-json-contract-guideline-v1|agent-docs resolve --context project-dev" DEVELOPMENT.md AGENTS.md`

## Sprint 3: Enforcement and template alignment
**Goal**: Reduce policy drift by adding lightweight checks and aligning the template crate guidance.
**Demo/Validation**:
- Command(s): `bash scripts/ci/cli-crate-policy-check.sh`
- Verify: script exits 0 and reports no policy violations for existing publishable CLI crates.

### Task 3.1: Implement CLI crate policy check script
- **Location**:
  - `scripts/ci/cli-crate-policy-check.sh`
- **Description**: Add a CI-friendly policy script that checks publishable CLI crates for required
  metadata/doc structure: Cargo package fields, README presence, bin target declaration, and release
  list inclusion.
- **Dependencies**:
  - Task 1.1
  - Task 2.1
  - Task 2.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Script fails with actionable errors on missing mandatory metadata or docs.
  - Script ignores known non-publish helper crates where appropriate.
  - Script output is stable enough for CI usage.
- **Validation**:
  - `bash scripts/ci/cli-crate-policy-check.sh`

### Task 3.2: Wire policy check into required checks entrypoint
- **Location**:
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `.agents/skills/nils-cli-checks/SKILL.md`
- **Description**: Integrate the new policy script into the required checks flow so violations are
  caught before delivery and release.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 5
- **Acceptance criteria**:
  - `nils-cli-checks.sh` runs policy check before completion/zsh checks.
  - Skill documentation lists the new check explicitly.
  - Failure mode clearly reports which policy check failed.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

### Task 3.3: Align `cli-template` docs with the new standard
- **Location**:
  - `crates/cli-template/README.md`
- **Description**: Update template README so it demonstrates the expected structure for new CLI
  crates, including text output contract and service JSON contract guidance references.
- **Dependencies**:
  - Task 2.1
  - Task 2.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Template README references the new standard and JSON guideline.
  - Template README includes explicit output-contract section.
  - Guidance remains minimal and reusable for future crates.
- **Validation**:
  - `rg -n "Output contract|JSON contract|new-cli-crate-development-standard" crates/cli-template/README.md`

### Task 3.4: Add policy-check regression tests
- **Location**:
  - `scripts/ci/tests/cli-crate-policy-check.bats`
  - `scripts/ci/tests/common.sh`
- **Description**: Add regression tests for the policy script to ensure it correctly reports both
  compliant and non-compliant cases.
- **Dependencies**:
  - Task 3.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Test suite covers at least one positive and one negative scenario.
  - Script behavior changes are captured by tests before release.
  - Tests run non-interactively in CI environments.
- **Validation**:
  - `bash scripts/ci/tests/cli-crate-policy-check.bats`

## Sprint 4: Adoption hardening and release safety
**Goal**: Ensure the new standard is operationally usable and rollback-safe.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- Verify: required checks pass and docs are discoverable via strict agent-docs resolve.

### Task 4.1: Add end-to-end policy adoption verification
- **Location**:
  - `docs/runbooks/new-cli-crate-development-standard.md`
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
- **Description**: Add an end-to-end verification section showing how a contributor validates a new
  CLI crate against the standard from scaffold to publish dry-run.
- **Dependencies**:
  - Task 3.2
  - Task 3.3
- **Complexity**: 4
- **Acceptance criteria**:
  - Runbook contains executable verification sequence.
  - Sequence includes strict agent-doc resolve and publish dry-run steps.
  - Sequence includes JSON contract test expectations.
- **Validation**:
  - `rg -n "agent-docs resolve --context project-dev --strict|scripts/publish-crates.sh --dry-run|contract test" docs/runbooks/new-cli-crate-development-standard.md`

### Task 4.2: Validate release/readiness docs remain consistent
- **Location**:
  - `DEVELOPMENT.md`
  - `BINARY_DEPENDENCIES.md`
  - `.agents/skills/nils-cli-release/SKILL.md`
- **Description**: Cross-check that development, dependency, and release docs do not conflict with
  the new CLI crate standard and JSON contract policy.
- **Dependencies**:
  - Task 2.4
  - Task 4.1
- **Complexity**: 3
- **Acceptance criteria**:
  - No contradictory instructions remain across docs.
  - Release skill docs still align with publishability checklist.
  - Any policy exceptions are documented explicitly.
- **Validation**:
  - `rg -n "publish|release|JSON|contract|project-dev" DEVELOPMENT.md BINARY_DEPENDENCIES.md .agents/skills/nils-cli-release/SKILL.md`

### Task 4.3: Execute full required checks and summarize residual risks
- **Location**:
  - `DEVELOPMENT.md`
  - `.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
- **Description**: Run required checks and produce a short rollout summary covering residual risks,
  especially policy-check false positives and JSON contract drift risk.
- **Dependencies**:
  - Task 3.4
  - Task 4.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Required check entrypoint exits 0.
  - Rollout summary documents known limitations and owner follow-ups.
  - Policy is ready for default use in new CLI crate work.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Dependency and parallelization map
- Critical path:
  - Task 1.1 -> Task 1.2 -> Task 1.4 -> Task 2.1 -> Task 2.3 -> Task 2.4 -> Task 3.1 -> Task 3.2 -> Task 4.1 -> Task 4.3
- Parallelizable lanes:
  - Task 2.2 can run in parallel with Task 2.3 after Task 2.1 baseline text is stable.
  - Task 3.3 can run in parallel with Task 3.1 once canonical docs are merged.
  - Task 4.2 can run in parallel with Task 4.1 after Sprint 3 completion.

## Testing Strategy
- Unit: policy-check logic tests for required metadata/rules and edge-case exclusions.
- Integration: `agent-docs resolve --context project-dev --strict --format checklist` after
  registration changes.
- E2E/manual: simulate "new CLI crate checklist" flow from runbook using template crate plus
  publish dry-run command.

## Risks & gotchas
- Overly strict policy checks can block legitimate crates with different constraints. Mitigation:
  define explicit allow-list and documented exceptions.
- JSON contract rules can be interpreted inconsistently across crates. Mitigation: single guideline
  document + contract tests requirement.
- Agent-doc registration can fail strict preflight if path changes. Mitigation: keep stable path and
  verify with checklist resolve in CI/check scripts.
- Cross-document drift can reintroduce conflicting instructions. Mitigation: add periodic consistency
  check in required checks.

## Rollback plan
- Trigger criteria:
  - Policy check blocks two or more unrelated PRs in one week due to false positives.
  - Strict preflight fails on default contributor setup because of policy-doc registration drift.
- Owner:
  - `nils-cli` maintainers for policy-check gating and `AGENT_DOCS.toml` registration decisions.
- Rollback steps:
  - Keep the runbook and JSON guideline, but make policy check non-blocking temporarily.
  - Revert `nils-cli-checks.sh` integration first while preserving documentation updates.
  - Keep `AGENT_DOCS.toml` registration intact unless strict preflight regressions require emergency
    downgrade to optional.
- Roll-forward criteria:
  - Re-enable blocking enforcement only after false-positive cases are covered by regression tests
    and required checks pass in CI.

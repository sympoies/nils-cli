# Plan: Workspace Documentation Cleanup and Consolidation

## Overview
This plan performs a full documentation cleanup across the workspace with strict ownership boundaries: workspace-level docs stay in root `docs/`, and crate-local docs stay under `crates/<crate>/docs/`.  
The cleanup explicitly removes development-phase records that are no longer needed, consolidates duplicated guidance, and reduces over-linked crate docs to keep navigation concise.  
The approach is safety-first: generate a keep/move/merge/remove manifest, preserve CI-critical docs, execute changes in bounded batches, then run docs governance checks.  
Assumption: `docs/reports/completion-coverage-matrix.md` remains protected unless all script/test dependencies are migrated in the same change set.

## Scope
- In scope: remove stale development-only records under `docs/` and `crates/*/docs/` when no longer operationally needed.
- In scope: move any crate-local canonical docs from root `docs/` to `crates/<crate>/docs/...` and keep only compatibility stubs when required.
- In scope: deduplicate overlapping root/crate guidance and reduce unnecessary deep cross-links in crate docs.
- In scope: keep docs placement policy and validation workflow enforceable in CI.
- Out of scope: CLI behavior changes, runtime feature work, or schema changes unrelated to docs cleanup.
- Out of scope: removing CI-consumed canonical docs (for example completion matrix) without coordinated script/test migration.

## Assumptions (if any)
1. Root compatibility stubs are retained only when inbound references require stable historical paths; otherwise old paths are removed.
2. Cleanup applies to tracked Markdown docs (`*.md`) in root, `docs/`, and `crates/*/docs/`.
3. Temporary analysis artifacts live in `$AGENT_HOME/out/nils-cli-docs-cleanup/` and are not committed as canonical documentation.

## Sprint 1: Baseline and Cleanup Contract
**Goal**: Establish deterministic ownership/action decisions before deleting or moving any docs.
**Demo/Validation**:
- Command(s): `find docs crates -type f -name '*.md' | sort`
- Verify: every Markdown file is classified once with a traceable action and rationale.

### Task 1.1: Build full docs inventory and ownership classification
- **Location**:
  - `docs/specs/crate-docs-placement-policy.md`
  - `docs/runbooks/cli-completion-development-standard.md`
  - `docs/reports/completion-coverage-matrix.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/gemini-cli/docs/README.md`
  - `crates/memo-cli/docs/README.md`
  - `$AGENT_HOME/out/nils-cli-docs-cleanup/inventory.md`
  - `$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv`
- **Description**: Create a complete inventory with ownership (`workspace-level` or `crate-local`), lifecycle state (`canonical`, `compat-stub`, `transient-dev-record`), and action (`keep`, `move`, `merge`, `remove`, `stub`).
- **Dependencies**:
  - none
- **Complexity**: 4
- **Acceptance criteria**:
  - Every tracked Markdown file under `docs/` and `crates/*/docs/` appears exactly once in the action table.
  - Each entry includes ownership, lifecycle state, action, and rationale.
  - CI-bound files are explicitly marked as protected in the action table.
- **Validation**:
  - `mkdir -p "$AGENT_HOME/out/nils-cli-docs-cleanup"`
  - `find docs crates -type f -name '*.md' | sort > "$AGENT_HOME/out/nils-cli-docs-cleanup/all-md.txt"`
  - `test -f "$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv"`
  - `awk -F '\t' 'NR>1 {print $1}' "$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv" | sort > "$AGENT_HOME/out/nils-cli-docs-cleanup/actions-paths.txt"`
  - `comm -3 "$AGENT_HOME/out/nils-cli-docs-cleanup/all-md.txt" "$AGENT_HOME/out/nils-cli-docs-cleanup/actions-paths.txt" | sed '/^$/d' | wc -l | rg '^0$'`
  - `for p in docs/reports/completion-coverage-matrix.md docs/specs/crate-docs-placement-policy.md docs/runbooks/cli-completion-development-standard.md; do rg -n "^${p}\t.*\tprotected" "$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv"; done`

### Task 1.2: Codify retention and cross-link rules in canonical governance docs
- **Location**:
  - `docs/specs/crate-docs-placement-policy.md`
  - `DEVELOPMENT.md`
- **Description**: Extend governance docs with a lifecycle rule for development-only docs, explicit compatibility-stub retention criteria, and required hygiene checks for ongoing prevention.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 5
- **Acceptance criteria**:
  - Placement policy includes explicit lifecycle handling for transient development docs.
  - Placement policy states when to keep/remove compatibility stubs.
  - Development guide references the hygiene audit command in required checks.
- **Validation**:
  - `rg -n 'transient|development-only|lifecycle|compatibility stub' docs/specs/crate-docs-placement-policy.md`
  - `rg -n 'docs-hygiene-audit|docs-placement-audit' DEVELOPMENT.md`

### Task 1.3: Produce executable cleanup manifest and change order
- **Location**:
  - `$AGENT_HOME/out/nils-cli-docs-cleanup/cleanup-manifest.md`
- **Description**: Materialize the exact file-level cleanup order (remove/move/merge/stub) with dependency notes so implementation can run in deterministic batches.
- **Dependencies**:
  - `Task 1.1`
  - `Task 1.2`
- **Complexity**: 3
- **Acceptance criteria**:
  - Manifest includes every file action and target canonical path (when moved/merged).
  - Manifest includes dependency ordering and rollback notes for risky removals.
  - Manifest is directly usable as implementation checklist.
- **Validation**:
  - `test -f "$AGENT_HOME/out/nils-cli-docs-cleanup/cleanup-manifest.md"`
  - `rg -n '^\| .* \| (keep|move|merge|remove|stub) \|' "$AGENT_HOME/out/nils-cli-docs-cleanup/cleanup-manifest.md"`

## Sprint 2: Root Docs Cleanup and Consolidation
**Goal**: Keep root docs lean and workspace-scoped, removing stale records and moving crate-local canonicals out of root.
**Demo/Validation**:
- Command(s): `bash scripts/ci/docs-placement-audit.sh --strict`
- Verify: root docs only contain workspace-level canonical content plus approved compatibility stubs.

### Task 2.1: Remove obsolete root development records
- **Location**:
  - `$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv` (root `docs/` entries classified as `remove`)
- **Description**: Delete root-level development records classified as `remove` after confirming they are not contract-critical and have no remaining inbound references.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Root docs marked `remove` are deleted.
  - Deleted paths are no longer referenced from tracked files.
  - Protected docs remain present.
- **Validation**:
  - `test -f docs/reports/completion-coverage-matrix.md`
  - `awk -F '\t' 'NR>1 && $2=="remove" && $1 ~ /^docs\\// {print $1}' "$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv" > "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-root.txt"`
  - `if [ -s "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-root.txt" ]; then while read -r p; do test ! -e "$p" || { echo "still exists: $p"; exit 1; }; done < "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-root.txt"; fi`
  - `if [ -s "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-root.txt" ]; then while read -r p; do rg -n --fixed-strings "$p" . && { echo "stale references: $p"; exit 1; }; done < "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-root.txt"; fi`

### Task 2.2: Move root crate-local docs to owning crates, with stubs only when required
- **Location**:
  - `docs/specs/codex-gemini-runtime-contract.md`
  - `docs/specs/codex-gemini-cli-parity-contract-v1.md`
  - `crates/image-processing/docs/runbooks/llm-svg-workflow.md`
  - `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md`
- **Description**: Relocate crate-owned canonical docs from root paths to crate-local canonical paths and keep root stubs only for compatibility-required routes.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 7
- **Acceptance criteria**:
  - Root canonical crate-local docs are removed from `docs/`.
  - Remaining root stubs are redirect-only with `Moved to` and migration metadata.
  - Moved docs have updated inbound links.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `for f in docs/runbooks/*.md docs/specs/*.md; do [ -f "$f" ] || continue; if rg -q '^Moved to:' "$f"; then rg -n '^Moved to:|^Migration metadata:' "$f"; fi; done`

### Task 2.3: Consolidate duplicated root guidance and simplify navigation
- **Location**:
  - `README.md`
  - `docs/runbooks/new-cli-crate-development-standard.md`
  - `docs/runbooks/cli-completion-development-standard.md`
  - `docs/specs/crate-docs-placement-policy.md`
- **Description**: Remove duplicated procedural text across root docs, keep one canonical source per topic, and make root navigation concise.
- **Dependencies**:
  - `Task 2.1`
  - `Task 2.2`
- **Complexity**: 5
- **Acceptance criteria**:
  - Root docs use references to canonical sources rather than duplicated long-form procedures.
  - Root README remains concise and points to canonical runbooks/specs.
  - No exact duplicated Markdown payload remains across root docs.
- **Validation**:
  - `rg -n 'canonical|source of truth|see .*docs/' README.md docs/runbooks/*.md docs/specs/*.md`
  - `find README.md docs/runbooks docs/specs -type f -name '*.md' -print0 | xargs -0 shasum | awk '{print $1}' | sort | uniq -d | wc -l | rg '^0$'`

## Sprint 3: Crate Docs Cleanup and De-duplication
**Goal**: Keep crate docs crate-centric, concise, and free of stale rollout/adoption records and excessive cross-layer references.
**Demo/Validation**:
- Command(s): `find crates -type f -path '*/docs/*' -name '*.md' | sort`
- Verify: crate docs contain only active, non-duplicated, crate-relevant documentation.

### Task 3.1: Clean rollout/adoption docs in crates to active-state docs only
- **Location**:
  - `crates/api-test/docs/runbooks/api-test-websocket-adoption.md`
  - `crates/api-websocket/docs/runbooks/api-websocket-rollout.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-rollout.md`
  - `crates/api-test/README.md`
  - `crates/api-websocket/README.md`
  - `crates/memo-cli/README.md`
  - `crates/api-test/docs/README.md`
  - `crates/api-websocket/docs/README.md`
  - `crates/memo-cli/docs/README.md`
- **Description**: For each crate rollout/adoption runbook, either remove it (if fully historical) or rewrite it as stable operational guidance; update all crate links accordingly.
- **Dependencies**:
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Historical-only rollout/adoption docs marked `remove` are deleted.
  - Any retained runbook is rewritten for steady-state operations (not one-time rollout logging).
  - No broken links remain in crate README/docs index files.
- **Validation**:
  - `awk -F '\t' 'NR>1 && $2=="remove" && $1 ~ /^crates\\/.+\\/docs\\// {print $1}' "$AGENT_HOME/out/nils-cli-docs-cleanup/actions.tsv" > "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-crate-docs.txt"`
  - `if [ -s "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-crate-docs.txt" ]; then while read -r p; do test ! -e "$p" || { echo "still exists: $p"; exit 1; }; done < "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-crate-docs.txt"; fi`
  - `if [ -s "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-crate-docs.txt" ]; then while read -r p; do rg -n --fixed-strings "$p" crates README.md docs && { echo "stale references: $p"; exit 1; }; done < "$AGENT_HOME/out/nils-cli-docs-cleanup/removed-crate-docs.txt"; fi`

### Task 3.2: Remove duplicated codex/gemini consumer guidance via shared canonical source
- **Location**:
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
  - `crates/codex-cli/docs/runbooks/json-consumers.md`
  - `crates/gemini-cli/docs/runbooks/json-consumers.md`
- **Description**: Move generic consumer parsing/retry guidance to a shared canonical workspace doc, leaving crate runbooks focused on provider-specific contract deltas.
- **Dependencies**:
  - `Task 2.3`
- **Complexity**: 7
- **Acceptance criteria**:
  - Shared guidance exists in one workspace-level canonical doc.
  - Codex/Gemini runbooks include provider-specific schema rules and link to shared guidance.
  - Duplicate long-form sections between the two runbooks are removed.
- **Validation**:
  - `test -f docs/specs/cli-service-json-contract-guideline-v1.md`
  - `for f in crates/codex-cli/docs/runbooks/json-consumers.md crates/gemini-cli/docs/runbooks/json-consumers.md; do rg -n 'cli-service-json-contract-guideline-v1.md' "$f"; done`
  - `for f in crates/codex-cli/docs/runbooks/json-consumers.md crates/gemini-cli/docs/runbooks/json-consumers.md; do rg -n 'schema_version|auth\\.v1|diag\\.rate-limits\\.v1' "$f"; done`
  - `wc -l crates/codex-cli/docs/runbooks/json-consumers.md crates/gemini-cli/docs/runbooks/json-consumers.md`

### Task 3.3: Minimize deep cross-layer links in crate docs indexes
- **Location**:
  - `crates/codex-cli/docs/README.md`
  - `crates/gemini-cli/docs/README.md`
  - `crates/api-test/docs/README.md`
  - `crates/api-websocket/docs/README.md`
  - `crates/memo-cli/docs/README.md`
  - `crates/codex-cli/README.md`
  - `crates/gemini-cli/README.md`
  - `crates/api-test/README.md`
  - `crates/api-websocket/README.md`
  - `crates/memo-cli/README.md`
- **Description**: Keep crate docs indexes focused on crate-local docs and allow only essential workspace-level links that are contract-critical.
- **Dependencies**:
  - `Task 3.2`
- **Complexity**: 5
- **Acceptance criteria**:
  - Unnecessary deep relative links are removed.
  - Any remaining deep cross-layer links are explicitly justified and allowlisted.
  - Every crate docs index still presents complete local navigation.
- **Validation**:
  - `if rg -n '\\.\\./\\.\\./\\.\\./docs/' crates/*/docs/README.md | rg -v 'codex-gemini-cli-parity-contract-v1.md'; then echo 'unexpected deep cross-layer link'; exit 1; fi`
  - `for f in crates/*/docs/README.md; do rg -n '^## Specs|^## Runbooks|^## Reports|^## Links' "$f"; done`

### Task 3.4: Merge or remove duplicate crate/root docs by single-owner canonical path
- **Location**:
  - `docs/specs/cli-service-json-contract-guideline-v1.md`
  - `crates/codex-cli/docs/runbooks/json-consumers.md`
  - `crates/gemini-cli/docs/runbooks/json-consumers.md`
  - `crates/api-test/docs/runbooks/api-test-websocket-adoption.md`
  - `crates/api-websocket/docs/runbooks/api-websocket-rollout.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-rollout.md`
- **Description**: For overlapping docs with the same ownership/topic, keep one canonical file, merge required content, and remove or stub superseded paths.
- **Dependencies**:
  - `Task 3.1`
  - `Task 3.2`
  - `Task 3.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Each topic has one canonical owner/path.
  - Superseded files are removed or converted to policy-compliant stubs.
  - No references point to superseded paths.
- **Validation**:
  - `find docs crates/*/docs -type f -name '*.md' -print0 | xargs -0 shasum | awk '{print $1}' | sort | uniq -d | wc -l | rg '^0$'`
  - `bash scripts/ci/docs-placement-audit.sh --strict`

## Sprint 4: Verification and Non-Regression Guardrails
**Goal**: Validate cleanup integrity and prevent future doc sprawl.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
- Verify: required docs checks pass and hygiene checks become repeatable.

### Task 4.1: Run required checks with docs-only/full-check branching
- **Location**:
  - `scripts/ci/docs-placement-audit.sh`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- **Description**: Execute docs-placement and required checks according to actual changed-file class (`docs-only` vs non-doc changes).
- **Dependencies**:
  - `Task 2.3`
  - `Task 3.4`
- **Complexity**: 4
- **Acceptance criteria**:
  - Docs-placement strict audit passes.
  - Docs-only path is used only when all changed files qualify.
  - Full required checks are executed when non-doc files are changed.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
  - `if git diff --name-only | rg -n -v '^(README\\.md|DEVELOPMENT\\.md|AGENTS\\.md|BINARY_DEPENDENCIES\\.md|docs/|crates/[^/]+/docs/)'; then ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh; fi`

### Task 4.2: Add CI docs-hygiene audit for sprawl prevention
- **Location**:
  - `scripts/ci/docs-hygiene-audit.sh`
  - `DEVELOPMENT.md`
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- **Description**: Add a deterministic CI audit for stale doc references, disallowed root crate-local canonicals, and unexpected deep cross-layer links.
- **Dependencies**:
  - `Task 1.2`
  - `Task 3.4`
- **Complexity**: 8
- **Acceptance criteria**:
  - Hygiene audit exits non-zero on policy violations and zero on clean state.
  - Required checks entrypoint includes the hygiene audit.
  - Development guide documents when/how to run hygiene audit.
- **Validation**:
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`

## Dependency and Parallel Execution Map
- Initial setup: `Task 1.1` -> `Task 1.2` -> `Task 1.3`
- Parallel batch A after `Task 1.3`: `Task 2.1`, `Task 2.2`, `Task 3.1`
- Batch B after root cleanup convergence: `Task 2.3`
- Parallel batch C after `Task 2.3`: `Task 3.2` and `Task 4.2` (script scaffold can start while runbook dedupe proceeds)
- Batch D: `Task 3.3` -> `Task 3.4`
- Final gate: `Task 4.1` (depends on consolidated docs state and hygiene checks wired)

## Testing Strategy
- Unit:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `bash scripts/ci/docs-hygiene-audit.sh --strict`
- Integration:
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh --docs-only`
  - `if git diff --name-only | rg -n -v '^(README\\.md|DEVELOPMENT\\.md|AGENTS\\.md|BINARY_DEPENDENCIES\\.md|docs/|crates/[^/]+/docs/)'; then ./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh; fi`
- E2E/manual:
  - `find docs crates -type f -name '*.md' | sort`
  - `find docs crates -type f -name '*.md' -print0 | xargs -0 rg -n '\\]\\((\\.\\./){2,}|/docs/|docs/)'`
  - Manual review: root `README.md` and each `crates/*/docs/README.md` render clear navigation without redundant links.

## Risks & gotchas
- Removing historical docs too aggressively can break scripts/tests that parse specific report files.
- Compatibility stubs can drift into canonical content if not kept redirect-only.
- Deduplicating codex/gemini consumer docs can accidentally remove provider-specific contract details.
- Deep link cleanup may introduce broken relative paths when docs are moved across ownership boundaries.
- Hygiene scripts can create noise if allowlists are too strict or under-specified.

## Rollback plan
- Execute cleanup in small commits by sprint/task group so each docs cluster can be reverted independently.
- Keep a pre-cleanup path snapshot from `Task 1.1`; if a removal is wrong, restore exact files and links from git history.
- If CI breaks due to removed “report” docs, restore the required file immediately and defer removal until dependent scripts/tests are migrated.
- If cross-link cleanup causes broken navigation, restore prior README/docs index links first, then re-attempt with narrower link-scope changes.
- If hygiene audit introduces false positives, gate it as non-blocking initially, tune allowlist rules, then re-enable strict mode.

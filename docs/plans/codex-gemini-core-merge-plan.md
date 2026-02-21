# Plan: Merge codex-core and gemini-core into CLI crates with shared runtime foundation

## Overview
This plan consolidates `codex-core` into `codex-cli` and `gemini-core` into `gemini-cli` while preserving current CLI contracts (output text, JSON schema IDs, and exit semantics).  
The implementation avoids copy-paste divergence by first extracting provider-neutral runtime primitives into `nils-common`, then wiring each CLI through a provider profile adapter.  
Both CLI lanes are migrated with parallelizable sprints, followed by a single workspace cleanup sprint that removes both core crates together.  
Assumption: no external workspace crate depends on `codex-core` or `gemini-core` at build time.

## Scope
- In scope: remove standalone `codex-core` and `gemini-core` crates from the workspace, keep only `codex-cli` and `gemini-cli`, and maximize shared provider-neutral runtime code.
- In scope: preserve provider-specific behavior differences (env var names, default model values, auth/cache path conventions, upstream command shapes).
- In scope: strengthen codex-vs-gemini parity tests for command topology and contract-level behavior.
- In scope: perform post-migration documentation cleanup, remove outdated/duplicated migration artifacts, and keep one canonical latest parity document.
- In scope: correct `README.md`, `crates/codex-cli/README.md`, and `crates/gemini-cli/README.md` to match the merged architecture.
- Out of scope: changing public command surfaces, renaming binary names, or introducing new user-facing features.
- Out of scope: changing JSON schema version identifiers (`codex-cli.*`, `gemini-cli.*`).

## Assumptions (if any)
1. Current `codex-core` and `gemini-core` are consumed only by their corresponding CLI crates inside this workspace.
2. Crates published historically on crates.io can remain historical artifacts; forward workspace releases no longer publish `nils-gemini-core`.
3. CLI behavior parity means identical command topology and error semantics where expected, while provider-specific strings remain provider-specific.

## Sprint 1: Shared Runtime Foundation and Parity Guardrails
**Goal**: Define and implement a provider-runtime boundary in `nils-common` so both CLI migrations can proceed in parallel without reintroducing drift.
**Demo/Validation**:
- Command(s): `cargo test -p nils-common`
- Verify: shared runtime module compiles/tests cleanly and both CLI crates can consume it via `cargo check`.

### Task 1.1: Define runtime contract matrix and provider profile invariants
- **Location**:
  - `docs/specs/codex-gemini-runtime-contract.md`
  - `crates/codex-core/src/config.rs`
  - `crates/gemini-core/src/config.rs`
  - `crates/codex-core/src/exec.rs`
  - `crates/gemini-core/src/exec.rs`
  - `crates/codex-core/src/paths.rs`
  - `crates/gemini-core/src/paths.rs`
- **Description**: Document what must stay provider-specific (env keys, defaults, path precedence, command argument shape) versus what can be centralized as domain-neutral runtime logic.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Contract document enumerates every provider-specific field consumed by config, paths, and exec flows.
  - Contract document explicitly states non-negotiable compatibility rules for CLI output and exit behavior.
  - Contract document is referenced by both migration sprints as the single source of truth.
- **Validation**:
  - `test -f docs/specs/codex-gemini-runtime-contract.md`
  - `rg -n "^## Provider-specific matrix$|^## Compatibility rules$" docs/specs/codex-gemini-runtime-contract.md`
  - `cargo test -p nils-codex-core --test paths_config_contract --test exec_contract`
  - `cargo test -p nils-gemini-core --test paths_config_contract --test exec_contract`

### Task 1.2: Implement provider-neutral runtime primitives in nils-common
- **Location**:
  - `crates/nils-common/src/lib.rs`
  - `crates/nils-common/src/provider_runtime/mod.rs`
  - `crates/nils-common/src/provider_runtime/error.rs`
  - `crates/nils-common/src/provider_runtime/auth.rs`
  - `crates/nils-common/src/provider_runtime/json.rs`
  - `crates/nils-common/src/provider_runtime/jwt.rs`
  - `crates/nils-common/src/provider_runtime/config.rs`
  - `crates/nils-common/src/provider_runtime/exec.rs`
  - `crates/nils-common/src/provider_runtime/paths.rs`
- **Description**: Move shared core logic into `nils-common` with profile-driven APIs so CLIs inject provider specifics without forking logic.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 7
- **Acceptance criteria**:
  - Shared runtime APIs are domain-neutral and contain no provider brand strings.
  - Provider profile structs capture every required provider-specific knob used by codex and gemini.
  - Unit tests in `nils-common` cover success and failure paths for runtime helpers.
- **Validation**:
  - `cargo test -p nils-common`
  - `cargo check -p nils-codex-cli -p nils-gemini-cli`

### Task 1.3: Establish bidirectional parity oracles for both CLI lanes
- **Location**:
  - `crates/gemini-cli/tests/parity_oracle.rs`
  - `crates/codex-cli/tests/parity_oracle.rs`
  - `crates/nils-test-support/src/bin.rs`
- **Description**: Normalize parity tests so codex and gemini both assert command-topology parity, legacy redirect parity, and schema-field parity from opposite directions.
- **Dependencies**:
  - `Task 1.1`
- **Complexity**: 5
- **Acceptance criteria**:
  - Both CLI test suites include parity-oracle coverage for `--help`, legacy redirects, and auth/diag JSON mode.
  - Parity assertions pin expected provider-specific schema IDs while keeping structure parity.
  - Test failures clearly identify which provider lane regressed.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test parity_oracle`
  - `cargo test -p nils-codex-cli --test parity_oracle`

## Sprint 2: Collapse codex-core into codex-cli
**Goal**: Remove runtime dependency on `codex-core` by embedding a codex profile adapter in `codex-cli` backed by shared runtime primitives.
**Demo/Validation**:
- Command(s): `cargo test -p nils-codex-cli`
- Verify: codex CLI behavior remains stable while no `codex_core` imports remain in codex CLI sources.

### Task 2.1: Replace codex_core call sites with codex-cli runtime adapter
- **Location**:
  - `crates/codex-cli/Cargo.toml`
  - `crates/codex-cli/src/lib.rs`
  - `crates/codex-cli/src/runtime/mod.rs`
  - `crates/codex-cli/src/provider_profile.rs`
  - `crates/codex-cli/src/agent/exec.rs`
  - `crates/codex-cli/src/auth/mod.rs`
  - `crates/codex-cli/src/config.rs`
  - `crates/codex-cli/src/jwt.rs`
  - `crates/codex-cli/src/paths.rs`
- **Description**: Rewire codex runtime access through crate-local adapters that call `nils-common::provider_runtime` with codex-specific profile values.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 8
- **Acceptance criteria**:
  - `codex-core` dependency is removed from `crates/codex-cli/Cargo.toml`.
  - All previous `codex_core::` call sites in codex CLI source are replaced by local adapter paths.
  - Existing command UX text and exit codes remain unchanged.
- **Validation**:
  - `cargo test -p nils-codex-cli --test dispatch --test main_entrypoint`
  - `cargo test -p nils-codex-cli --test auth_json_contract --test diag_json_contract`
  - `cargo test -p nils-codex-cli --test rate_limits_all --test starship_refresh`

### Task 2.2: Port codex-core contract tests into codex-cli
- **Location**:
  - `crates/codex-cli/tests/runtime_auth_contract.rs`
  - `crates/codex-cli/tests/runtime_error_contract.rs`
  - `crates/codex-cli/tests/runtime_exec_contract.rs`
  - `crates/codex-cli/tests/runtime_paths_config_contract.rs`
- **Description**: Preserve former `codex-core` contract coverage by moving equivalent tests into codex-cli integration tests against the new adapter/runtime boundary.
- **Dependencies**:
  - `Task 2.1`
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Prior core contract assertions are represented in codex CLI tests with equivalent pass/fail semantics.
  - Runtime tests pass without importing `codex_core`.
  - Test names and structure align with gemini lane equivalents for parity maintainability.
- **Validation**:
  - `cargo test -p nils-codex-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
  - `cargo test -p nils-codex-cli --test parity_oracle`

### Task 2.3: Remove codex-core references from codex lane metadata and docs
- **Location**:
  - `crates/codex-cli/README.md`
  - `README.md`
  - `Cargo.lock`
- **Description**: Update codex documentation and lockfile state so codex lane no longer refers to `codex-core` as an active runtime dependency.
- **Dependencies**:
  - `Task 2.2`
- **Complexity**: 4
- **Acceptance criteria**:
  - Codex CLI docs describe runtime ownership as internal to `codex-cli` plus shared `nils-common` helpers.
  - `cargo tree -p nils-codex-cli` no longer contains `nils-codex-core` in its dependency graph.
  - Repository search confirms codex lane source has no active `codex_core::` usage.
- **Validation**:
  - `if cargo tree -p nils-codex-cli | rg -q "nils-codex-core"; then echo "unexpected codex core dependency edge"; exit 1; fi`
  - `if rg -n "codex_core::|codex-core" crates/codex-cli/src crates/codex-cli/README.md README.md; then echo "unexpected codex-core reference in active codex lane files"; exit 1; fi`
  - `cargo check -p nils-codex-cli`

## Sprint 3: Collapse gemini-core into gemini-cli
**Goal**: Remove runtime dependency on `gemini-core` by embedding a gemini profile adapter in `gemini-cli` backed by shared runtime primitives.
**Demo/Validation**:
- Command(s): `cargo test -p nils-gemini-cli`
- Verify: gemini CLI behavior remains stable while no `gemini_core` imports remain in gemini CLI sources.

### Task 3.1: Replace gemini_core call sites with gemini-cli runtime adapter
- **Location**:
  - `crates/gemini-cli/Cargo.toml`
  - `crates/gemini-cli/src/lib.rs`
  - `crates/gemini-cli/src/runtime/mod.rs`
  - `crates/gemini-cli/src/provider_profile.rs`
  - `crates/gemini-cli/src/agent/exec.rs`
  - `crates/gemini-cli/src/auth/mod.rs`
  - `crates/gemini-cli/src/auth/refresh.rs`
  - `crates/gemini-cli/src/auth/login.rs`
  - `crates/gemini-cli/src/auth/save.rs`
  - `crates/gemini-cli/src/auth/remove.rs`
  - `crates/gemini-cli/src/auth/sync.rs`
  - `crates/gemini-cli/src/auth/auto_refresh.rs`
  - `crates/gemini-cli/src/auth/current.rs`
  - `crates/gemini-cli/src/auth/use_secret.rs`
  - `crates/gemini-cli/src/config.rs`
  - `crates/gemini-cli/src/json.rs`
  - `crates/gemini-cli/src/jwt.rs`
  - `crates/gemini-cli/src/paths.rs`
- **Description**: Rewire gemini runtime access through crate-local adapters that call `nils-common::provider_runtime` with gemini-specific profile values.
- **Dependencies**:
  - `Task 1.2`
- **Complexity**: 9
- **Acceptance criteria**:
  - `gemini-core` dependency is removed from `crates/gemini-cli/Cargo.toml`.
  - All previous `gemini_core::` call sites in gemini CLI source are replaced by local adapter paths.
  - Existing command UX text and exit codes remain unchanged.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test dispatch --test main_entrypoint`
  - `cargo test -p nils-gemini-cli --test auth_json_contract --test auth_json_contract_more`
  - `cargo test -p nils-gemini-cli --test diag_json_contract --test parity_oracle`

### Task 3.2: Port gemini-core contract tests into gemini-cli
- **Location**:
  - `crates/gemini-cli/tests/runtime_auth_contract.rs`
  - `crates/gemini-cli/tests/runtime_error_contract.rs`
  - `crates/gemini-cli/tests/runtime_exec_contract.rs`
  - `crates/gemini-cli/tests/runtime_paths_config_contract.rs`
- **Description**: Preserve former `gemini-core` contract coverage by moving equivalent tests into gemini-cli integration tests against the new adapter/runtime boundary.
- **Dependencies**:
  - `Task 3.1`
  - `Task 1.3`
- **Complexity**: 6
- **Acceptance criteria**:
  - Prior core contract assertions are represented in gemini CLI tests with equivalent pass/fail semantics.
  - Runtime tests pass without importing `gemini_core`.
  - Test names and structure align with codex lane equivalents for parity maintainability.
- **Validation**:
  - `cargo test -p nils-gemini-cli --test runtime_auth_contract --test runtime_error_contract --test runtime_exec_contract --test runtime_paths_config_contract`
  - `cargo test -p nils-gemini-cli --test parity_oracle`

### Task 3.3: Remove gemini-core references from gemini lane metadata and release order
- **Location**:
  - `crates/gemini-cli/README.md`
  - `README.md`
  - `release/crates-io-publish-order.txt`
  - `Cargo.lock`
- **Description**: Update gemini documentation, publish-order metadata, and lockfile state so gemini lane no longer refers to `gemini-core` as an active runtime dependency.
- **Dependencies**:
  - `Task 3.2`
- **Complexity**: 4
- **Acceptance criteria**:
  - Gemini CLI docs describe runtime ownership as internal to `gemini-cli` plus shared `nils-common` helpers.
  - `release/crates-io-publish-order.txt` no longer includes `nils-gemini-core`.
  - `cargo tree -p nils-gemini-cli` no longer contains `nils-gemini-core` in its dependency graph.
- **Validation**:
  - `if cargo tree -p nils-gemini-cli | rg -q "nils-gemini-core"; then echo "unexpected gemini core dependency edge"; exit 1; fi`
  - `if rg -n "gemini_core::|gemini-core" crates/gemini-cli/src crates/gemini-cli/README.md README.md; then echo "unexpected gemini-core reference in active gemini lane files"; exit 1; fi`
  - `if rg -n "nils-gemini-core" release/crates-io-publish-order.txt; then echo "unexpected publish-order entry for nils-gemini-core"; exit 1; fi`
  - `cargo check -p nils-gemini-cli`

## Sprint 4: Workspace Removal, Consistency Sweep, and Delivery Gate
**Goal**: Remove both core crates from workspace membership, complete migration-doc cleanup/deduplication, keep one canonical latest parity doc, and close with full required checks + coverage gate.
**Demo/Validation**:
- Command(s): `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
- Verify: full required checks pass and workspace coverage remains at or above 85 percent.

### Task 4.1: Remove core crates from workspace and filesystem
- **Location**:
  - `Cargo.toml`
  - `crates/codex-core/Cargo.toml`
  - `crates/codex-core/src/lib.rs`
  - `crates/codex-core/docs/README.md`
  - `crates/gemini-core/Cargo.toml`
  - `crates/gemini-core/src/lib.rs`
  - `crates/gemini-core/docs/README.md`
- **Description**: Remove `crates/codex-core` and `crates/gemini-core` from workspace members and delete their crate contents after both CLI lanes no longer depend on them.
- **Dependencies**:
  - `Task 2.2`
  - `Task 3.2`
- **Complexity**: 7
- **Acceptance criteria**:
  - Workspace `members` list no longer includes `crates/codex-core` or `crates/gemini-core`.
  - Build graph resolves without either core crate present.
  - No source imports reference `codex_core` or `gemini_core`.
- **Validation**:
  - `if cargo metadata --no-deps | rg -q "\"name\":\"nils-codex-core\"|\"name\":\"nils-gemini-core\""; then echo "core crates still present in workspace metadata"; exit 1; fi`
  - `if rg -n "codex_core::|gemini_core::" crates; then echo "runtime core imports still present"; exit 1; fi`
  - `cargo check --workspace`

### Task 4.2: Build migration doc inventory and stale/duplicate cleanup map
- **Location**:
  - `docs/reports/codex-gemini-doc-audit.md`
  - `docs/runbooks/codex-core-migration.md`
  - `README.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/gemini-cli/docs/README.md`
  - `crates/codex-cli/docs/runbooks/json-consumers.md`
  - `crates/gemini-cli/docs/runbooks/json-consumers.md`
  - `crates/codex-cli/docs/specs/codex-cli-diag-auth-json-contract-v1.md`
  - `crates/gemini-cli/docs/specs/gemini-cli-diag-auth-json-contract-v1.md`
- **Description**: Audit codex/gemini migration-related docs and produce a keep/merge/remove map so outdated or duplicate docs are removed in a controlled way.
- **Dependencies**:
  - `Task 4.1`
  - `Task 2.3`
  - `Task 3.3`
- **Complexity**: 5
- **Acceptance criteria**:
  - Audit report exists and marks every migration-related doc with `keep`, `merge`, or `remove`, including rationale.
  - Any broken moved-path redirect (for example stale `Moved to:` paths under removed crates) is flagged and scheduled for fix/remove.
  - Duplicate-purpose docs are explicitly mapped to one canonical survivor path.
- **Validation**:
  - `test -f docs/reports/codex-gemini-doc-audit.md`
  - `rg -n "^\\| .* \\| (keep|merge|remove) \\|" docs/reports/codex-gemini-doc-audit.md`
  - `if test -f docs/runbooks/codex-core-migration.md && rg -n "Moved to: \`crates/codex-core/docs/runbooks/codex-core-migration.md\`" docs/runbooks/codex-core-migration.md; then echo "broken codex-core migration redirect still present"; exit 1; fi`

### Task 4.3: Consolidate parity docs to one canonical latest document
- **Location**:
  - `docs/specs/codex-gemini-cli-parity-contract-v1.md`
  - `docs/reports/codex-gemini-doc-audit.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/gemini-cli/docs/README.md`
- **Description**: Keep one latest parity markdown document as the canonical source, remove/redirect older parity docs, and wire crate docs indexes to that canonical path.
- **Dependencies**:
  - `Task 4.2`
- **Complexity**: 6
- **Acceptance criteria**:
  - `docs/specs/codex-gemini-cli-parity-contract-v1.md` exists and is the single canonical parity markdown doc.
  - Any previous codex/gemini migration parity markdown docs are removed or replaced by explicit redirects to the canonical parity doc.
  - Both crate docs indexes include a link to the canonical parity doc.
- **Validation**:
  - `test -f docs/specs/codex-gemini-cli-parity-contract-v1.md`
  - `if find docs crates/codex-cli/docs crates/gemini-cli/docs -type f -name '*parity*.md' | rg -v '^docs/specs/codex-gemini-cli-parity-contract-v1.md$' | rg .; then echo "unexpected extra parity markdown docs"; exit 1; fi`
  - `rg -n "codex-gemini-cli-parity-contract-v1.md" crates/codex-cli/docs/README.md crates/gemini-cli/docs/README.md`

### Task 4.4: Correct root and crate README files after migration cleanup
- **Location**:
  - `README.md`
  - `crates/codex-cli/README.md`
  - `crates/gemini-cli/README.md`
  - `crates/codex-cli/docs/README.md`
  - `crates/gemini-cli/docs/README.md`
- **Description**: Normalize root/crate README ownership and links after core-crate removal and doc deduplication so repository guidance is accurate and non-redundant.
- **Dependencies**:
  - `Task 4.3`
- **Complexity**: 4
- **Acceptance criteria**:
  - Root README workspace layout no longer lists `crates/codex-core` or `crates/gemini-core`.
  - Codex and gemini crate READMEs describe runtime ownership as crate-local plus shared `nils-common` helpers.
  - README links do not point to removed docs/crates and include the canonical parity doc link.
- **Validation**:
  - `if rg -n "crates/codex-core|crates/gemini-core|nils-codex-core|nils-gemini-core" README.md crates/codex-cli/README.md crates/gemini-cli/README.md; then echo "unexpected removed-core references in README files"; exit 1; fi`
  - `rg -n "nils-common" README.md crates/codex-cli/README.md crates/gemini-cli/README.md`
  - `rg -n "codex-gemini-cli-parity-contract-v1.md" README.md crates/codex-cli/README.md crates/gemini-cli/README.md`

### Task 4.5: Execute mandatory checks and coverage gate before delivery
- **Location**:
  - `.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `scripts/ci/coverage-summary.sh`
- **Description**: Run the repository-required verification sequence and coverage gate after all merges/cleanup are complete.
- **Dependencies**:
  - `Task 4.4`
- **Complexity**: 6
- **Acceptance criteria**:
  - Required checks script completes successfully.
  - Coverage command completes successfully with fail-under-lines threshold satisfied.
  - Any regressions are fixed before final delivery.
- **Validation**:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `./.agents/skills/nils-cli-verify-required-checks/scripts/nils-cli-verify-required-checks.sh`
  - `mkdir -p target/coverage`
  - `cargo llvm-cov nextest --profile ci --workspace --lcov --output-path target/coverage/lcov.info --fail-under-lines 85`
  - `scripts/ci/coverage-summary.sh target/coverage/lcov.info`

## Dependency and Parallel Execution Map
- Initial linear setup: `Task 1.1` -> `Task 1.2`
- Parallel batch A after `Task 1.1`: `Task 1.3`
- Parallel batch B after `Task 1.2`: `Task 2.1` and `Task 3.1`
- Parallel batch C: `Task 2.2` depends on `Task 2.1`; `Task 3.2` depends on `Task 3.1`; both can run concurrently.
- Parallel batch D: `Task 2.3` and `Task 3.3` can run concurrently after their lane test ports are complete.
- Parallel batch E: `Task 4.1` can start after `Task 2.2` and `Task 3.2`; `Task 2.3` and `Task 3.3` can continue in parallel.
- Linear closeout for docs: `Task 4.2` -> `Task 4.3` -> `Task 4.4` -> `Task 4.5`

## Testing Strategy
- Unit:
  - `cargo test -p nils-common`
  - `cargo test -p nils-codex-cli --lib`
  - `cargo test -p nils-gemini-cli --lib`
- Integration:
  - `cargo test -p nils-codex-cli`
  - `cargo test -p nils-gemini-cli`
  - `cargo test -p nils-gemini-cli --test parity_oracle`
  - `cargo test -p nils-codex-cli --test parity_oracle`
- E2E/manual:
  - `cargo run -p nils-codex-cli -- --help`
  - `cargo run -p nils-gemini-cli -- --help`
  - `cargo run -p nils-codex-cli -- diag rate-limits --help`
  - `cargo run -p nils-gemini-cli -- diag rate-limits --help`
- Documentation hygiene:
  - `bash scripts/ci/docs-placement-audit.sh --strict`
  - `if find docs crates/codex-cli/docs crates/gemini-cli/docs -type f -name '*parity*.md' | rg -v '^docs/specs/codex-gemini-cli-parity-contract-v1.md$' | rg .; then echo "unexpected extra parity markdown docs"; exit 1; fi`
  - `if find docs crates/codex-cli/docs crates/gemini-cli/docs -type f -name '*.md' -print0 | xargs -0 shasum | awk '{print $1}' | sort | uniq -d | rg .; then echo "unexpected duplicate markdown payloads"; exit 1; fi`

## Risks & gotchas
- Provider-specific defaults and path precedence can drift silently if profile constants are incomplete.
- Auth and JWT parsing appears identical today; accidental local forks can reappear without parity-oracle enforcement in both lanes.
- Removing crates from workspace can break release or docs scripts if stale references remain (`README`, publish-order list, migration stub).
- Aggressive deduplication can remove intentionally different consumer docs; the audit map must capture rationale before deletion.
- Concurrent edits in both CLI crates create merge-conflict risk; keep runtime adapter file structure intentionally mirrored to reduce conflict surface.

## Rollback plan
- Keep migration as a sequence of small commits aligned to sprint boundaries so each lane can be reverted independently.
- If codex lane regresses, restore `crates/codex-cli/Cargo.toml` dependency on `nils-codex-core`, restore `codex_core::` call paths, and re-run codex test suite before touching gemini lane.
- If gemini lane regresses, restore `crates/gemini-cli/Cargo.toml` dependency on `nils-gemini-core`, restore `gemini_core::` call paths, and re-run gemini test suite before touching codex lane.
- If shared runtime abstraction is the root cause, revert only `crates/nils-common/src/provider_runtime/*` changes and temporarily keep duplicated crate-local adapters to unblock delivery.
- If workspace cleanup causes release/doc breakage, re-add `crates/codex-core` and `crates/gemini-core` entries to `Cargo.toml` members, reinsert removed publish-order/doc references, and ship a stabilization PR before retrying removal.

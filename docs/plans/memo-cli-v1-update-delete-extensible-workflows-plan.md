# Plan: memo-cli v1 rebaseline with update/delete and extensible workflow cleanup

## Overview
This plan rebases `memo-cli` contracts before first production rollout: archive the current v1 contracts as v0, then redefine active v1 contracts to include `update` and hard `delete`. The implementation keeps command/version labels at v1 (because the product is not yet launched) while still using internal DB migration versions for safe upgrades from existing local dev databases. The core architecture goal is consistency: when raw memo content is updated or hard-deleted, derivations, search projections, and future workflow-specific data must remain in sync and be cleaned deterministically. For extension modeling, this plan standardizes on an `anchor table` ownership model rather than direct workflow-to-raw foreign keys.

## Scope
- In scope:
  - Archive current v1 specs as v0 historical snapshot and replace active v1 specs.
  - Add `update` and `delete --hard` command contracts, JSON contracts, and CLI surface.
  - Add transactional cleanup semantics so update/delete do not leave stale derivation or search state.
  - Add trigger/fk strategy for future workflow-specific data (for example `game`, `sport`, `health`) with mandatory cleanup on raw delete.
  - Extend tests, completions, and runbooks for new behavior.
- Out of scope:
  - Implementing full domain workflows (`game`, `sport`, `health`) end-to-end in this change.
  - Soft delete, recycle bin, or time-travel restore UX.
  - Cloud sync / multi-device conflict handling.

## Assumptions (if any)
1. `memo-cli` has not been released to production users, so contract relabeling (v1 -> archived v0, new active v1) is acceptable.
2. Local/dev DB files using the current schema may already exist and need an upgrade path.
3. Hard delete is intentionally destructive and no tombstone retention is required.
4. Future workflow tables will follow repo-defined anchor-table contracts (fk + cascade) rather than ad-hoc schemas.

## Success Criteria
1. Active docs under `crates/memo-cli/docs/specs/` define the new v1 behavior with `update` and hard `delete`.
2. Archived docs preserve prior semantics as v0 and remain discoverable.
3. `update` and `delete --hard` do not leave stale rows in derivation/search layers.
4. A documented extension contract guarantees future workflow data is cleaned when a raw item is deleted.
5. Required checks in `DEVELOPMENT.md` pass with new tests covering update/delete consistency and cleanup.

## Sprint 1: Contract rebaseline and architecture freeze
**Goal**: Freeze naming/version policy and v1 behavior before code changes.
**Demo/Validation**:
- Command(s):
  - `plan-tooling validate --file docs/plans/memo-cli-v1-update-delete-extensible-workflows-plan.md`
  - `rg -n "update|delete|hard delete|v0|archive" crates/memo-cli/docs/specs -S`
- Verify:
  - Active v1 docs and archived v0 docs are both present and cross-referenced.
  - New v1 docs explicitly define update/delete lifecycle and cleanup guarantees.

### Task 1.1: Archive existing v1 specs as v0 snapshot
- **Location**:
  - `crates/memo-cli/docs/specs/archive/v0/memo-cli-command-contract-v0.md`
  - `crates/memo-cli/docs/specs/archive/v0/memo-cli-json-contract-v0.md`
  - `crates/memo-cli/docs/specs/archive/v0/memo-cli-storage-schema-v0.md`
  - `crates/memo-cli/README.md`
- **Description**: Copy current v1 contract/spec files to an archive v0 path, add short provenance notes (date + reason), and update README/spec index links so active docs remain v1 while old behavior is traceable as v0.
- **Dependencies**:
  - none
- **Complexity**: 3
- **Acceptance criteria**:
  - Archived v0 files exactly preserve prior behavior statements (append-only, immutable raw).
  - Active references in README/spec index point to the new v1 files, with explicit note that old v1 moved to v0 archive pre-launch.
- **Validation**:
  - `rg -n "append-only|immutable raw" crates/memo-cli/docs/specs/archive/v0 -S`
  - `rg -n "archive/v0|v1" crates/memo-cli/README.md crates/memo-cli/docs/specs -S`

### Task 1.2: Redefine active v1 command and JSON contracts for update/delete
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md`
  - `crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md`
  - `crates/memo-cli/README.md`
  - `completions/zsh/_memo-cli`
  - `completions/bash/memo-cli`
- **Description**: Replace append-only wording in active v1 docs with mutable lifecycle semantics: `update` mutates raw text and invalidates/removes downstream derivation outputs; `delete --hard` permanently removes raw and downstream data. Define command usage, output envelopes, exit codes, and cursor/state expectations after updates/deletes.
- **Dependencies**:
  - Task 1.1
- **Complexity**: 6
- **Acceptance criteria**:
  - Command surface includes `update` and `delete`.
  - JSON schema table includes stable identifiers for new commands (`memo-cli.update.v1`, `memo-cli.delete.v1`).
  - Contract explicitly states post-update state transition (item returns to pending) and hard-delete semantics (item no longer queryable).
  - Completion docs include new subcommands/options.
- **Validation**:
  - `rg -n "memo-cli update|memo-cli delete|memo-cli.update.v1|memo-cli.delete.v1" crates/memo-cli/docs/specs/memo-cli-command-contract-v1.md crates/memo-cli/docs/specs/memo-cli-json-contract-v1.md -S`
  - `rg -n "update|delete" completions/zsh/_memo-cli completions/bash/memo-cli -S`

### Task 1.3: Define extensible workflow cleanup contract (anchor-table + trigger boundaries)
- **Location**:
  - `crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md`
  - `crates/memo-cli/docs/specs/memo-cli-workflow-extension-contract-v1.md`
- **Description**: Introduce a normative extension contract for future typed workflow data (`game`, `sport`, `health`, others). Define canonical ownership chain through an extension `anchor table` rooted at `inbox_items.item_id`, with `on delete cascade` across the ownership chain. Triggers are reserved for denormalized projection refresh and invariant checks, not primary cleanup of unknown future tables.
- **Dependencies**:
  - Task 1.2
- **Complexity**: 7
- **Acceptance criteria**:
  - Spec defines the required schema pattern for extension tables:
    - extension records must reference an anchor row;
    - anchor rows must reference `inbox_items(item_id) on delete cascade`.
  - Spec forbids `on delete restrict` on the anchor ownership cleanup path for extension-owned rows.
  - Spec defines naming/ownership conventions for workflow-type data (including tags and typed fields such as `game_name`, `source_url`, `description`).
- **Validation**:
  - `rg -n "on delete cascade|extension|workflow_type|game_name|source_url|description" crates/memo-cli/docs/specs/memo-cli-storage-schema-v1.md crates/memo-cli/docs/specs/memo-cli-workflow-extension-contract-v1.md -S`

## Sprint 2: Storage migration and consistency mechanics
**Goal**: Implement schema/trigger/command behavior that enforces the new lifecycle without stale data.
**Demo/Validation**:
- Command(s):
  - `cargo test -p nils-memo-cli --test add_and_list --test fetch_apply_flow --test search_and_report`
  - `cargo test -p nils-memo-cli update_delete_flow`
- Verify:
  - Existing behavior remains stable where unchanged.
  - Update/delete paths maintain consistency across list/search/fetch/report.

### Task 2.1: Upgrade migration framework for non-breaking contract relabel + schema evolution
- **Location**:
  - `crates/memo-cli/src/storage/migrate.rs`
  - `crates/memo-cli/src/storage/sql/schema_v1.sql`
  - `crates/memo-cli/src/storage/sql/migrations/0002_mutable_raw_and_hard_delete.sql`
  - `crates/memo-cli/src/storage/sql/migrations/0003_extension_anchor.sql`
  - `crates/memo-cli/src/storage/mod.rs`
- **Description**: Move from single-shot schema apply to incremental migrations so dev DBs can upgrade safely. Keep external contract naming at v1 while allowing internal schema migration version increments (for example `schema_migrations` v2/v3) to implement mutable raw + hard-delete support.
- **Dependencies**:
  - Task 1.3
- **Complexity**: 8
- **Acceptance criteria**:
  - New DB bootstrap yields the latest schema in one run.
  - Existing DB at prior version upgrades in-place without manual intervention.
  - Migration tests cover idempotency and upgrade path.
- **Validation**:
  - `cargo test -p nils-memo-cli storage::tests::migration_idempotent`
  - `cargo test -p nils-memo-cli storage::tests::init_db`

### Task 2.2: Implement update/delete commands with transactional downstream cleanup
- **Location**:
  - `crates/memo-cli/src/cli.rs`
  - `crates/memo-cli/src/commands/mod.rs`
  - `crates/memo-cli/src/commands/update.rs`
  - `crates/memo-cli/src/commands/delete.rs`
  - `crates/memo-cli/src/storage/repository.rs`
  - `crates/memo-cli/src/output/text.rs`
  - `crates/memo-cli/src/output/json.rs`
- **Description**: Add CLI and repository operations for `update` and hard `delete`, enforcing single-transaction semantics. `update` must refresh raw text and clear/invalidate downstream derivation state so item re-enters pending. `delete --hard` must remove raw row plus all dependent rows (derivations, tags mappings, search projection, extension-owned data).
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - `memo-cli update itm_00000001 "revised memo text"` succeeds and updated item appears as pending in `list`/`fetch`.
  - `memo-cli delete itm_00000001 --hard` removes item from `list`, `search`, `fetch`, and report totals.
  - JSON and text outputs follow contract and error code rules.
- **Validation**:
  - `cargo run -p nils-memo-cli -- update --help`
  - `cargo run -p nils-memo-cli -- delete --help`
  - `cargo test -p nils-memo-cli update_delete_flow`
  - `cargo test -p nils-memo-cli json_contract`

### Task 2.3: Add trigger set for projection consistency and delete hygiene
- **Location**:
  - `crates/memo-cli/src/storage/sql/schema_v1.sql`
  - `crates/memo-cli/src/storage/sql/migrations/0002_mutable_raw_and_hard_delete.sql`
- **Description**: Add/adjust triggers so search projections remain correct after update/delete paths. At minimum, add refresh behavior for raw text updates and derivation deletes; ensure no stale `derived_text`/`tags_text` survives after downstream cleanup.
- **Dependencies**:
  - Task 2.1
- **Complexity**: 7
- **Acceptance criteria**:
  - Raw text update updates `item_search_documents.raw_text` and FTS content deterministically.
  - Derivation deletes (direct or cascade) refresh projection to empty/fallback state for that item.
  - Hard delete removes associated search projection rows without orphan records.
- **Validation**:
  - `cargo test -p nils-memo-cli metadata_projection`
  - `cargo test -p nils-memo-cli search_and_report`

### Task 2.4: Introduce extension anchor schema and FK policy checks
- **Location**:
  - `crates/memo-cli/src/storage/sql/schema_v1.sql`
  - `crates/memo-cli/src/storage/sql/migrations/0003_extension_anchor.sql`
  - `crates/memo-cli/src/storage/repository.rs`
  - `crates/memo-cli/docs/specs/memo-cli-workflow-extension-contract-v1.md`
- **Description**: Add a minimal extension anchor model as the only approved pattern for future typed workflow records tied to `item_id` with guaranteed cascade cleanup. Provide helper/query patterns and guardrails so future tables can add typed columns (`game_name`, `source_url`, `description`, etc.) without breaking delete consistency.
- **Dependencies**:
  - Task 1.3
  - Task 2.1
- **Complexity**: 8
- **Acceptance criteria**:
  - Schema includes a documented extension ownership path rooted at `inbox_items.item_id`.
  - At least one test fixture demonstrates extension row cleanup on hard delete.
  - Contract document includes onboarding checklist for new workflow table authors.
- **Validation**:
  - `cargo test -p nils-memo-cli extension_cleanup_contract`
  - `rg -n "workflow|extension|cascade|on delete" crates/memo-cli/src/storage/sql/schema_v1.sql crates/memo-cli/docs/specs/memo-cli-workflow-extension-contract-v1.md -S`

## Sprint 3: Test hardening, docs rollout, and delivery gates
**Goal**: Prove behavior parity/coherence and publishable developer guidance.
**Demo/Validation**:
- Command(s):
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`
  - `cargo test -p nils-memo-cli`
  - `zsh -f tests/zsh/completion.test.zsh`
- Verify:
  - Required checks are green.
  - New commands and cleanup guarantees are covered by tests and docs.

### Task 3.1: Add end-to-end tests for update/delete lifecycle consistency
- **Location**:
  - `crates/memo-cli/tests/update_delete_flow.rs`
  - `crates/memo-cli/tests/fetch_apply_flow.rs`
  - `crates/memo-cli/tests/metadata_projection.rs`
  - `crates/memo-cli/tests/json_contract.rs`
- **Description**: Add deterministic tests for update/delete covering list/search/fetch/report/json envelopes and edge cases (non-existent item, invalid id, missing required `--hard` safety flag, re-apply after update).
- **Dependencies**:
  - Task 2.2
  - Task 2.3
- **Complexity**: 6
- **Acceptance criteria**:
  - Update test proves prior derivation/search metadata is removed or invalidated.
  - Delete test proves all command surfaces no longer expose deleted item.
  - JSON contract tests include new schemas and stable error code behavior.
- **Validation**:
  - `cargo test -p nils-memo-cli update_delete_flow`
  - `cargo test -p nils-memo-cli json_contract`

### Task 3.2: Add extension cleanup contract tests with typed workflow fixture
- **Location**:
  - `crates/memo-cli/tests/extension_cleanup_contract.rs`
  - `crates/memo-cli/tests/support/mod.rs`
- **Description**: Add integration tests that simulate a typed workflow dataset (for example `game_name`, `source_url`, `description`) linked through approved FK path and verify hard delete cleans all dependent rows.
- **Dependencies**:
  - Task 2.4
- **Complexity**: 7
- **Acceptance criteria**:
  - Test inserts synthetic workflow rows for one item and confirms cleanup on hard delete.
  - Test fails if FK policy is bypassed (documented negative/guard case).
- **Validation**:
  - `cargo test -p nils-memo-cli extension_cleanup_contract`

### Task 3.3: Update operator docs, runbooks, and completion guidance
- **Location**:
  - `crates/memo-cli/README.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md`
  - `crates/memo-cli/docs/runbooks/memo-cli-rollout.md`
  - `completions/zsh/_memo-cli`
  - `completions/bash/memo-cli`
- **Description**: Update workflows and rollout docs for update/delete behavior, including caution text for hard delete, agent-loop expectations after update, and extension-authoring FK rules.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
- **Complexity**: 4
- **Acceptance criteria**:
  - Runbooks include update/delete examples and rollback notes.
  - Completion files expose new command options.
  - README clearly differentiates active v1 vs archived v0 documentation.
- **Validation**:
  - `rg -n "update|delete|hard|archive/v0|workflow" crates/memo-cli/README.md crates/memo-cli/docs/runbooks/memo-cli-agent-workflow.md crates/memo-cli/docs/runbooks/memo-cli-rollout.md completions/zsh/_memo-cli completions/bash/memo-cli -S`

### Task 3.4: Run mandatory gates and finalize delivery package
- **Location**:
  - `DEVELOPMENT.md`
  - `crates/memo-cli/Cargo.toml`
  - `crates/memo-cli/src/main.rs`
- **Description**: Execute required repository gates and ensure failures are triaged before delivery. Produce concise evidence notes for changed behavior and migration assumptions.
- **Dependencies**:
  - Task 3.1
  - Task 3.2
  - Task 3.3
- **Complexity**: 3
- **Acceptance criteria**:
  - `fmt`, `clippy`, `cargo test --workspace`, and zsh completion tests pass.
  - Any skipped tests are explicitly documented with blocker/reason.
- **Validation**:
  - `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`

## Parallelization Plan
- Parallel group A (after Task 1.3): Task 2.1 and design prototyping for Task 2.4 can start in parallel, but Task 2.4 merge waits for migration scaffolding from Task 2.1.
- Parallel group B (after Task 2.4): Task 3.1 and Task 3.2 can run in parallel because they target different test files and assertions.
- Parallel group C (after Task 3.1 + Task 3.2): Task 3.3 docs/completions updates can proceed while Task 3.4 gate runs are prepared.

## Testing Strategy
- Unit/integration:
  - Add repository/command tests for update/delete transaction behavior and error cases.
  - Add projection consistency tests around trigger behavior after derivation/raw mutations.
- Schema/migration:
  - Validate clean bootstrap and upgrade from pre-change schema state.
  - Verify no orphan rows remain after hard delete.
- Workflow extension contract:
  - Add fixture-backed tests for typed extension data cleanup via FK cascade chain.
- Required repo gates:
  - Run `./.agents/skills/nils-cli-checks/scripts/nils-cli-checks.sh`.

## Risks & gotchas
- SQLite table/FK alterations may require table rebuild migrations; careless migration order can break existing local DB files.
- Contract relabeling (old v1 -> v0 archive, new active v1) can confuse maintainers if README/spec links are incomplete.
- Hard delete is irreversible; command UX must minimize accidental destructive execution (for example explicit `--hard` gating and clear output).
- Future extension tables can still create inconsistency if authors bypass the FK contract; docs/tests must make the contract enforceable.

## Rollback plan
- Freeze rollout to capture-only + read paths (`add`, `list`, `search`, `report`, `fetch`) if update/delete causes instability.
- Keep DB backup/export before applying schema upgrade in operational environments.
- If regression is found, revert code and restore archived-v0 behavior by:
  - restoring prior command surface (without update/delete),
  - restoring prior schema path via migration rollback script or DB restore,
  - rerunning `cargo test -p nils-memo-cli memo_flow fetch_apply_flow`.
- Re-enable update/delete only after failing test class has a reproducible fix and full required checks are green.
